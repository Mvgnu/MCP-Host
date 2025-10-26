use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sqlx::{PgPool, Row};
use thiserror::Error;

use crate::marketplace::{classify_tier, derive_health, MarketplacePlatform};

// key: runtime-policy -> placement-decisions,marketplace-health

const POLICY_VERSION: &str = "runtime-policy-v0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeBackend {
    Docker,
    Kubernetes,
}

impl RuntimeBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeBackend::Docker => "docker",
            RuntimeBackend::Kubernetes => "kubernetes",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyDecision {
    pub backend: RuntimeBackend,
    pub image: String,
    pub requires_build: bool,
    pub artifact_run_id: Option<i32>,
    pub manifest_digest: Option<String>,
    pub policy_version: String,
    pub evaluation_required: bool,
    pub tier: Option<String>,
    pub health_overall: Option<String>,
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
}

impl RuntimePolicyEngine {
    pub fn new(default_backend: RuntimeBackend) -> Self {
        Self { default_backend }
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

        if use_gpu && !matches!(backend, RuntimeBackend::Kubernetes) {
            backend = RuntimeBackend::Kubernetes;
            notes.push("gpu:requested -> backend:kubernetes".to_string());
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

        let evaluation_required = requires_build
            || health_overall
                .as_ref()
                .map(|status| status != "healthy")
                .unwrap_or(true);

        Ok(PolicyDecision {
            backend,
            image,
            requires_build,
            artifact_run_id,
            manifest_digest,
            policy_version: POLICY_VERSION.to_string(),
            evaluation_required,
            tier,
            health_overall,
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

        sqlx::query(
            r#"
            INSERT INTO runtime_policy_decisions (
                server_id,
                backend,
                image,
                requires_build,
                artifact_run_id,
                manifest_digest,
                policy_version,
                evaluation_required,
                tier,
                health_overall,
                notes,
                decided_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12
            )
            "#,
        )
        .bind(server_id)
        .bind(decision.backend.as_str())
        .bind(&decision.image)
        .bind(decision.requires_build)
        .bind(decision.artifact_run_id)
        .bind(&decision.manifest_digest)
        .bind(&decision.policy_version)
        .bind(decision.evaluation_required)
        .bind(decision.tier.as_deref())
        .bind(decision.health_overall.as_deref())
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
