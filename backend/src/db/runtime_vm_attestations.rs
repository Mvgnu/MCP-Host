use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeVmAttestationRecord {
    pub id: i64,
    pub runtime_vm_instance_id: i64,
    pub attestation_kind: String,
    pub verification_status: String,
    pub raw_quote: Option<Vec<u8>>,
    pub parsed_claims: Option<Value>,
    pub signer_metadata: Option<Value>,
    pub freshness_expires_at: Option<DateTime<Utc>>,
    pub verified_at: DateTime<Utc>,
    pub verification_notes: Vec<String>,
    pub remediation_notes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewRuntimeVmAttestation<'a> {
    pub runtime_vm_instance_id: i64,
    pub attestation_kind: &'a str,
    pub verification_status: &'a str,
    pub raw_quote: Option<&'a [u8]>,
    pub parsed_claims: Option<&'a Value>,
    pub signer_metadata: Option<&'a Value>,
    pub freshness_expires_at: Option<DateTime<Utc>>,
    pub verification_notes: &'a [String],
    pub remediation_notes: &'a [String],
}

pub async fn insert_attestation(
    pool: &PgPool,
    input: NewRuntimeVmAttestation<'_>,
) -> Result<RuntimeVmAttestationRecord, sqlx::Error> {
    let row = sqlx::query(
        r#"
        INSERT INTO runtime_vm_attestations (
            runtime_vm_instance_id,
            attestation_kind,
            verification_status,
            raw_quote,
            parsed_claims,
            signer_metadata,
            freshness_expires_at,
            verification_notes,
            remediation_notes
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING
            id,
            runtime_vm_instance_id,
            attestation_kind,
            verification_status,
            raw_quote,
            parsed_claims,
            signer_metadata,
            freshness_expires_at,
            verified_at,
            verification_notes,
            remediation_notes,
            created_at,
            updated_at
        "#,
    )
    .bind(input.runtime_vm_instance_id)
    .bind(input.attestation_kind)
    .bind(input.verification_status)
    .bind(input.raw_quote)
    .bind(input.parsed_claims)
    .bind(input.signer_metadata)
    .bind(input.freshness_expires_at)
    .bind(input.verification_notes)
    .bind(input.remediation_notes)
    .fetch_one(pool)
    .await?;

    Ok(map_row(&row))
}

pub async fn latest_for_instance(
    pool: &PgPool,
    runtime_vm_instance_id: i64,
) -> Result<Option<RuntimeVmAttestationRecord>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            id,
            runtime_vm_instance_id,
            attestation_kind,
            verification_status,
            raw_quote,
            parsed_claims,
            signer_metadata,
            freshness_expires_at,
            verified_at,
            verification_notes,
            remediation_notes,
            created_at,
            updated_at
        FROM runtime_vm_attestations
        WHERE runtime_vm_instance_id = $1
        ORDER BY verified_at DESC
        LIMIT 1
        "#,
    )
    .bind(runtime_vm_instance_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.as_ref().map(map_row))
}

fn map_row(row: &sqlx::postgres::PgRow) -> RuntimeVmAttestationRecord {
    RuntimeVmAttestationRecord {
        id: row.get("id"),
        runtime_vm_instance_id: row.get("runtime_vm_instance_id"),
        attestation_kind: row.get("attestation_kind"),
        verification_status: row.get("verification_status"),
        raw_quote: row.get("raw_quote"),
        parsed_claims: row.get("parsed_claims"),
        signer_metadata: row.get("signer_metadata"),
        freshness_expires_at: row.get("freshness_expires_at"),
        verified_at: row.get("verified_at"),
        verification_notes: row.get("verification_notes"),
        remediation_notes: row.get("remediation_notes"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
