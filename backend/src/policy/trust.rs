use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::db::runtime_vm_attestations::{
    insert_attestation, NewRuntimeVmAttestation, RuntimeVmAttestationRecord,
};
use crate::db::runtime_vm_trust_history::{
    insert_trust_event, NewRuntimeVmTrustEvent, RuntimeVmTrustEvent,
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
    pub trust_event: RuntimeVmTrustEvent,
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
            "trust_event": {
                "id": self.trust_event.id,
                "triggered_at": self.trust_event.triggered_at,
                "transition_reason": self.trust_event.transition_reason,
                "remediation_state": self.trust_event.remediation_state,
            }
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
    let metadata = outcome.evidence.as_ref().map(|value| {
        serde_json::json!({
            "attestation_kind": outcome.attestation_kind.as_str(),
            "claims": value,
        })
    });

    let trust_event = insert_trust_event(
        pool,
        NewRuntimeVmTrustEvent {
            runtime_vm_instance_id: vm_instance_id,
            attestation_id: Some(attestation.id),
            previous_status: previous_status.as_deref(),
            current_status: outcome.status.as_str(),
            transition_reason: Some("attestation"),
            remediation_state: remediation_notes.first().map(String::as_str),
            metadata: metadata.as_ref(),
        },
    )
    .await?;

    let posture_note = format!(
        "{} trust:{}:{}",
        Utc::now().to_rfc3339(),
        outcome.attestation_kind.as_str(),
        outcome.status.as_str()
    );

    sqlx::query(
        r#"
        UPDATE evaluation_certifications
        SET
            last_attestation_status = $1,
            fallback_launched_at = CASE
                WHEN $1 = 'untrusted' THEN COALESCE(fallback_launched_at, NOW())
                WHEN $1 = 'trusted' THEN NULL
                ELSE fallback_launched_at
            END,
            remediation_attempts = CASE
                WHEN $1 = 'untrusted' THEN remediation_attempts + 1
                ELSE remediation_attempts
            END,
            governance_notes = CASE
                WHEN governance_notes IS NULL OR governance_notes = '' THEN $3
                ELSE governance_notes || E'\n' || $3
            END,
            updated_at = NOW()
        WHERE build_artifact_run_id IN (
            SELECT id FROM build_artifact_runs WHERE server_id = $2
        )
        "#,
    )
    .bind(outcome.status.as_str())
    .bind(server_id)
    .bind(&posture_note)
    .execute(pool)
    .await?;

    Ok(TrustTransition {
        vm_instance_id,
        server_id,
        previous_status,
        current_status: outcome.status.as_str().to_string(),
        posture_changed,
        freshness_expires_at: outcome.freshness_deadline,
        attestation,
        trust_event,
    })
}

pub fn remediation_notes_for_status(status: AttestationStatus) -> Vec<String> {
    match status {
        AttestationStatus::Trusted => vec!["remediation:none".to_string()],
        AttestationStatus::Unknown => vec!["remediation:monitor".to_string()],
        AttestationStatus::Untrusted => vec!["remediation:investigate".to_string()],
    }
}
