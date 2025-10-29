pub mod trust;

use std::collections::{HashMap, HashSet};
use std::convert::{Infallible, TryFrom};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use axum::{
    extract::Query,
    response::sse::{Event, Sse},
};
use chrono::{DateTime, Duration, Utc};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;

use crate::config;
use crate::db::runtime_vm_trust_history::{
    latest_for_instance as latest_trust_event, RuntimeVmTrustEvent,
};
use crate::evaluations::{self, CertificationStatus};
use crate::extractor::AuthUser;
use crate::governance::GovernanceEngine;
use crate::intelligence::{self, IntelligenceError, IntelligenceStatus, RecomputeContext};
use crate::job_queue;
use crate::keys::{ProviderKeyDecisionPosture, ProviderKeyService, ProviderKeyServiceConfig};
use crate::marketplace::{classify_tier, derive_health, MarketplacePlatform};

// key: runtime-policy -> placement-decisions,marketplace-health

const POLICY_VERSION: &str = "runtime-policy-v0.1";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyEventType {
    Decision,
    Attestation,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyEvent {
    #[serde(skip_serializing)]
    pub owner_id: i32,
    pub server_id: i32,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: PolicyEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub governance_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_key_posture: Option<ProviderKeyDecisionPosture>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

static POLICY_EVENT_CHANNEL: Lazy<broadcast::Sender<PolicyEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(64);
    tx
});

#[derive(Debug, Default, Deserialize)]
pub struct PolicyWatchParams {
    #[serde(default)]
    pub server_id: Option<i32>,
}

pub fn publish_policy_event(event: PolicyEvent) {
    let _ = POLICY_EVENT_CHANNEL.send(event);
}

fn subscribe_policy_events() -> broadcast::Receiver<PolicyEvent> {
    POLICY_EVENT_CHANNEL.subscribe()
}

pub async fn stream_policy_events(
    AuthUser { user_id, .. }: AuthUser,
    Query(params): Query<PolicyWatchParams>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>> + Send> {
    let target_server = params.server_id;
    let receiver = subscribe_policy_events();
    let stream = BroadcastStream::new(receiver).filter_map(move |item| {
        let filter_user = user_id;
        let target = target_server;
        async move {
            match item {
                Ok(event) if event.owner_id == filter_user => {
                    if let Some(server) = target {
                        if server != event.server_id {
                            return None;
                        }
                    }
                    match serde_json::to_string(&event) {
                        Ok(payload) => Some(Ok(Event::default().data(payload))),
                        Err(err) => {
                            tracing::error!(?err, "failed to serialize policy event");
                            None
                        }
                    }
                }
                Ok(_) => None,
                Err(err) => {
                    tracing::debug!(?err, "dropped policy event subscriber update");
                    None
                }
            }
        }
    });
    Sse::new(stream)
}

impl PolicyEvent {
    pub fn decision(
        owner_id: i32,
        server_id: i32,
        decision: &PolicyDecision,
        posture: Option<&VmAttestationPolicyOutcome>,
    ) -> Self {
        let fallback_backend = posture
            .and_then(|outcome| outcome.backend_override)
            .map(|backend| backend.as_str().to_string());
        let attestation_status = posture.and_then(|outcome| outcome.attestation_status.clone());
        let stale_flag =
            posture.and_then(|outcome| outcome.attestation_status.as_ref().map(|_| outcome.stale));
        let mut notes = decision.notes.clone();
        if !decision.promotion_notes.is_empty() {
            notes.extend(decision.promotion_notes.clone());
        }
        Self {
            owner_id,
            server_id,
            timestamp: Utc::now(),
            event_type: PolicyEventType::Decision,
            backend: Some(decision.backend.as_str().to_string()),
            candidate_backend: Some(decision.candidate_backend.as_str().to_string()),
            fallback_backend,
            attestation_status,
            evaluation_required: Some(decision.evaluation_required),
            governance_required: Some(decision.governance_required),
            instance_id: None,
            stale: stale_flag,
            provider_key_posture: decision.provider_key_posture.clone(),
            notes,
        }
    }

    pub fn attestation(
        owner_id: i32,
        server_id: i32,
        backend: &RuntimeBackend,
        instance_id: Option<String>,
        status: String,
        notes: Vec<String>,
        fallback_backend: Option<RuntimeBackend>,
    ) -> Self {
        let stale = notes.iter().any(|note| note == "attestation:stale");
        Self {
            owner_id,
            server_id,
            timestamp: Utc::now(),
            event_type: PolicyEventType::Attestation,
            backend: Some(backend.as_str().to_string()),
            candidate_backend: None,
            fallback_backend: fallback_backend.map(|backend| backend.as_str().to_string()),
            attestation_status: Some(status),
            evaluation_required: None,
            governance_required: None,
            instance_id,
            stale: Some(stale),
            provider_key_posture: None,
            notes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeBackend {
    Docker,
    Kubernetes,
    VirtualMachine,
}

impl RuntimeBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeBackend::Docker => "docker",
            RuntimeBackend::Kubernetes => "kubernetes",
            RuntimeBackend::VirtualMachine => "virtual-machine",
        }
    }
}

impl fmt::Display for RuntimeBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RuntimeBackend {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "docker" => Ok(RuntimeBackend::Docker),
            "kubernetes" => Ok(RuntimeBackend::Kubernetes),
            "virtual-machine" => Ok(RuntimeBackend::VirtualMachine),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeCapability {
    Gpu,
    ImageBuild,
}

impl RuntimeCapability {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeCapability::Gpu => "gpu",
            RuntimeCapability::ImageBuild => "image-build",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeExecutorDescriptor {
    pub backend: RuntimeBackend,
    pub display_name: String,
    pub capabilities: HashSet<RuntimeCapability>,
}

impl RuntimeExecutorDescriptor {
    pub fn new(
        backend: RuntimeBackend,
        display_name: impl Into<String>,
        capabilities: impl IntoIterator<Item = RuntimeCapability>,
    ) -> Self {
        Self {
            backend,
            display_name: display_name.into(),
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn supports(&self, requirement: &RuntimeCapability) -> bool {
        self.capabilities.contains(requirement)
    }

    pub fn supports_all(&self, requirements: &[RuntimeCapability]) -> bool {
        requirements.iter().all(|req| self.supports(req))
    }

    pub fn capability_keys(&self) -> Vec<&'static str> {
        self.capabilities
            .iter()
            .map(RuntimeCapability::as_str)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct PolicyDecision {
    pub backend: RuntimeBackend,
    pub candidate_backend: RuntimeBackend,
    pub image: String,
    pub requires_build: bool,
    pub artifact_run_id: Option<i32>,
    pub manifest_digest: Option<String>,
    pub policy_version: String,
    pub evaluation_required: bool,
    pub governance_required: bool,
    pub governance_run_id: Option<i64>,
    pub tier: Option<String>,
    pub health_overall: Option<String>,
    pub capability_requirements: Vec<RuntimeCapability>,
    pub capabilities_satisfied: bool,
    pub executor_name: Option<String>,
    pub notes: Vec<String>,
    pub promotion_track_id: Option<i32>,
    pub promotion_track_name: Option<String>,
    pub promotion_stage: Option<String>,
    pub promotion_status: Option<String>,
    pub promotion_notes: Vec<String>,
    pub provider_key_posture: Option<ProviderKeyDecisionPosture>,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct RuntimePolicyEngine {
    default_backend: RuntimeBackend,
    executors: Arc<RwLock<HashMap<RuntimeBackend, RuntimeExecutorDescriptor>>>,
    governance: Arc<RwLock<Option<Arc<GovernanceEngine>>>>,
}

impl RuntimePolicyEngine {
    pub fn new(default_backend: RuntimeBackend) -> Self {
        Self {
            default_backend,
            executors: Arc::new(RwLock::new(HashMap::new())),
            governance: Arc::new(RwLock::new(None)),
        }
    }

    pub fn default_backend(&self) -> RuntimeBackend {
        self.default_backend
    }

    pub async fn attach_governance(&self, engine: Arc<GovernanceEngine>) {
        let mut guard = self.governance.write().await;
        *guard = Some(engine);
    }

    pub async fn register_executor(&self, descriptor: RuntimeExecutorDescriptor) {
        let mut executors = self.executors.write().await;
        executors.insert(descriptor.backend, descriptor);
    }

    pub async fn executor_descriptor(
        &self,
        backend: RuntimeBackend,
    ) -> Option<RuntimeExecutorDescriptor> {
        let executors = self.executors.read().await;
        executors.get(&backend).cloned()
    }

    pub async fn decide_and_record(
        self: &Arc<Self>,
        pool: &PgPool,
        server_id: i32,
        server_type: &str,
        config: Option<&Value>,
        use_gpu: bool,
    ) -> Result<PolicyDecision, PolicyError> {
        let (decision, vm_posture) = self
            .evaluate(pool, server_id, server_type, config, use_gpu)
            .await?;
        let decision_id = self.record_decision(pool, server_id, &decision).await?;

        job_queue::enqueue_intelligence_refresh(pool, server_id).await;

        if let Some(run_id) = decision.governance_run_id {
            if let Some(engine) = self.governance.read().await.clone() {
                if let Err(err) = engine
                    .attach_policy_decision(pool, run_id, decision_id)
                    .await
                {
                    tracing::warn!(
                        ?err,
                        %server_id,
                        %run_id,
                        decision_id,
                        "failed to link governance run to policy decision",
                    );
                }
            }
        }

        match sqlx::query("SELECT owner_id FROM mcp_servers WHERE id = $1")
            .bind(server_id)
            .fetch_optional(pool)
            .await
        {
            Ok(Some(row)) => {
                let owner_id: i32 = row.get("owner_id");
                publish_policy_event(PolicyEvent::decision(
                    owner_id,
                    server_id,
                    &decision,
                    vm_posture.as_ref(),
                ));
            }
            Ok(None) => tracing::warn!(
                %server_id,
                "policy decision recorded for missing server",
            ),
            Err(err) => tracing::warn!(
                ?err,
                %server_id,
                "failed to publish policy decision event due to owner lookup error",
            ),
        }
        Ok(decision)
    }

    async fn evaluate(
        &self,
        pool: &PgPool,
        server_id: i32,
        server_type: &str,
        config: Option<&Value>,
        use_gpu: bool,
    ) -> Result<(PolicyDecision, Option<VmAttestationPolicyOutcome>), PolicyError> {
        let mut notes = Vec::new();
        let mut backend = self.default_backend;
        let mut capability_requirements = Vec::new();
        let mut governance_required = false;
        let mut governance_run_id = None;
        let mut promotion_track_id = None;
        let mut promotion_track_name = None;
        let mut promotion_stage = None;
        let mut promotion_status = None;
        let mut promotion_notes = Vec::new();
        let mut evaluation_required = false;
        let mut vm_posture: Option<VmAttestationPolicyOutcome> = None;
        let mut provider_key_posture: Option<ProviderKeyDecisionPosture> = None;

        if use_gpu && !matches!(backend, RuntimeBackend::Kubernetes) {
            backend = RuntimeBackend::Kubernetes;
            notes.push("gpu:requested -> backend:kubernetes".to_string());
            capability_requirements.push(RuntimeCapability::Gpu);
        }

        if let Some(runtime_override) = config
            .and_then(|v| v.get("runtime"))
            .and_then(|v| v.as_str())
        {
            match runtime_override {
                "docker" => {
                    if !matches!(backend, RuntimeBackend::Docker) {
                        notes.push("runtime_override:docker".to_string());
                    }
                    backend = RuntimeBackend::Docker;
                }
                "kubernetes" => {
                    if !matches!(backend, RuntimeBackend::Kubernetes) {
                        notes.push("runtime_override:kubernetes".to_string());
                    }
                    backend = RuntimeBackend::Kubernetes;
                }
                "virtual-machine" => {
                    if !matches!(backend, RuntimeBackend::VirtualMachine) {
                        notes.push("runtime_override:virtual-machine".to_string());
                    }
                    backend = RuntimeBackend::VirtualMachine;
                }
                other => notes.push(format!("runtime_override:unknown:{other}")),
            }
        }

        let requested_image = config
            .and_then(|v| v.get("image"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());

        let mut image = requested_image
            .clone()
            .unwrap_or_else(|| default_image_for(server_type).to_string());

        let requires_build = config
            .and_then(|v| v.get("repo_url"))
            .and_then(|v| v.as_str())
            .is_some();

        if requires_build {
            notes.push("build:git-requested".to_string());
            capability_requirements.push(RuntimeCapability::ImageBuild);
        }

        let artifact_row = sqlx::query(
            r#"
            SELECT
                id,
                registry_image,
                local_image,
                manifest_digest,
                status,
                multi_arch,
                credential_health_status
            FROM build_artifact_runs
            WHERE server_id = $1
            ORDER BY completed_at DESC
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .fetch_optional(pool)
        .await?;

        let mut artifact_run_id = None;
        let mut manifest_digest = None;
        let mut tier = None;
        let mut health_overall = None;

        if let Some(row) = artifact_row {
            let run_id: i32 = row.get("id");
            let registry_image: Option<String> = row.get("registry_image");
            let local_image: String = row.get("local_image");
            let manifest_digest_value: Option<String> = row.get("manifest_digest");
            let status: String = row.get("status");
            let multi_arch: bool = row.get("multi_arch");
            let credential_health_status: String = row.get("credential_health_status");

            artifact_run_id = Some(run_id);
            manifest_digest = manifest_digest_value;

            let platform_rows = sqlx::query(
                r#"
                SELECT
                    platform,
                    remote_image,
                    remote_tag,
                    digest,
                    auth_refresh_attempted,
                    auth_refresh_succeeded,
                    auth_rotation_attempted,
                    auth_rotation_succeeded,
                    credential_health_status
                FROM build_artifact_platforms
                WHERE run_id = $1
                ORDER BY platform
                "#,
            )
            .bind(run_id)
            .fetch_all(pool)
            .await?;

            let mut platforms = Vec::with_capacity(platform_rows.len());
            for platform in platform_rows {
                platforms.push(MarketplacePlatform {
                    platform: platform.get("platform"),
                    remote_image: platform.get("remote_image"),
                    remote_tag: platform.get("remote_tag"),
                    digest: platform.get("digest"),
                    auth_refresh_attempted: platform.get("auth_refresh_attempted"),
                    auth_refresh_succeeded: platform.get("auth_refresh_succeeded"),
                    auth_rotation_attempted: platform.get("auth_rotation_attempted"),
                    auth_rotation_succeeded: platform.get("auth_rotation_succeeded"),
                    credential_health_status: platform.get("credential_health_status"),
                });
            }

            let health = derive_health(&status, &credential_health_status, &platforms);
            health_overall = Some(health.overall.clone());
            tier = Some(classify_tier(server_type.to_string(), multi_arch, &health));

            notes.push(format!(
                "artifact:{}:status:{}:health:{}",
                run_id, status, health.overall
            ));

            if requested_image.is_none() {
                if let Some(registry_image) = registry_image {
                    image = registry_image;
                    notes.push("image:registry-promoted".to_string());
                } else {
                    image = local_image;
                    notes.push("image:local-build".to_string());
                }
            }
        } else {
            notes.push("artifact:none".to_string());
        }

        if matches!(backend, RuntimeBackend::VirtualMachine) {
            let fallback_backend = if use_gpu {
                RuntimeBackend::Kubernetes
            } else {
                RuntimeBackend::Docker
            };
            let stale_seconds =
                i64::try_from(*config::VM_ATTESTATION_MAX_AGE_SECONDS).unwrap_or(300);
            let stale_limit = Duration::seconds(stale_seconds.max(60));
            let vm_row = sqlx::query(
                r#"
                SELECT id, attestation_status, updated_at, terminated_at
                FROM runtime_vm_instances
                WHERE server_id = $1
                ORDER BY created_at DESC
                LIMIT 1
                "#,
            )
            .bind(server_id)
            .fetch_optional(pool)
            .await?;

            let mut record = vm_row.map(|row| VmAttestationRecord {
                instance_id: row.get("id"),
                status: row.get("attestation_status"),
                updated_at: row.get("updated_at"),
                terminated_at: row.get("terminated_at"),
                trust_event: None,
            });

            if let Some(ref mut record) = record {
                if let Ok(Some(event)) = latest_trust_event(pool, record.instance_id).await {
                    record.trust_event = Some(event);
                }
            }

            let posture =
                evaluate_vm_attestation_posture(record, Utc::now(), stale_limit, fallback_backend);
            notes.extend(posture.notes.clone().into_iter());
            if let Some(override_backend) = posture.backend_override {
                backend = override_backend;
            }
            if posture.evaluation_required {
                evaluation_required = true;
            }
            vm_posture = Some(posture);
        }

        if !capability_requirements.is_empty() {
            let reqs = capability_requirements
                .iter()
                .map(|cap| cap.as_str())
                .collect::<Vec<_>>()
                .join(",");
            notes.push(format!("capabilities:requested:{reqs}"));
        }

        let candidate_backend = backend;
        let (backend, capabilities_satisfied, executor_name) = self
            .select_backend(candidate_backend, &capability_requirements, &mut notes)
            .await;

        let capability_keys: Vec<String> = capability_requirements
            .iter()
            .map(|cap| cap.as_str().to_string())
            .collect();

        let intelligence_context = RecomputeContext {
            server_id,
            backend: backend.as_str(),
            tier: tier.as_deref(),
            capability_keys: &capability_keys,
            fallback_capabilities_satisfied: capabilities_satisfied,
        };

        let intelligence_scores = intelligence::ensure_scores(pool, &intelligence_context)
            .await
            .map_err(|err| match err {
                IntelligenceError::Database(db_err) => PolicyError::Database(db_err),
            })?;

        if intelligence_scores.is_empty() {
            evaluation_required = true;
            notes.push("intelligence:missing-scores".to_string());
        }

        for (capability, score) in &intelligence_scores {
            let threshold = intelligence::minimum_threshold(capability, tier.as_deref());
            notes.push(format!(
                "intelligence:{}:{:.1}:threshold:{:.1}",
                capability, score.score, threshold
            ));
            if score.score < threshold || matches!(score.status, IntelligenceStatus::Critical) {
                evaluation_required = true;
                notes.push(format!(
                    "intelligence:degraded:{}:{}:{:.1}",
                    capability,
                    match score.status {
                        IntelligenceStatus::Healthy => "healthy",
                        IntelligenceStatus::Warning => "warning",
                        IntelligenceStatus::Critical => "critical",
                    },
                    score.score
                ));
                if matches!(score.status, IntelligenceStatus::Critical) {
                    governance_required = true;
                }
            } else {
                notes.push(format!(
                    "intelligence:stable:{}:{}:{:.1}",
                    capability,
                    match score.status {
                        IntelligenceStatus::Healthy => "healthy",
                        IntelligenceStatus::Warning => "warning",
                        IntelligenceStatus::Critical => "critical",
                    },
                    score.score
                ));
            }
        }

        if requires_build {
            evaluation_required = true;
            notes.push("evaluation:reason:requires-build".to_string());
        }

        match health_overall.as_deref() {
            Some("healthy") => {}
            Some(status) => {
                evaluation_required = true;
                notes.push(format!("evaluation:reason:health:{status}"));
            }
            None => {
                evaluation_required = true;
                notes.push("evaluation:reason:health:unknown".to_string());
            }
        }

        if !capabilities_satisfied {
            evaluation_required = true;
            notes.push("evaluation:reason:capabilities".to_string());
        }

        let mut certification_blocked = false;
        if let Some(manifest_digest) = &manifest_digest {
            if let Some(tier_value) = tier.as_ref() {
                let latest =
                    evaluations::latest_per_requirement(pool, manifest_digest, tier_value).await?;
                if latest.is_empty() {
                    certification_blocked = true;
                    notes.push(format!(
                        "evaluation:missing-certification:{}:{}",
                        tier_value, manifest_digest
                    ));
                } else {
                    let mut entries: Vec<_> = latest.into_iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                    let now = Utc::now();
                    for (requirement, certification) in entries {
                        let active = certification.is_active(now);
                        let mut stale_detail: Option<String> = None;
                        if let Some(next_refresh) = certification.next_refresh_at {
                            if next_refresh <= now {
                                stale_detail = Some(format!("due@{}", next_refresh.to_rfc3339()));
                            }
                        } else if certification.refresh_cadence_seconds.is_some() {
                            stale_detail = Some("missing-next-refresh".to_string());
                        }
                        if let Some(detail) = stale_detail.as_ref() {
                            certification_blocked = true;
                            notes.push(format!(
                                "evaluation:stale:{}:{}:{}",
                                tier_value, requirement, detail
                            ));
                        }
                        if let Some(source) = &certification.evidence_source {
                            if let Ok(serialized) = serde_json::to_string(source) {
                                notes.push(format!(
                                    "evaluation:provenance:{}:{}:{}",
                                    tier_value, requirement, serialized
                                ));
                            }
                        }
                        match certification.status {
                            CertificationStatus::Pass if active && stale_detail.is_none() => {
                                notes.push(format!(
                                    "evaluation:certified:{}:{}",
                                    tier_value, requirement
                                ));
                            }
                            CertificationStatus::Pass if active => {
                                certification_blocked = true;
                            }
                            CertificationStatus::Pass => {
                                certification_blocked = true;
                                notes.push(format!(
                                    "evaluation:expired:{}:{}",
                                    tier_value, requirement
                                ));
                            }
                            CertificationStatus::Pending => {
                                certification_blocked = true;
                                let state = if active {
                                    "pending"
                                } else {
                                    "pending-inactive"
                                };
                                notes.push(format!(
                                    "evaluation:{}:{}:{}",
                                    state, tier_value, requirement
                                ));
                            }
                            CertificationStatus::Fail => {
                                certification_blocked = true;
                                let state = if active { "failed" } else { "failed-inactive" };
                                notes.push(format!(
                                    "evaluation:{}:{}:{}",
                                    state, tier_value, requirement
                                ));
                            }
                        }
                    }
                }
            } else {
                certification_blocked = true;
                notes.push("evaluation:missing-tier".to_string());
            }
        } else {
            certification_blocked = true;
            notes.push("evaluation:missing-manifest".to_string());
        }

        evaluation_required |= certification_blocked;

        let governance_engine = self.governance.read().await.clone();
        if let Some(engine) = governance_engine {
            match engine
                .ensure_promotion_ready(pool, manifest_digest.as_deref(), tier.as_deref())
                .await
            {
                Ok(gate) => {
                    let run_id = gate.run_id;
                    let satisfied = gate.satisfied;
                    promotion_track_id = gate.promotion_track_id;
                    promotion_track_name = gate.promotion_track_name.clone();
                    promotion_stage = gate.promotion_stage.clone();
                    promotion_status = gate.promotion_status.clone();
                    promotion_notes = gate.notes.clone();
                    notes.extend(gate.notes);
                    if satisfied {
                        governance_run_id = run_id;
                    } else {
                        governance_required = true;
                    }
                }
                Err(err) => {
                    notes.push(format!("governance:error:{err}"));
                    governance_required = true;
                }
            }
        }

        if let Some(tier_name) = tier.clone() {
            let key_service =
                ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());
            if let Some(requirement) = key_service
                .tier_requirement(&tier_name)
                .await
                .map_err(PolicyError::Database)?
            {
                let summary = key_service
                    .summarize_for_policy(requirement.provider_id)
                    .await
                    .map_err(PolicyError::Database)?;
                let gating_veto = requirement.byok_required && summary.vetoed;
                let mut posture_notes = summary.notes.clone();
                if requirement.byok_required && !summary.vetoed {
                    posture_notes.push("healthy".to_string());
                } else if !requirement.byok_required {
                    posture_notes.push("optional".to_string());
                }

                if gating_veto {
                    evaluation_required = true;
                    governance_required = true;
                    if summary.notes.is_empty() {
                        notes.push("provider-key:veto".to_string());
                    }
                    for note in summary.notes.iter() {
                        notes.push(format!("provider-key:{note}"));
                    }
                } else if requirement.byok_required {
                    notes.push("provider-key:healthy".to_string());
                } else {
                    notes.push("provider-key:optional".to_string());
                }

                provider_key_posture = Some(ProviderKeyDecisionPosture {
                    provider_id: requirement.provider_id,
                    provider_key_id: summary.record.as_ref().map(|record| record.id),
                    tier: Some(tier_name),
                    state: summary.posture_state(),
                    rotation_due_at: summary
                        .record
                        .as_ref()
                        .and_then(|record| record.rotation_due_at),
                    attestation_registered: summary
                        .record
                        .as_ref()
                        .and_then(|record| record.attestation_digest.as_ref())
                        .is_some(),
                    attestation_signature_verified: summary
                        .record
                        .as_ref()
                        .map(|record| record.attestation_signature_registered)
                        .unwrap_or(false),
                    attestation_verified_at: summary
                        .record
                        .as_ref()
                        .and_then(|record| record.attestation_verified_at),
                    vetoed: gating_veto,
                    notes: posture_notes,
                });
            }
        }

        Ok((
            PolicyDecision {
                backend,
                candidate_backend,
                image,
                requires_build,
                artifact_run_id,
                manifest_digest,
                policy_version: POLICY_VERSION.to_string(),
                evaluation_required,
                governance_required,
                governance_run_id,
                tier,
                health_overall,
                capability_requirements,
                capabilities_satisfied,
                executor_name,
                notes,
                promotion_track_id,
                promotion_track_name,
                promotion_stage,
                promotion_status,
                promotion_notes,
                provider_key_posture,
            },
            vm_posture,
        ))
    }

    async fn record_decision(
        &self,
        pool: &PgPool,
        server_id: i32,
        decision: &PolicyDecision,
    ) -> Result<i32, PolicyError> {
        let notes_json =
            serde_json::Value::Array(decision.notes.iter().cloned().map(Value::String).collect());
        let capability_json = serde_json::Value::Array(
            decision
                .capability_requirements
                .iter()
                .map(|cap| Value::String(cap.as_str().to_string()))
                .collect(),
        );
        let key_posture_json = decision
            .provider_key_posture
            .as_ref()
            .map(|posture| serde_json::to_value(posture).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);

        let row = sqlx::query(
            r#"
            INSERT INTO runtime_policy_decisions (
                server_id,
                candidate_backend,
                backend,
                image,
                requires_build,
                artifact_run_id,
                manifest_digest,
                policy_version,
                evaluation_required,
                governance_required,
                governance_run_id,
                tier,
                health_overall,
                capability_requirements,
                capabilities_satisfied,
                executor_name,
                notes,
                promotion_track_id,
                promotion_stage,
                promotion_status,
                promotion_notes,
                key_posture,
                decided_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22
            )
            RETURNING id
            "#,
        )
        .bind(server_id)
        .bind(decision.candidate_backend.as_str())
        .bind(decision.backend.as_str())
        .bind(&decision.image)
        .bind(decision.requires_build)
        .bind(decision.artifact_run_id)
        .bind(&decision.manifest_digest)
        .bind(&decision.policy_version)
        .bind(decision.evaluation_required)
        .bind(decision.governance_required)
        .bind(decision.governance_run_id)
        .bind(decision.tier.as_deref())
        .bind(decision.health_overall.as_deref())
        .bind(capability_json)
        .bind(decision.capabilities_satisfied)
        .bind(decision.executor_name.as_deref())
        .bind(notes_json)
        .bind(decision.promotion_track_id)
        .bind(decision.promotion_stage.as_deref())
        .bind(decision.promotion_status.as_deref())
        .bind(&decision.promotion_notes)
        .bind(key_posture_json)
        .bind(Utc::now())
        .fetch_one(pool)
        .await?;

        tracing::info!(
            target: "runtime.policy",
            %server_id,
            backend = %decision.backend.as_str(),
            image = %decision.image,
            requires_build = decision.requires_build,
            artifact_run = ?decision.artifact_run_id,
            tier = ?decision.tier,
            health = ?decision.health_overall,
            evaluation_required = decision.evaluation_required,
            governance_required = decision.governance_required,
            policy_version = %decision.policy_version,
            "recorded runtime policy decision"
        );

        Ok(row.get("id"))
    }

    async fn select_backend(
        &self,
        candidate: RuntimeBackend,
        requirements: &[RuntimeCapability],
        notes: &mut Vec<String>,
    ) -> (RuntimeBackend, bool, Option<String>) {
        let executors = self.executors.read().await;
        let candidate_descriptor = executors.get(&candidate).cloned();

        if let Some(ref descriptor) = candidate_descriptor {
            if descriptor.supports_all(requirements) {
                if !requirements.is_empty() {
                    let supported = descriptor.capability_keys().join(",");
                    notes.push(format!(
                        "executor:{}:capabilities-satisfied:{supported}",
                        descriptor.backend.as_str()
                    ));
                }
                return (candidate, true, Some(descriptor.display_name.clone()));
            }
        } else {
            notes.push(format!("executor:unavailable:{}", candidate.as_str()));
        }

        let alternative = executors
            .values()
            .find(|descriptor| {
                descriptor.backend != candidate && descriptor.supports_all(requirements)
            })
            .cloned();

        if let Some(descriptor) = alternative {
            let reqs = requirements
                .iter()
                .map(RuntimeCapability::as_str)
                .collect::<Vec<_>>()
                .join(",");
            notes.push(format!(
                "capabilities:routed:{}->{}:{reqs}",
                candidate.as_str(),
                descriptor.backend.as_str()
            ));
            return (
                descriptor.backend,
                true,
                Some(descriptor.display_name.clone()),
            );
        }

        if requirements.is_empty() {
            return (
                candidate,
                true,
                candidate_descriptor
                    .as_ref()
                    .map(|descriptor| descriptor.display_name.clone()),
            );
        }

        let reqs = requirements
            .iter()
            .map(RuntimeCapability::as_str)
            .collect::<Vec<_>>()
            .join(",");
        notes.push(format!(
            "capabilities:unsatisfied:{}:{reqs}",
            candidate.as_str()
        ));

        (
            candidate,
            false,
            candidate_descriptor
                .as_ref()
                .map(|descriptor| descriptor.display_name.clone()),
        )
    }

    pub async fn resolve_backend_for(
        &self,
        pool: &PgPool,
        server_id: i32,
    ) -> Result<Option<RuntimeBackend>, PolicyError> {
        let row = sqlx::query(
            r#"
            SELECT backend
            FROM runtime_policy_decisions
            WHERE server_id = $1
            ORDER BY decided_at DESC
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = row {
            let backend_str: String = row.get("backend");
            Ok(RuntimeBackend::from_str(&backend_str).ok())
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VmAttestationRecord {
    pub instance_id: i64,
    pub status: String,
    pub updated_at: DateTime<Utc>,
    pub terminated_at: Option<DateTime<Utc>>,
    pub trust_event: Option<RuntimeVmTrustEvent>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct VmAttestationPolicyOutcome {
    pub notes: Vec<String>,
    pub backend_override: Option<RuntimeBackend>,
    pub evaluation_required: bool,
    pub attestation_status: Option<String>,
    pub stale: bool,
}

pub fn evaluate_vm_attestation_posture(
    record: Option<VmAttestationRecord>,
    now: DateTime<Utc>,
    stale_limit: Duration,
    fallback_backend: RuntimeBackend,
) -> VmAttestationPolicyOutcome {
    let mut outcome = VmAttestationPolicyOutcome::default();
    match record {
        Some(record) => {
            outcome.attestation_status = Some(record.status.clone());
            outcome
                .notes
                .push(format!("vm:last-status:{}", record.status));
            outcome.notes.push(format!(
                "vm:last-updated:{}",
                record.updated_at.to_rfc3339()
            ));
            if let Some(terminated) = record.terminated_at {
                outcome
                    .notes
                    .push(format!("vm:last-terminated:{}", terminated.to_rfc3339()));
            }
            if let Some(trust_event) = &record.trust_event {
                outcome.notes.push(format!(
                    "vm:trust-event:{}:{}",
                    trust_event.id, trust_event.current_status
                ));
                if let Some(reason) = &trust_event.transition_reason {
                    outcome.notes.push(format!("vm:trust-reason:{}", reason));
                }
                outcome.notes.push(format!(
                    "vm:trust-lifecycle:{}",
                    trust_event.current_lifecycle_state
                ));
                if let Some(previous) = &trust_event.previous_lifecycle_state {
                    outcome
                        .notes
                        .push(format!("vm:trust-previous-lifecycle:{}", previous));
                }
                if trust_event.remediation_attempts > 0 {
                    outcome.notes.push(format!(
                        "vm:trust-remediation-attempts:{}",
                        trust_event.remediation_attempts
                    ));
                }
                if let Some(deadline) = trust_event.freshness_deadline {
                    outcome.notes.push(format!(
                        "vm:trust-freshness-deadline:{}",
                        deadline.to_rfc3339()
                    ));
                }
                if let Some(provenance_ref) = &trust_event.provenance_ref {
                    outcome
                        .notes
                        .push(format!("vm:trust-provenance:{}", provenance_ref));
                }
            }

            match record.status.as_str() {
                "trusted" => outcome.notes.push("vm:attestation:trusted".to_string()),
                "untrusted" => {
                    outcome.notes.push("vm:attestation:untrusted".to_string());
                    outcome.notes.push("vm:attestation:blocked".to_string());
                    outcome.notes.push(format!(
                        "vm:attestation:fallback:{}",
                        fallback_backend.as_str()
                    ));
                    outcome.backend_override = Some(fallback_backend);
                    outcome.evaluation_required = true;
                }
                "pending" | "unknown" => {
                    let age = now - record.updated_at;
                    let age_seconds = age.num_seconds().max(0);
                    outcome
                        .notes
                        .push(format!("vm:attestation:pending:{}s", age_seconds));
                    outcome.evaluation_required = true;
                    if age > stale_limit {
                        outcome.stale = true;
                        outcome.notes.push("vm:attestation:stale".to_string());
                        outcome.notes.push(format!(
                            "vm:attestation:fallback:{}",
                            fallback_backend.as_str()
                        ));
                        outcome.backend_override = Some(fallback_backend);
                    }
                }
                other => outcome.notes.push(format!("vm:attestation:status:{other}")),
            }
        }
        None => {
            outcome.attestation_status = Some("none".to_string());
            outcome.notes.push("vm:attestation:none".to_string());
            outcome.notes.push(format!(
                "vm:attestation:fallback:{}",
                fallback_backend.as_str()
            ));
            outcome.backend_override = Some(fallback_backend);
            outcome.evaluation_required = true;
        }
    }
    outcome
}

fn default_image_for(server_type: &str) -> &str {
    match server_type {
        "PostgreSQL" => "ghcr.io/anycontext/postgres-mcp:latest",
        "Slack" => "ghcr.io/anycontext/slack-mcp:latest",
        "PDF Parser" => "ghcr.io/anycontext/pdf-mcp:latest",
        "Notion" => "ghcr.io/anycontext/notion-mcp:latest",
        "Router" => "ghcr.io/anycontext/router-mcp:latest",
        _ => "ghcr.io/anycontext/default-mcp:latest",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluations::{CertificationStatus, CertificationUpsert};
    use crate::governance::GovernanceEngine;
    use chrono::{Duration, Utc};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn policy_requires_certifications() -> Result<(), Box<dyn std::error::Error>> {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("skipping policy_requires_certifications: DATABASE_URL not set");
                return Ok(());
            }
        };

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
        sqlx::migrate!("../backend/migrations").run(&pool).await?;
        let user_id: i32 = sqlx::query_scalar(
            "INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id",
        )
        .bind("operator@example.com")
        .bind("hash")
        .fetch_one(&pool)
        .await?;

        let server_id: i32 = sqlx::query_scalar(
            "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(user_id)
        .bind("Router Server")
        .bind("Router")
        .bind(json!({}))
        .bind("ready")
        .bind("test-key")
        .fetch_one(&pool)
        .await?;

        let manifest_digest = "sha256:test-digest".to_string();
        let start = Utc::now() - Duration::minutes(5);
        let end = Utc::now();

        let run_id: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO build_artifact_runs (
                server_id,
                source_repo,
                source_branch,
                source_revision,
                registry,
                local_image,
                registry_image,
                manifest_tag,
                manifest_digest,
                started_at,
                completed_at,
                status,
                multi_arch,
                auth_refresh_attempted,
                auth_refresh_succeeded,
                auth_rotation_attempted,
                auth_rotation_succeeded,
                credential_health_status
            ) VALUES (
                $1, NULL, NULL, NULL, NULL, $2, $3, $4, $5, $6, $7, 'succeeded', TRUE,
                FALSE, FALSE, FALSE, FALSE, 'healthy'
            ) RETURNING id
            "#,
        )
        .bind(server_id)
        .bind("router/local:latest")
        .bind(Some("registry.test/router:latest"))
        .bind(Some("router:latest"))
        .bind(&manifest_digest)
        .bind(start)
        .bind(end)
        .fetch_one(&pool)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO build_artifact_platforms (
                run_id,
                platform,
                remote_image,
                remote_tag,
                digest,
                auth_refresh_attempted,
                auth_refresh_succeeded,
                auth_rotation_attempted,
                auth_rotation_succeeded,
                credential_health_status
            ) VALUES ($1, $2, $3, $4, $5, FALSE, FALSE, FALSE, FALSE, 'healthy')
            "#,
        )
        .bind(run_id)
        .bind("linux/amd64")
        .bind("registry.test/router:amd64")
        .bind("router-amd64")
        .bind(Some(manifest_digest.clone()))
        .execute(&pool)
        .await?;

        let engine = Arc::new(RuntimePolicyEngine::new(RuntimeBackend::Docker));
        let governance_engine = Arc::new(GovernanceEngine::new());
        engine
            .register_executor(RuntimeExecutorDescriptor::new(
                RuntimeBackend::Docker,
                "Docker",
                [],
            ))
            .await;
        engine.attach_governance(governance_engine.clone()).await;

        let decision_initial = engine
            .decide_and_record(&pool, server_id, "Router", None, false)
            .await?;
        assert!(decision_initial.evaluation_required);
        assert!(decision_initial
            .notes
            .iter()
            .any(|note| note.starts_with("evaluation:missing-certification")));

        let tier = decision_initial
            .tier
            .clone()
            .expect("tier should be derived");

        crate::evaluations::upsert_certification(
            &pool,
            CertificationUpsert {
                build_artifact_run_id: run_id,
                manifest_digest: manifest_digest.clone(),
                tier: tier.clone(),
                policy_requirement: "baseline".to_string(),
                status: CertificationStatus::Pass,
                evidence: None,
                evidence_source: None,
                evidence_lineage: None,
                valid_from: Utc::now() - Duration::minutes(1),
                valid_until: Some(Utc::now() + Duration::minutes(30)),
                refresh_cadence_seconds: Some(1_800),
                next_refresh_at: Some(Utc::now() + Duration::minutes(15)),
                governance_notes: None,
            },
        )
        .await?;

        let workflow_id: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO governance_workflows (owner_id, name, workflow_type, tier)
            VALUES ($1, $2, 'promotion', $3)
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind("Default promotion")
        .bind(&tier)
        .fetch_one(&pool)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO governance_workflow_runs (
                workflow_id,
                initiated_by,
                target_artifact_run_id,
                target_manifest_digest,
                target_tier,
                status,
                notes
            ) VALUES ($1, $2, $3, $4, $5, 'completed', ARRAY['test'])
            "#,
        )
        .bind(workflow_id)
        .bind(user_id)
        .bind(run_id)
        .bind(&manifest_digest)
        .bind(&tier)
        .execute(&pool)
        .await?;

        let decision_with_cert = engine
            .decide_and_record(&pool, server_id, "Router", None, false)
            .await?;
        assert!(!decision_with_cert.evaluation_required);
        assert!(!decision_with_cert.governance_required);
        assert!(decision_with_cert
            .notes
            .iter()
            .any(|note| note.starts_with("evaluation:certified")));

        sqlx::query("DELETE FROM evaluation_certifications WHERE build_artifact_run_id = $1")
            .bind(run_id)
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM build_artifact_platforms WHERE run_id = $1")
            .bind(run_id)
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM build_artifact_runs WHERE id = $1")
            .bind(run_id)
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
            .bind(server_id)
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await?;

        Ok(())
    }
}
