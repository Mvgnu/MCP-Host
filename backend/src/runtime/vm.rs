use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use sqlx::Row;
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

use crate::policy::{PolicyDecision, RuntimeBackend};
use crate::servers::{add_metric, set_status};

// key: runtime-vm-executor -> attestation,policy-hooks

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmProvisioningResult {
    pub instance_id: String,
    pub isolation_tier: Option<String>,
    pub attestation_evidence: Option<Value>,
    pub requested_image: String,
}

impl VmProvisioningResult {
    pub fn new(
        instance_id: String,
        isolation_tier: Option<String>,
        attestation_evidence: Option<Value>,
        requested_image: String,
    ) -> Self {
        Self {
            instance_id,
            isolation_tier,
            attestation_evidence,
            requested_image,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttestationStatus {
    Trusted,
    Untrusted,
    Unknown,
}

impl AttestationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AttestationStatus::Trusted => "trusted",
            AttestationStatus::Untrusted => "untrusted",
            AttestationStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationOutcome {
    pub status: AttestationStatus,
    pub evidence: Option<Value>,
    pub notes: Vec<String>,
}

impl AttestationOutcome {
    pub fn trusted(evidence: Option<Value>, notes: Vec<String>) -> Self {
        Self {
            status: AttestationStatus::Trusted,
            evidence,
            notes,
        }
    }

    pub fn untrusted(evidence: Option<Value>, notes: Vec<String>) -> Self {
        Self {
            status: AttestationStatus::Untrusted,
            evidence,
            notes,
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

#[async_trait]
pub trait AttestationVerifier: Send + Sync {
    async fn verify(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        provisioning: &VmProvisioningResult,
        config: Option<&Value>,
    ) -> Result<AttestationOutcome>;
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

        let record_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO runtime_vm_instances (
                server_id,
                instance_id,
                isolation_tier,
                attestation_status,
                attestation_evidence,
                policy_version,
                capability_notes
            ) VALUES ($1, $2, $3, 'pending', $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(server_id)
        .bind(&provisioning.instance_id)
        .bind(provisioning.isolation_tier.as_deref())
        .bind(provisioning.attestation_evidence.clone())
        .bind(&decision.policy_version)
        .bind(&notes)
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
                add_metric(
                    &pool,
                    server_id,
                    "vm.provision.start",
                    Some(&start_metric),
                )
                .await
                .ok();

                let provisioning = provisioner
                    .provision(server_id, &decision, config.as_ref())
                    .await
                    .context("provisioner failed to allocate VM")?;

                vm_id = Some(
                    VirtualMachineExecutor::persist_instance(&pool, server_id, &provisioning, &decision)
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

                if let Some(record_id) = vm_id {
                    VirtualMachineExecutor::record_event(
                        &pool,
                        record_id,
                        "attestation",
                        json!({
                            "status": attestation.status.as_str(),
                            "notes": attestation.notes,
                        }),
                    )
                    .await
                    .ok();

                    sqlx::query(
                        "UPDATE runtime_vm_instances SET attestation_status = $1, attestation_evidence = COALESCE($2, attestation_evidence) WHERE id = $3",
                    )
                    .bind(attestation.status.as_str())
                    .bind(attestation.evidence.clone())
                    .bind(record_id)
                    .execute(&pool)
                    .await
                    .ok();
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
                        let success_metric =
                            json!({ "instance_id": provisioning.instance_id });
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
        let (tx, rx) = tokio::sync::mpsc::channel(32);
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

pub struct LocalVmProvisioner;

#[async_trait]
impl VmProvisioner for LocalVmProvisioner {
    async fn provision(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        config: Option<&Value>,
    ) -> Result<VmProvisioningResult> {
        let instance_id = format!("vm-{}", Uuid::new_v4());
        let evidence = config
            .and_then(|cfg| cfg.get("attestation"))
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "issued_at": Utc::now().to_rfc3339(),
                    "server_id": server_id,
                    "policy_version": decision.policy_version,
                })
            });
        Ok(VmProvisioningResult::new(
            instance_id,
            decision.tier.clone(),
            Some(evidence),
            decision.image.clone(),
        ))
    }

    async fn start(&self, _instance_id: &str) -> Result<()> {
        Ok(())
    }

    async fn stop(&self, _instance_id: &str) -> Result<()> {
        Ok(())
    }

    async fn teardown(&self, _instance_id: &str) -> Result<()> {
        Ok(())
    }

    async fn fetch_logs(&self, _instance_id: &str) -> Result<String> {
        Ok("vm log capture is not yet implemented".to_string())
    }

    async fn stream_logs(&self, _instance_id: &str) -> Result<Option<Receiver<String>>> {
        Ok(None)
    }
}

pub struct InlineEvidenceAttestor;

#[async_trait]
impl AttestationVerifier for InlineEvidenceAttestor {
    async fn verify(
        &self,
        _server_id: i32,
        _decision: &PolicyDecision,
        provisioning: &VmProvisioningResult,
        _config: Option<&Value>,
    ) -> Result<AttestationOutcome> {
        let evidence = provisioning.attestation_evidence.clone();
        let trusted = evidence
            .as_ref()
            .and_then(|value| value.get("trusted"))
            .and_then(|value| value.as_bool())
            .unwrap_or(true);

        if trusted {
            Ok(AttestationOutcome::trusted(
                evidence,
                vec!["attestation:trusted".to_string()],
            ))
        } else {
            Ok(AttestationOutcome::untrusted(
                provisioning.attestation_evidence.clone(),
                vec!["attestation:rejected".to_string()],
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn attestation_status_strings_are_stable() {
        assert_eq!(AttestationStatus::Trusted.as_str(), "trusted");
        assert_eq!(AttestationStatus::Untrusted.as_str(), "untrusted");
        assert_eq!(AttestationStatus::Unknown.as_str(), "unknown");
    }
}
