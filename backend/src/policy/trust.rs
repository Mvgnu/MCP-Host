use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::db::runtime_vm_attestations::{
    insert_attestation, NewRuntimeVmAttestation, RuntimeVmAttestationRecord,
};
use crate::runtime::vm::attestation::{AttestationOutcome, AttestationStatus};

#[derive(Debug, Clone, Serialize)]
pub struct TrustTransition {
    pub vm_instance_id: i64,
    pub server_id: i32,
    pub previous_status: Option<String>,
    pub current_status: String,
    pub posture_changed: bool,
    pub freshness_expires_at: Option<DateTime<Utc>>,
    pub attestation: RuntimeVmAttestationRecord,
}

impl TrustTransition {
    pub fn should_invalidate_cache(&self) -> bool {
        self.posture_changed
    }

    pub fn broadcast_payload(&self) -> Value {
        serde_json::json!({
            "vm_instance_id": self.vm_instance_id,
            "server_id": self.server_id,
            "status": self.current_status,
            "previous_status": self.previous_status,
            "freshness_expires_at": self.freshness_expires_at,
            "attestation_id": self.attestation.id,
        })
    }
}

pub async fn persist_vm_attestation_outcome(
    pool: &PgPool,
    server_id: i32,
    vm_instance_id: i64,
    outcome: &AttestationOutcome,
    remediation_notes: &[String],
) -> Result<TrustTransition, sqlx::Error> {
    let previous_status: Option<String> =
        sqlx::query_scalar("SELECT attestation_status FROM runtime_vm_instances WHERE id = $1")
            .bind(vm_instance_id)
            .fetch_optional(pool)
            .await?
            .flatten();

    let verification_notes = outcome.notes.clone();
    let raw_quote_bytes = outcome
        .evidence
        .as_ref()
        .and_then(|value| value.get("raw"))
        .and_then(|value| value.as_str())
        .and_then(|encoded| Base64Engine.decode(encoded).ok());
    let attestation = insert_attestation(
        pool,
        NewRuntimeVmAttestation {
            runtime_vm_instance_id: vm_instance_id,
            attestation_kind: outcome.attestation_kind.as_str(),
            verification_status: outcome.status.as_str(),
            raw_quote: raw_quote_bytes.as_deref(),
            parsed_claims: outcome.evidence.as_ref(),
            signer_metadata: None,
            freshness_expires_at: outcome.freshness_deadline,
            verification_notes: &verification_notes,
            remediation_notes,
        },
    )
    .await?;

    sqlx::query(
        "UPDATE runtime_vm_instances SET attestation_status = $1, attestation_evidence = COALESCE($2, attestation_evidence) WHERE id = $3",
    )
    .bind(outcome.status.as_str())
    .bind(outcome.evidence.clone())
    .bind(vm_instance_id)
    .execute(pool)
    .await?;

    let posture_changed = previous_status.as_deref() != Some(outcome.status.as_str());
    Ok(TrustTransition {
        vm_instance_id,
        server_id,
        previous_status,
        current_status: outcome.status.as_str().to_string(),
        posture_changed,
        freshness_expires_at: outcome.freshness_deadline,
        attestation,
    })
}

pub fn remediation_notes_for_status(status: AttestationStatus) -> Vec<String> {
    match status {
        AttestationStatus::Trusted => vec!["remediation:none".to_string()],
        AttestationStatus::Unknown => vec!["remediation:monitor".to_string()],
        AttestationStatus::Untrusted => vec!["remediation:investigate".to_string()],
    }
}
