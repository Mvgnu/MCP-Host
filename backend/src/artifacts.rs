use chrono::{DateTime, Utc};
use sqlx::PgPool;

// key: artifact-persistence -> build_artifact_runs,build_artifact_platforms
#[derive(Debug, Clone)]
pub struct ArtifactPlatformRecord {
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

#[derive(Debug, Clone)]
pub struct ArtifactPersistenceRequest {
    pub server_id: i32,
    pub source_repo: Option<String>,
    pub source_branch: Option<String>,
    pub source_revision: Option<String>,
    pub registry: Option<String>,
    pub local_image: String,
    pub registry_image: Option<String>,
    pub manifest_tag: String,
    pub manifest_digest: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub status: String,
    pub multi_arch: bool,
    pub auth_refresh_attempted: bool,
    pub auth_refresh_succeeded: bool,
    pub auth_rotation_attempted: bool,
    pub auth_rotation_succeeded: bool,
    pub credential_health_status: String,
    pub platforms: Vec<ArtifactPlatformRecord>,
}

pub async fn record_build_artifacts(
    pool: &PgPool,
    request: ArtifactPersistenceRequest,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
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
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18
        )
        RETURNING id
        "#,
    )
    .bind(request.server_id)
    .bind(request.source_repo.as_deref())
    .bind(request.source_branch.as_deref())
    .bind(request.source_revision.as_deref())
    .bind(request.registry.as_deref())
    .bind(&request.local_image)
    .bind(request.registry_image.as_deref())
    .bind(&request.manifest_tag)
    .bind(request.manifest_digest.as_deref())
    .bind(request.started_at)
    .bind(request.completed_at)
    .bind(&request.status)
    .bind(request.multi_arch)
    .bind(request.auth_refresh_attempted)
    .bind(request.auth_refresh_succeeded)
    .bind(request.auth_rotation_attempted)
    .bind(request.auth_rotation_succeeded)
    .bind(&request.credential_health_status)
    .fetch_one(&mut *tx)
    .await?;

    for platform in request.platforms {
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
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
            )
            "#,
        )
        .bind(run_id)
        .bind(&platform.platform)
        .bind(&platform.remote_image)
        .bind(&platform.remote_tag)
        .bind(platform.digest.as_deref())
        .bind(platform.auth_refresh_attempted)
        .bind(platform.auth_refresh_succeeded)
        .bind(platform.auth_rotation_attempted)
        .bind(platform.auth_rotation_succeeded)
        .bind(&platform.credential_health_status)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await
}
