use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::evaluations::{self, CertificationStatus};
use crate::governance::GovernanceEngine;
use crate::intelligence::{self, IntelligenceError, IntelligenceStatus, RecomputeContext};
use crate::job_queue;
use crate::marketplace::{classify_tier, derive_health, MarketplacePlatform};

// key: runtime-policy -> placement-decisions,marketplace-health

const POLICY_VERSION: &str = "runtime-policy-v0.1";

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
        let decision = self
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
        Ok(decision)
    }

    async fn evaluate(
        &self,
        pool: &PgPool,
        server_id: i32,
        server_type: &str,
        config: Option<&Value>,
        use_gpu: bool,
    ) -> Result<PolicyDecision, PolicyError> {
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

        let mut evaluation_required = false;

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
                        match certification.status {
                            CertificationStatus::Pass if active => {
                                notes.push(format!(
                                    "evaluation:certified:{}:{}",
                                    tier_value, requirement
                                ));
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

        Ok(PolicyDecision {
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
        })
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
                decided_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21
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
                valid_from: Utc::now() - Duration::minutes(1),
                valid_until: Some(Utc::now() + Duration::minutes(30)),
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
