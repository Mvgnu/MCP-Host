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
