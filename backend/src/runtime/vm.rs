use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use sqlx::Row;
use tokio::sync::mpsc::Receiver;

use crate::policy::trust::{persist_vm_attestation_outcome, remediation_notes_for_status};
use crate::policy::{publish_policy_event, PolicyDecision, PolicyEvent, RuntimeBackend};
use crate::servers::{add_metric, set_status};

pub mod attestation;
pub mod libvirt;

pub use attestation::{AttestationStatus, AttestationVerifier, TpmAttestationVerifier};

// key: runtime-vm-executor -> attestation,policy-hooks

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmProvisioningResult {
    pub instance_id: String,
    pub isolation_tier: Option<String>,
    pub attestation_evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_hint: Option<Value>,
    pub requested_image: String,
    pub hypervisor: Option<HypervisorSnapshot>,
}

impl VmProvisioningResult {
    pub fn new(
        instance_id: String,
        isolation_tier: Option<String>,
        attestation_evidence: Option<Value>,
        requested_image: String,
        hypervisor: Option<HypervisorSnapshot>,
    ) -> Self {
        Self {
            instance_id,
            isolation_tier,
            attestation_evidence,
            attestation_hint: None,
            requested_image,
            hypervisor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypervisorSnapshot {
    pub endpoint: String,
    #[serde(default)]
    pub credentials: Option<Value>,
    #[serde(default)]
    pub network_template: Option<Value>,
    #[serde(default)]
    pub volume_template: Option<Value>,
    #[serde(default)]
    pub gpu_passthrough_policy: Option<Value>,
}

impl HypervisorSnapshot {
    pub fn new(
        endpoint: String,
        credentials: Option<Value>,
        network_template: Option<Value>,
        volume_template: Option<Value>,
        gpu_passthrough_policy: Option<Value>,
    ) -> Self {
        Self {
            endpoint,
            credentials,
            network_template,
            volume_template,
            gpu_passthrough_policy,
        }
    }
}

#[async_trait]
pub trait VmProvisioner: Send + Sync {
    async fn provision(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        config: Option<&Value>,
    ) -> Result<VmProvisioningResult>;

    async fn start(&self, instance_id: &str) -> Result<()>;

    async fn stop(&self, instance_id: &str) -> Result<()>;

    async fn teardown(&self, instance_id: &str) -> Result<()>;

    async fn fetch_logs(&self, instance_id: &str) -> Result<String>;

    async fn stream_logs(&self, instance_id: &str) -> Result<Option<Receiver<String>>>;
}

pub struct VirtualMachineExecutor {
    provisioner: Arc<dyn VmProvisioner>,
    attestor: Arc<dyn AttestationVerifier>,
    pool: PgPool,
}

impl VirtualMachineExecutor {
    pub fn new(
        pool: PgPool,
        provisioner: Arc<dyn VmProvisioner>,
        attestor: Arc<dyn AttestationVerifier>,
    ) -> Self {
        Self {
            provisioner,
            attestor,
            pool,
        }
    }

    pub fn descriptor() -> crate::policy::RuntimeExecutorDescriptor {
        crate::policy::RuntimeExecutorDescriptor::new(
            RuntimeBackend::VirtualMachine,
            "Secure virtual machines",
            [],
        )
    }

    async fn persist_instance(
        pool: &PgPool,
        server_id: i32,
        provisioning: &VmProvisioningResult,
        decision: &PolicyDecision,
    ) -> Result<i64> {
        let mut notes = decision.notes.clone();
        if let Some(evidence) = &provisioning.attestation_evidence {
            if evidence.get("trusted").is_some() {
                notes.push("attestation:evidence-present".to_string());
            }
        }

        let snapshot = provisioning.hypervisor.as_ref();
        let record_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO runtime_vm_instances (
                server_id,
                instance_id,
                isolation_tier,
                attestation_status,
                attestation_evidence,
                attestation_hint,
                policy_version,
                capability_notes,
                hypervisor_endpoint,
                hypervisor_credentials,
                hypervisor_network_template,
                hypervisor_volume_template,
                gpu_passthrough_policy
            ) VALUES (
                $1, $2, $3, 'pending', $4, $5, $6, $7, $8, $9, $10, $11, $12
            )
            RETURNING id
            "#,
        )
        .bind(server_id)
        .bind(&provisioning.instance_id)
        .bind(provisioning.isolation_tier.as_deref())
        .bind(provisioning.attestation_evidence.clone())
        .bind(provisioning.attestation_hint.clone())
        .bind(&decision.policy_version)
        .bind(&notes)
        .bind(snapshot.map(|s| s.endpoint.clone()))
        .bind(snapshot.and_then(|s| s.credentials.clone()))
        .bind(snapshot.and_then(|s| s.network_template.clone()))
        .bind(snapshot.and_then(|s| s.volume_template.clone()))
        .bind(snapshot.and_then(|s| s.gpu_passthrough_policy.clone()))
        .fetch_one(pool)
        .await?;

        Ok(record_id)
    }

    async fn record_event(
        pool: &PgPool,
        vm_instance_id: i64,
        event_type: &str,
        payload: Value,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO runtime_vm_events (vm_instance_id, event_type, event_payload) VALUES ($1, $2, $3)",
        )
        .bind(vm_instance_id)
        .bind(event_type)
        .bind(payload)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn map_error(pool: &PgPool, vm_id: Option<i64>, error: &anyhow::Error) {
        if let Some(id) = vm_id {
            let _ = sqlx::query("UPDATE runtime_vm_instances SET last_error = $1 WHERE id = $2")
                .bind(error.to_string())
                .bind(id)
                .execute(pool)
                .await;
        }
    }

    fn map_to_bollard(err: anyhow::Error) -> bollard::errors::Error {
        use std::io;
        bollard::errors::Error::IOError {
            err: io::Error::new(io::ErrorKind::Other, err.to_string()),
        }
    }

    async fn active_instance_for(&self, server_id: i32) -> Result<Option<ActiveVmInstance>> {
        let row = sqlx::query(
            r#"
            SELECT id, instance_id
            FROM runtime_vm_instances
            WHERE server_id = $1
              AND terminated_at IS NULL
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|record| ActiveVmInstance {
            instance_id: record.get("instance_id"),
        }))
    }
}

struct ActiveVmInstance {
    instance_id: String,
}

#[async_trait]
impl crate::runtime::RuntimeExecutor for VirtualMachineExecutor {
    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::VirtualMachine
    }

    fn spawn_server_task(
        &self,
        decision: PolicyDecision,
        server_id: i32,
        server_type: String,
        config: Option<Value>,
        _api_key: String,
        use_gpu: bool,
        pool: PgPool,
    ) {
        let provisioner = Arc::clone(&self.provisioner);
        let attestor = Arc::clone(&self.attestor);
        tokio::spawn(async move {
            let mut vm_id = None;
            let launch_result: Result<()> = async {
                if !matches!(decision.backend, RuntimeBackend::VirtualMachine) {
                    tracing::warn!(
                        %server_id,
                        backend = %decision.backend.as_str(),
                        "runtime policy selected different backend for VM executor",
                    );
                }

                set_status(&pool, server_id, "provisioning")
                    .await
                    .context("failed to set provisioning status")?;

                let start_metric = json!({
                    "backend": decision.backend.as_str(),
                    "server_type": server_type,
                    "use_gpu": use_gpu,
                });
                add_metric(&pool, server_id, "vm.provision.start", Some(&start_metric))
                    .await
                    .ok();

                let provisioning = provisioner
                    .provision(server_id, &decision, config.as_ref())
                    .await
                    .context("provisioner failed to allocate VM")?;

                vm_id = Some(
                    VirtualMachineExecutor::persist_instance(
                        &pool,
                        server_id,
                        &provisioning,
                        &decision,
                    )
                    .await
                    .context("failed to persist VM instance")?,
                );

                if let Some(record_id) = vm_id {
                    VirtualMachineExecutor::record_event(
                        &pool,
                        record_id,
                        "provisioned",
                        json!({
                            "instance_id": provisioning.instance_id,
                            "requested_image": provisioning.requested_image,
                        }),
                    )
                    .await
                    .ok();
                }

                let attestation = attestor
                    .verify(server_id, &decision, &provisioning, config.as_ref())
                    .await
                    .context("attestation verification failed")?;

                let attestation_notes_for_event;

                if let Some(record_id) = vm_id {
                    let remediation = remediation_notes_for_status(attestation.status);
                    let mut attestation_notes = attestation.notes.clone();
                    let mut transition_payload = None;

                    match persist_vm_attestation_outcome(
                        &pool,
                        server_id,
                        record_id,
                        &attestation,
                        &remediation,
                    )
                    .await
                    {
                        Ok(transition) => {
                            if transition.posture_changed {
                                attestation_notes.push("attestation:posture:changed".to_string());
                            }
                            if let Some(freshness) = transition.freshness_expires_at {
                                attestation_notes.push(format!(
                                    "attestation:freshness-expires:{}",
                                    freshness.to_rfc3339()
                                ));
                            }
                            if transition.should_invalidate_cache() {
                                attestation_notes.push("attestation:cache:invalidate".to_string());
                            }
                            let mut payload = transition.broadcast_payload();
                            if let Value::Object(ref mut map) = payload {
                                map.insert(
                                    "trust_event_id".to_string(),
                                    transition.trust_event.id.into(),
                                );
                                map.insert(
                                    "trust_transition_reason".to_string(),
                                    transition
                                        .trust_event
                                        .transition_reason
                                        .clone()
                                        .map(Value::from)
                                        .unwrap_or(Value::Null),
                                );
                            }
                            transition_payload = Some(payload);
                        }
                        Err(err) => {
                            tracing::warn!(
                                ?err,
                                %server_id,
                                "failed to persist attestation trust transition"
                            );
                        }
                    }

                    VirtualMachineExecutor::record_event(
                        &pool,
                        record_id,
                        "attestation",
                        json!({
                            "status": attestation.status.as_str(),
                            "notes": attestation_notes.clone(),
                            "kind": attestation.attestation_kind.as_str(),
                            "freshness_expires_at": attestation.freshness_deadline,
                            "transition": transition_payload,
                        }),
                    )
                    .await
                    .ok();

                    attestation_notes_for_event = attestation_notes;
                } else {
                    attestation_notes_for_event = attestation.notes.clone();
                }

                match sqlx::query("SELECT owner_id FROM mcp_servers WHERE id = $1")
                    .bind(server_id)
                    .fetch_optional(&pool)
                    .await
                {
                    Ok(Some(row)) => {
                        let owner_id: i32 = row.get("owner_id");
                        publish_policy_event(PolicyEvent::attestation(
                            owner_id,
                            server_id,
                            &decision.backend,
                            Some(provisioning.instance_id.clone()),
                            attestation.status.as_str().to_string(),
                            attestation_notes_for_event.clone(),
                            None,
                        ));
                    }
                    Ok(None) => tracing::warn!(
                        %server_id,
                        "attestation completed for missing server owner",
                    ),
                    Err(err) => tracing::warn!(
                        ?err,
                        %server_id,
                        "failed to publish attestation event due to owner lookup error",
                    ),
                }

                match attestation.status {
                    AttestationStatus::Trusted => {
                        provisioner
                            .start(&provisioning.instance_id)
                            .await
                            .context("failed to start VM instance")?;
                        set_status(&pool, server_id, "running")
                            .await
                            .context("failed to set running status")?;
                        let success_metric = json!({ "instance_id": provisioning.instance_id });
                        add_metric(
                            &pool,
                            server_id,
                            "vm.provision.success",
                            Some(&success_metric),
                        )
                        .await
                        .ok();
                        Ok(())
                    }
                    AttestationStatus::Unknown => {
                        tracing::warn!(
                            %server_id,
                            "attestation outcome unknown; leaving instance pending",
                        );
                        set_status(&pool, server_id, "pending-attestation")
                            .await
                            .context("failed to set pending-attestation status")?;
                        Ok(())
                    }
                    AttestationStatus::Untrusted => {
                        provisioner
                            .teardown(&provisioning.instance_id)
                            .await
                            .context("failed to teardown untrusted VM")?;
                        set_status(&pool, server_id, "blocked")
                            .await
                            .context("failed to set blocked status")?;
                        Err(anyhow::anyhow!("attestation rejected for VM instance"))
                    }
                }
            }
            .await;

            if let Err(err) = launch_result {
                tracing::error!(?err, %server_id, "vm executor failed to launch instance");
                VirtualMachineExecutor::map_error(&pool, vm_id, &err).await;
                let failure_metric = json!({
                    "error": err.to_string(),
                    "backend": decision.backend.as_str(),
                });
                let _ = add_metric(
                    &pool,
                    server_id,
                    "vm.provision.failure",
                    Some(&failure_metric),
                )
                .await;
                let _ = set_status(&pool, server_id, "error").await;
            } else if let Some(record_id) = vm_id {
                VirtualMachineExecutor::record_event(
                    &pool,
                    record_id,
                    "ready",
                    json!({ "server_id": server_id }),
                )
                .await
                .ok();
            }
        });
    }

    fn stop_server_task(&self, server_id: i32, pool: PgPool) {
        let provisioner = Arc::clone(&self.provisioner);
        let status_pool = pool.clone();
        tokio::spawn(async move {
            match sqlx::query(
                r#"SELECT id, instance_id FROM runtime_vm_instances WHERE server_id = $1 AND terminated_at IS NULL ORDER BY created_at DESC LIMIT 1"#,
            )
            .bind(server_id)
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(row)) => {
                    let instance_id: String = row.get("instance_id");
                    let db_id: i64 = row.get("id");
                    if let Err(err) = provisioner.stop(&instance_id).await {
                        tracing::error!(?err, %server_id, "failed to stop vm instance");
                    }
                    let _ = sqlx::query(
                        "UPDATE runtime_vm_instances SET terminated_at = NOW() WHERE id = $1",
                    )
                    .bind(db_id)
                    .execute(&pool)
                    .await;
                    let _ = set_status(&status_pool, server_id, "stopped").await;
                }
                Ok(None) => {
                    tracing::warn!(%server_id, "no active vm instance to stop");
                }
                Err(err) => {
                    tracing::error!(?err, %server_id, "failed to locate vm instance to stop");
                }
            }
        });
    }

    fn delete_server_task(&self, server_id: i32, pool: PgPool) {
        let provisioner = Arc::clone(&self.provisioner);
        tokio::spawn(async move {
            match sqlx::query(
                r#"SELECT id, instance_id FROM runtime_vm_instances WHERE server_id = $1 ORDER BY created_at DESC LIMIT 1"#,
            )
            .bind(server_id)
            .fetch_optional(&pool)
            .await
            {
                Ok(Some(row)) => {
                    let instance_id: String = row.get("instance_id");
                    let db_id: i64 = row.get("id");
                    if let Err(err) = provisioner.teardown(&instance_id).await {
                        tracing::error!(?err, %server_id, "failed to teardown vm instance");
                    }
                    let _ = sqlx::query(
                        "UPDATE runtime_vm_instances SET terminated_at = NOW() WHERE id = $1",
                    )
                    .bind(db_id)
                    .execute(&pool)
                    .await;
                }
                Ok(None) => tracing::warn!(%server_id, "no vm instance to delete"),
                Err(err) => tracing::error!(?err, %server_id, "failed to fetch vm instance for delete"),
            }

            let _ = sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
                .bind(server_id)
                .execute(&pool)
                .await;
        });
    }

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error> {
        let active = self
            .active_instance_for(server_id)
            .await
            .map_err(Self::map_to_bollard)?;
        if let Some(instance) = active {
            self.provisioner
                .fetch_logs(&instance.instance_id)
                .await
                .map_err(Self::map_to_bollard)
        } else {
            Ok(String::new())
        }
    }

    fn stream_logs_task(&self, server_id: i32, _pool: PgPool) -> Option<Receiver<String>> {
        let provisioner = Arc::clone(&self.provisioner);
        let pool_lookup = self.pool.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            match sqlx::query(
                r#"SELECT instance_id FROM runtime_vm_instances WHERE server_id = $1 AND terminated_at IS NULL ORDER BY created_at DESC LIMIT 1"#,
            )
            .bind(server_id)
            .fetch_optional(&pool_lookup)
            .await
            {
                Ok(Some(row)) => {
                    let instance_id: String = row.get("instance_id");
                    match provisioner.stream_logs(&instance_id).await {
                        Ok(Some(mut upstream)) => {
                            while let Some(line) = upstream.recv().await {
                                if tx.send(line).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            tracing::debug!(%server_id, "vm provisioner does not expose log stream");
                        }
                        Err(err) => {
                            tracing::error!(?err, %server_id, "failed to open vm log stream");
                        }
                    }
                }
                Ok(None) => {
                    tracing::warn!(%server_id, "no active vm instance for log stream");
                }
                Err(err) => {
                    tracing::error!(?err, %server_id, "failed to fetch vm instance for log stream");
                }
            }
        });
        Some(rx)
    }
}

#[derive(Clone)]
pub struct HttpHypervisorProvisioner {
    client: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
    log_tail: usize,
}

#[derive(Debug, Deserialize)]
struct HypervisorInstanceResponse {
    instance_id: String,
    #[serde(default)]
    isolation_tier: Option<String>,
    #[serde(default, rename = "attestation")]
    attestation_evidence: Option<Value>,
    #[serde(default)]
    image: Option<String>,
}

impl HttpHypervisorProvisioner {
    pub fn new(
        base_url: impl Into<String>,
        auth_token: Option<String>,
        log_tail: usize,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build hypervisor client")?;
        Ok(Self {
            client,
            base_url: base_url.into(),
            auth_token,
            log_tail,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.auth_token {
            request.bearer_auth(token)
        } else {
            request
        }
    }

    fn hypervisor_snapshot(&self) -> Option<HypervisorSnapshot> {
        let credentials = self
            .auth_token
            .as_ref()
            .map(|_| json!({ "method": "bearer", "configured": true }));
        Some(HypervisorSnapshot::new(
            self.base_url.clone(),
            credentials,
            None,
            None,
            None,
        ))
    }
}

#[async_trait]
impl VmProvisioner for HttpHypervisorProvisioner {
    async fn provision(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        config: Option<&Value>,
    ) -> Result<VmProvisioningResult> {
        let mut payload = json!({
            "server_id": server_id,
            "image": decision.image,
            "tier": decision.tier,
            "capabilities": decision
                .capability_requirements
                .iter()
                .map(|cap| cap.as_str())
                .collect::<Vec<_>>(),
        });

        if let Some(tier) = &decision.tier {
            payload["tier"] = json!(tier);
        }

        if let Some(config) = config {
            payload["config"] = config.clone();
        }

        let response = self
            .auth(self.client.post(self.endpoint("instances")))
            .json(&payload)
            .send()
            .await
            .context("failed to contact hypervisor")?
            .error_for_status()
            .context("hypervisor rejected provisioning request")?;

        let parsed: HypervisorInstanceResponse = response
            .json()
            .await
            .context("failed to decode hypervisor response")?;

        Ok(VmProvisioningResult::new(
            parsed.instance_id,
            parsed.isolation_tier.or_else(|| decision.tier.clone()),
            parsed.attestation_evidence,
            parsed.image.unwrap_or_else(|| decision.image.clone()),
            self.hypervisor_snapshot(),
        ))
    }

    async fn start(&self, instance_id: &str) -> Result<()> {
        self.auth(
            self.client
                .post(self.endpoint(&format!("instances/{instance_id}/start"))),
        )
        .send()
        .await
        .context("failed to reach hypervisor for start")?
        .error_for_status()
        .context("hypervisor rejected start request")?;
        Ok(())
    }

    async fn stop(&self, instance_id: &str) -> Result<()> {
        self.auth(
            self.client
                .post(self.endpoint(&format!("instances/{instance_id}/stop"))),
        )
        .send()
        .await
        .context("failed to reach hypervisor for stop")?
        .error_for_status()
        .context("hypervisor rejected stop request")?;
        Ok(())
    }

    async fn teardown(&self, instance_id: &str) -> Result<()> {
        self.auth(
            self.client
                .delete(self.endpoint(&format!("instances/{instance_id}"))),
        )
        .send()
        .await
        .context("failed to reach hypervisor for teardown")?
        .error_for_status()
        .context("hypervisor rejected teardown request")?;
        Ok(())
    }

    async fn fetch_logs(&self, instance_id: &str) -> Result<String> {
        let response = self
            .auth(
                self.client
                    .get(self.endpoint(&format!("instances/{instance_id}/logs")))
                    .query(&[("tail", self.log_tail)]),
            )
            .send()
            .await
            .context("failed to reach hypervisor for logs")?
            .error_for_status()
            .context("hypervisor rejected log fetch")?;
        Ok(response.text().await.context("failed to read log body")?)
    }

    async fn stream_logs(&self, instance_id: &str) -> Result<Option<Receiver<String>>> {
        let response = self
            .auth(
                self.client
                    .get(self.endpoint(&format!("instances/{instance_id}/logs/stream"))),
            )
            .send()
            .await
            .context("failed to reach hypervisor for log stream")?;

        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }

        let mut stream = response
            .error_for_status()
            .context("hypervisor rejected log stream request")?
            .bytes_stream();

        let (tx, rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            while let Some(item) = stream.next().await {
                match item {
                    Ok(chunk) => {
                        buffer.extend_from_slice(&chunk);
                        while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                            let line = buffer.drain(..=pos).collect::<Vec<u8>>();
                            let clean =
                                String::from_utf8_lossy(&line[..line.len().saturating_sub(1)])
                                    .to_string();
                            if tx.send(clean).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!(?err, "log stream chunk error");
                        break;
                    }
                }
            }
            if !buffer.is_empty() {
                match String::from_utf8(buffer) {
                    Ok(tail) => {
                        let _ = tx.send(tail).await;
                    }
                    Err(err) => {
                        let recovered = String::from_utf8_lossy(&err.into_bytes()).to_string();
                        let _ = tx.send(recovered).await;
                    }
                }
            }
        });

        Ok(Some(rx))
    }
}
