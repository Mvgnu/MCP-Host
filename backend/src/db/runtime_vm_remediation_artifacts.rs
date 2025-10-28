use serde_json::Value;
use sqlx::{Executor, Postgres};

// key: remediation-db -> artifact-ledger
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
