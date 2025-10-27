use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{postgres::PgRow, Executor, PgPool, Postgres, Row};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeVmTrustRegistryState {
    pub runtime_vm_instance_id: i64,
    pub attestation_status: String,
    pub lifecycle_state: String,
    pub remediation_state: Option<String>,
    pub remediation_attempts: i32,
    pub freshness_deadline: Option<DateTime<Utc>>,
    pub provenance_ref: Option<String>,
    pub provenance: Option<Value>,
    pub version: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertRuntimeVmTrustRegistryState<'a> {
    pub runtime_vm_instance_id: i64,
    pub attestation_status: &'a str,
    pub lifecycle_state: &'a str,
    pub remediation_state: Option<&'a str>,
    pub remediation_attempts: i32,
    pub freshness_deadline: Option<DateTime<Utc>>,
    pub provenance_ref: Option<&'a str>,
    pub provenance: Option<&'a Value>,
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ApplyRuntimeVmTrustTransition<'a> {
    pub runtime_vm_instance_id: i64,
    pub attestation_status: &'a str,
    pub lifecycle_state: &'a str,
    pub remediation_state: Option<&'a str>,
    pub remediation_attempts: i32,
    pub freshness_deadline: Option<DateTime<Utc>>,
    pub provenance_ref: Option<&'a str>,
    pub provenance: Option<&'a Value>,
    pub expected_version: Option<i64>,
    pub previous_status: Option<&'a str>,
    pub previous_lifecycle_state: Option<&'a str>,
    pub transition_reason: &'a str,
    pub metadata: Option<&'a Value>,
}

pub async fn get_state<'c, E>(
    executor: E,
    runtime_vm_instance_id: i64,
) -> Result<Option<RuntimeVmTrustRegistryState>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let row = sqlx::query(
        r#"
        SELECT
            runtime_vm_instance_id,
            attestation_status,
            lifecycle_state,
            remediation_state,
            remediation_attempts,
            freshness_deadline,
            provenance_ref,
            provenance,
            version,
            updated_at
        FROM runtime_vm_trust_registry
        WHERE runtime_vm_instance_id = $1
        "#,
    )
    .bind(runtime_vm_instance_id)
    .fetch_optional(executor)
    .await?;

    Ok(row.map(|row| map_row(&row)))
}

pub async fn upsert_state<'c, E>(
    executor: E,
    input: UpsertRuntimeVmTrustRegistryState<'_>,
) -> Result<RuntimeVmTrustRegistryState, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let expected_version = input.expected_version.unwrap_or(-1);
    let row = sqlx::query(
        r#"
        WITH upsert AS (
            INSERT INTO runtime_vm_trust_registry (
                runtime_vm_instance_id,
                attestation_status,
                lifecycle_state,
                remediation_state,
                remediation_attempts,
                freshness_deadline,
                provenance_ref,
                provenance,
                version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0)
            ON CONFLICT (runtime_vm_instance_id) DO UPDATE
            SET
                attestation_status = EXCLUDED.attestation_status,
                lifecycle_state = EXCLUDED.lifecycle_state,
                remediation_state = EXCLUDED.remediation_state,
                remediation_attempts = EXCLUDED.remediation_attempts,
                freshness_deadline = EXCLUDED.freshness_deadline,
                provenance_ref = EXCLUDED.provenance_ref,
                provenance = EXCLUDED.provenance,
                version = runtime_vm_trust_registry.version + 1,
                updated_at = NOW()
            WHERE runtime_vm_trust_registry.version = $9
            RETURNING *
        )
        SELECT * FROM upsert
        "#,
    )
    .bind(input.runtime_vm_instance_id)
    .bind(input.attestation_status)
    .bind(input.lifecycle_state)
    .bind(input.remediation_state)
    .bind(input.remediation_attempts)
    .bind(input.freshness_deadline)
    .bind(input.provenance_ref)
    .bind(input.provenance)
    .bind(expected_version)
    .fetch_optional(executor)
    .await?;

    match row {
        Some(row) => Ok(map_row(&row)),
        None => Err(sqlx::Error::RowNotFound),
    }
}

pub async fn apply_transition(
    pool: &PgPool,
    input: ApplyRuntimeVmTrustTransition<'_>,
) -> Result<RuntimeVmTrustRegistryState, sqlx::Error> {
    let expected_version = input.expected_version.unwrap_or(-1);
    let row = sqlx::query(
        r#"
        WITH updated AS (
            INSERT INTO runtime_vm_trust_registry (
                runtime_vm_instance_id,
                attestation_status,
                lifecycle_state,
                remediation_state,
                remediation_attempts,
                freshness_deadline,
                provenance_ref,
                provenance,
                version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0)
            ON CONFLICT (runtime_vm_instance_id) DO UPDATE
            SET
                attestation_status = EXCLUDED.attestation_status,
                lifecycle_state = EXCLUDED.lifecycle_state,
                remediation_state = EXCLUDED.remediation_state,
                remediation_attempts = EXCLUDED.remediation_attempts,
                freshness_deadline = EXCLUDED.freshness_deadline,
                provenance_ref = EXCLUDED.provenance_ref,
                provenance = EXCLUDED.provenance,
                version = runtime_vm_trust_registry.version + 1,
                updated_at = NOW()
            WHERE runtime_vm_trust_registry.version = $9
            RETURNING
                runtime_vm_instance_id,
                attestation_status,
                lifecycle_state,
                remediation_state,
                remediation_attempts,
                freshness_deadline,
                provenance_ref,
                provenance,
                version,
                updated_at
        ),
        history AS (
            INSERT INTO runtime_vm_trust_history (
                runtime_vm_instance_id,
                attestation_id,
                previous_status,
                current_status,
                previous_lifecycle_state,
                current_lifecycle_state,
                transition_reason,
                remediation_state,
                remediation_attempts,
                freshness_deadline,
                provenance_ref,
                provenance,
                metadata
            )
            SELECT
                $1,
                NULL,
                $10,
                updated.attestation_status,
                $11,
                updated.lifecycle_state,
                $12,
                updated.remediation_state,
                $5,
                updated.freshness_deadline,
                updated.provenance_ref,
                updated.provenance,
                $13
            FROM updated
            RETURNING 1
        )
        SELECT * FROM updated
        "#,
    )
    .bind(input.runtime_vm_instance_id)
    .bind(input.attestation_status)
    .bind(input.lifecycle_state)
    .bind(input.remediation_state)
    .bind(input.remediation_attempts)
    .bind(input.freshness_deadline)
    .bind(input.provenance_ref)
    .bind(input.provenance)
    .bind(expected_version)
    .bind(input.previous_status)
    .bind(input.previous_lifecycle_state)
    .bind(input.transition_reason)
    .bind(input.metadata)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => Ok(map_row(&row)),
        None => Err(sqlx::Error::RowNotFound),
    }
}

fn map_row(row: &PgRow) -> RuntimeVmTrustRegistryState {
    RuntimeVmTrustRegistryState {
        runtime_vm_instance_id: row.get("runtime_vm_instance_id"),
        attestation_status: row.get("attestation_status"),
        lifecycle_state: row.get("lifecycle_state"),
        remediation_state: row.try_get("remediation_state").ok().flatten(),
        remediation_attempts: row.get("remediation_attempts"),
        freshness_deadline: row.try_get("freshness_deadline").ok().flatten(),
        provenance_ref: row.try_get("provenance_ref").ok().flatten(),
        provenance: row.try_get("provenance").ok().flatten(),
        version: row.get("version"),
        updated_at: row.get("updated_at"),
    }
}
