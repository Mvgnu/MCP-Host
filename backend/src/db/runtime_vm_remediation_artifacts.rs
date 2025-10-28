use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres};

// key: remediation-db -> artifact-ledger
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationArtifact {
    pub id: i64,
    pub remediation_run_id: i64,
    pub artifact_type: String,
    pub uri: Option<String>,
    pub metadata: Value,
    pub recorded_by: Option<i32>,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_artifact<'c, E>(
    executor: E,
    remediation_run_id: i64,
    artifact_type: &str,
    uri: Option<&str>,
    metadata: &Value,
    recorded_by: Option<i32>,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_scalar(
        r#"
        INSERT INTO runtime_vm_remediation_artifacts (
            remediation_run_id,
            artifact_type,
            uri,
            metadata,
            recorded_by
        )
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(remediation_run_id)
    .bind(artifact_type)
    .bind(uri)
    .bind(metadata)
    .bind(recorded_by)
    .fetch_one(executor)
    .await?;

    Ok(record)
}

pub async fn list_artifacts(
    pool: &PgPool,
    remediation_run_id: i64,
) -> Result<Vec<RuntimeVmRemediationArtifact>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmRemediationArtifact>(
        r#"
        SELECT
            id,
            remediation_run_id,
            artifact_type,
            uri,
            metadata,
            recorded_by,
            created_at
        FROM runtime_vm_remediation_artifacts
        WHERE remediation_run_id = $1
        ORDER BY created_at
        "#,
    )
    .bind(remediation_run_id)
    .fetch_all(pool)
    .await
}
