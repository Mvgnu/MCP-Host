use axum::{
    extract::{Extension, Query},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::error::{AppError, AppResult};

// key: marketplace-catalog -> artifact-ledger,policy-tiering

#[derive(Debug, Deserialize, Default)]
pub struct MarketplaceQuery {
    pub server_type: Option<String>,
    pub status: Option<String>,
    pub tier: Option<String>,
    pub q: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketplacePlatform {
    pub platform: String,
    pub remote_image: String,
    pub remote_tag: String,
    pub digest: Option<String>,
    pub auth_refresh_attempted: bool,
    pub auth_refresh_succeeded: bool,
    pub auth_rotation_attempted: bool,
    pub auth_rotation_succeeded: bool,
    pub credential_health_status: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ArtifactHealth {
    pub overall: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MarketplaceArtifact {
    pub server_id: i32,
    pub server_name: String,
    pub server_type: String,
    pub manifest_tag: Option<String>,
    pub manifest_digest: Option<String>,
    pub registry_image: Option<String>,
    pub local_image: String,
    pub status: String,
    pub last_built_at: DateTime<Utc>,
    pub source_repo: Option<String>,
    pub source_branch: Option<String>,
    pub source_revision: Option<String>,
    pub multi_arch: bool,
    pub credential_health_status: String,
    pub tier: String,
    pub health: ArtifactHealth,
    pub platforms: Vec<MarketplacePlatform>,
}

pub async fn list_marketplace(
    Extension(pool): Extension<PgPool>,
    Query(params): Query<MarketplaceQuery>,
) -> AppResult<Json<Vec<MarketplaceArtifact>>> {
    let limit = params.limit.unwrap_or(50).min(200) as i64;
    let search_pattern = params
        .q
        .as_ref()
        .filter(|term| !term.trim().is_empty())
        .map(|term| format!("%{}%", term.trim()));

    let rows = sqlx::query(
        r#"
        SELECT
            runs.id,
            runs.server_id,
            runs.source_repo,
            runs.source_branch,
            runs.source_revision,
            runs.registry,
            runs.local_image,
            runs.registry_image,
            runs.manifest_tag,
            runs.manifest_digest,
            runs.status,
            runs.multi_arch,
            runs.completed_at,
            runs.credential_health_status,
            runs.auth_refresh_attempted,
            runs.auth_refresh_succeeded,
            runs.auth_rotation_attempted,
            runs.auth_rotation_succeeded,
            servers.name AS server_name,
            servers.server_type,
            COALESCE(
                json_agg(
                    json_build_object(
                        'platform', platforms.platform,
                        'remote_image', platforms.remote_image,
                        'remote_tag', platforms.remote_tag,
                        'digest', platforms.digest,
                        'auth_refresh_attempted', platforms.auth_refresh_attempted,
                        'auth_refresh_succeeded', platforms.auth_refresh_succeeded,
                        'auth_rotation_attempted', platforms.auth_rotation_attempted,
                        'auth_rotation_succeeded', platforms.auth_rotation_succeeded,
                        'credential_health_status', platforms.credential_health_status
                    )
                    ORDER BY platforms.platform
                ) FILTER (WHERE platforms.id IS NOT NULL),
                '[]'::json
            ) AS platforms
        FROM build_artifact_runs runs
        JOIN mcp_servers servers ON servers.id = runs.server_id
        LEFT JOIN build_artifact_platforms platforms ON platforms.run_id = runs.id
        WHERE ($2::text IS NULL OR servers.server_type = $2)
          AND ($3::text IS NULL OR runs.status = $3)
          AND (
                $4::text IS NULL
                OR servers.name ILIKE $4
                OR runs.manifest_tag ILIKE $4
                OR runs.manifest_digest ILIKE $4
                OR runs.registry_image ILIKE $4
                OR runs.local_image ILIKE $4
                OR runs.source_repo ILIKE $4
            )
        GROUP BY runs.id, servers.id
        ORDER BY runs.completed_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .bind(params.server_type.as_deref())
    .bind(params.status.as_deref())
    .bind(search_pattern.as_deref())
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    let mut artifacts = Vec::with_capacity(rows.len());

    for row in rows {
        let raw_platforms: serde_json::Value = row.get("platforms");
        let mut platforms: Vec<MarketplacePlatform> = serde_json::from_value(raw_platforms)
            .map_err(|error| {
                AppError::Message(format!("failed to deserialize platform slices: {error}"))
            })?;
        platforms.sort_by(|a, b| a.platform.cmp(&b.platform));

        let status: String = row.get("status");
        let credential_health_status: String = row.get("credential_health_status");
        let health = derive_health(&status, &credential_health_status, &platforms);

        let tier = classify_tier(row.get("server_type"), row.get("multi_arch"), &health);

        let artifact = MarketplaceArtifact {
            server_id: row.get("server_id"),
            server_name: row.get("server_name"),
            server_type: row.get("server_type"),
            manifest_tag: row.get("manifest_tag"),
            manifest_digest: row.get("manifest_digest"),
            registry_image: row.get("registry_image"),
            local_image: row.get("local_image"),
            status: status.clone(),
            last_built_at: row.get("completed_at"),
            source_repo: row.get("source_repo"),
            source_branch: row.get("source_branch"),
            source_revision: row.get("source_revision"),
            multi_arch: row.get("multi_arch"),
            credential_health_status,
            tier: tier.clone(),
            health,
            platforms,
        };

        let tier_filter_allows = params
            .tier
            .as_ref()
            .map(|expected| expected.eq_ignore_ascii_case(&tier))
            .unwrap_or(true);

        if tier_filter_allows {
            artifacts.push(artifact);
        }
    }

    Ok(Json(artifacts))
}

fn derive_health(
    status: &str,
    run_health: &str,
    platforms: &[MarketplacePlatform],
) -> ArtifactHealth {
    let mut issues = Vec::new();
    if !matches_success(status) {
        issues.push(format!("build_status:{status}"));
    }
    if !matches_healthy(run_health) {
        issues.push(format!("credential:{run_health}"));
    }
    for platform in platforms {
        if !matches_healthy(&platform.credential_health_status) {
            issues.push(format!(
                "platform:{}:{}",
                platform.platform, platform.credential_health_status
            ));
        }
    }

    let overall = if issues.is_empty() {
        "healthy"
    } else if issues
        .iter()
        .any(|issue| issue.starts_with("build_status:"))
    {
        "blocked"
    } else {
        "degraded"
    };

    ArtifactHealth {
        overall: overall.into(),
        issues,
    }
}

fn classify_tier(server_type: String, multi_arch: bool, health: &ArtifactHealth) -> String {
    let normalized_health = health.overall.as_str();
    if normalized_health == "healthy" && multi_arch {
        format!("gold:{}", server_type)
    } else if normalized_health == "healthy" {
        format!("silver:{}", server_type)
    } else if multi_arch {
        format!("watchlist-multi:{}", server_type)
    } else {
        format!("watchlist:{}", server_type)
    }
}

fn matches_success(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "succeeded" | "success" | "completed"
    )
}

fn matches_healthy(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "healthy" | "ok" | "success" | "succeeded" | "passing"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_health_for_successful_run() {
        let platforms = vec![MarketplacePlatform {
            platform: "linux/amd64".into(),
            remote_image: "example".into(),
            remote_tag: "latest".into(),
            digest: None,
            auth_refresh_attempted: false,
            auth_refresh_succeeded: true,
            auth_rotation_attempted: false,
            auth_rotation_succeeded: true,
            credential_health_status: "healthy".into(),
        }];

        let health = derive_health("succeeded", "healthy", &platforms);
        assert_eq!(health.overall, "healthy");
        assert!(health.issues.is_empty());
    }

    #[test]
    fn derives_health_with_platform_issue() {
        let platforms = vec![MarketplacePlatform {
            platform: "linux/arm64".into(),
            remote_image: "example".into(),
            remote_tag: "latest".into(),
            digest: None,
            auth_refresh_attempted: true,
            auth_refresh_succeeded: false,
            auth_rotation_attempted: false,
            auth_rotation_succeeded: true,
            credential_health_status: "error".into(),
        }];

        let health = derive_health("succeeded", "healthy", &platforms);
        assert_eq!(health.overall, "degraded");
        assert_eq!(health.issues.len(), 1);
    }

    #[test]
    fn classify_tier_promotes_multi_arch() {
        let health = ArtifactHealth {
            overall: "healthy".into(),
            issues: Vec::new(),
        };
        let tier = classify_tier("Router".into(), true, &health);
        assert!(tier.starts_with("gold:"));
    }
}
