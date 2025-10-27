use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{postgres::PgRow, PgPool, Row};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeVmTrustEvent {
    pub id: i64,
    pub runtime_vm_instance_id: i64,
    pub attestation_id: Option<i64>,
    pub previous_status: Option<String>,
    pub current_status: String,
    pub transition_reason: Option<String>,
    pub remediation_state: Option<String>,
    pub triggered_at: DateTime<Utc>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewRuntimeVmTrustEvent<'a> {
    pub runtime_vm_instance_id: i64,
    pub attestation_id: Option<i64>,
    pub previous_status: Option<&'a str>,
    pub current_status: &'a str,
    pub transition_reason: Option<&'a str>,
    pub remediation_state: Option<&'a str>,
    pub metadata: Option<&'a Value>,
}

pub async fn insert_trust_event(
    pool: &PgPool,
    input: NewRuntimeVmTrustEvent<'_>,
) -> Result<RuntimeVmTrustEvent, sqlx::Error> {
    let row = sqlx::query(
        r#"
        INSERT INTO runtime_vm_trust_history (
            runtime_vm_instance_id,
            attestation_id,
            previous_status,
            current_status,
            transition_reason,
            remediation_state,
            metadata
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING
            id,
            runtime_vm_instance_id,
            attestation_id,
            previous_status,
            current_status,
            transition_reason,
            remediation_state,
            triggered_at,
            metadata,
            created_at
        "#,
    )
    .bind(input.runtime_vm_instance_id)
    .bind(input.attestation_id)
    .bind(input.previous_status)
    .bind(input.current_status)
    .bind(input.transition_reason)
    .bind(input.remediation_state)
    .bind(input.metadata)
    .fetch_one(pool)
    .await?;

    Ok(map_row(&row))
}

pub async fn latest_for_instance(
    pool: &PgPool,
    runtime_vm_instance_id: i64,
) -> Result<Option<RuntimeVmTrustEvent>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            id,
            runtime_vm_instance_id,
            attestation_id,
            previous_status,
            current_status,
            transition_reason,
            remediation_state,
            triggered_at,
            metadata,
            created_at
        FROM runtime_vm_trust_history
        WHERE runtime_vm_instance_id = $1
        ORDER BY triggered_at DESC
        LIMIT 1
        "#,
    )
    .bind(runtime_vm_instance_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| map_row(&row)))
}

pub async fn history_for_instance(
    pool: &PgPool,
    runtime_vm_instance_id: i64,
    limit: i64,
) -> Result<Vec<RuntimeVmTrustEvent>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            runtime_vm_instance_id,
            attestation_id,
            previous_status,
            current_status,
            transition_reason,
            remediation_state,
            triggered_at,
            metadata,
            created_at
        FROM runtime_vm_trust_history
        WHERE runtime_vm_instance_id = $1
        ORDER BY triggered_at DESC
        LIMIT $2
        "#,
    )
    .bind(runtime_vm_instance_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(map_row).collect())
}

fn map_row(row: &PgRow) -> RuntimeVmTrustEvent {
    RuntimeVmTrustEvent {
        id: row.get("id"),
        runtime_vm_instance_id: row.get("runtime_vm_instance_id"),
        attestation_id: row.try_get("attestation_id").ok().flatten(),
        previous_status: row.try_get("previous_status").ok().flatten(),
        current_status: row.get("current_status"),
        transition_reason: row.try_get("transition_reason").ok().flatten(),
        remediation_state: row.try_get("remediation_state").ok().flatten(),
        triggered_at: row.get("triggered_at"),
        metadata: row.try_get("metadata").ok().flatten(),
        created_at: row.get("created_at"),
    }
}
