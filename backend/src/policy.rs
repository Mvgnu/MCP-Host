use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sqlx::{PgPool, Row};
use thiserror::Error;
use tokio::sync::RwLock;

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
    pub tier: Option<String>,
    pub health_overall: Option<String>,
    pub capability_requirements: Vec<RuntimeCapability>,
    pub capabilities_satisfied: bool,
    pub executor_name: Option<String>,
    pub notes: Vec<String>,
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
}

impl RuntimePolicyEngine {
    pub fn new(default_backend: RuntimeBackend) -> Self {
        Self {
            default_backend,
            executors: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn default_backend(&self) -> RuntimeBackend {
        self.default_backend
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
        self.record_decision(pool, server_id, &decision).await?;
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

        let evaluation_required = requires_build
            || health_overall
                .as_ref()
                .map(|status| status != "healthy")
                .unwrap_or(true);
        let evaluation_required = evaluation_required || !capabilities_satisfied;

        Ok(PolicyDecision {
            backend,
            candidate_backend,
            image,
            requires_build,
            artifact_run_id,
            manifest_digest,
            policy_version: POLICY_VERSION.to_string(),
            evaluation_required,
            tier,
            health_overall,
            capability_requirements,
            capabilities_satisfied,
            executor_name,
            notes,
        })
    }

    async fn record_decision(
        &self,
        pool: &PgPool,
        server_id: i32,
        decision: &PolicyDecision,
    ) -> Result<(), PolicyError> {
        let notes_json =
            serde_json::Value::Array(decision.notes.iter().cloned().map(Value::String).collect());
        let capability_json = serde_json::Value::Array(
            decision
                .capability_requirements
                .iter()
                .map(|cap| Value::String(cap.as_str().to_string()))
                .collect(),
        );

        sqlx::query(
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
                tier,
                health_overall,
                capability_requirements,
                capabilities_satisfied,
                executor_name,
                notes,
                decided_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
            )
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
        .bind(decision.tier.as_deref())
        .bind(decision.health_overall.as_deref())
        .bind(capability_json)
        .bind(decision.capabilities_satisfied)
        .bind(decision.executor_name.as_deref())
        .bind(notes_json)
        .bind(Utc::now())
        .execute(pool)
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
            policy_version = %decision.policy_version,
            "recorded runtime policy decision"
        );

        Ok(())
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
