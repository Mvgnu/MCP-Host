use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::db::runtime_vm_attestations::{
    insert_attestation, NewRuntimeVmAttestation, RuntimeVmAttestationRecord,
};
use crate::db::runtime_vm_remediation_runs::{
    get_active_run_for_instance, RuntimeVmRemediationRun,
};
use crate::db::runtime_vm_trust_history::{
    insert_trust_event, NewRuntimeVmTrustEvent, RuntimeVmTrustEvent,
};
use crate::db::runtime_vm_trust_registry::{
    get_state as get_registry_state, upsert_state as upsert_registry_state,
    UpsertRuntimeVmTrustRegistryState,
};
use crate::remediation::{RemediationFailureClassification, RemediationFailureReason};
use crate::runtime::vm::attestation::{AttestationOutcome, AttestationStatus};

#[derive(Debug, Clone, Serialize)]
pub struct TrustTransition {
    pub vm_instance_id: i64,
    pub server_id: i32,
    pub previous_status: Option<String>,
    pub current_status: String,
    pub previous_lifecycle_state: Option<String>,
    pub lifecycle_state: String,
    pub posture_changed: bool,
    pub freshness_expires_at: Option<DateTime<Utc>>,
    pub remediation_attempts: i32,
    pub provenance_ref: Option<String>,
    pub provenance: Option<Value>,
    pub attestation: RuntimeVmAttestationRecord,
    pub trust_event: RuntimeVmTrustEvent,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustPlacementGate {
    pub vm_instance_id: i64,
    pub attestation_status: Option<String>,
    pub lifecycle_state: Option<String>,
    pub remediation_state: Option<String>,
    pub remediation_attempts: i32,
    pub freshness_deadline: Option<DateTime<Utc>>,
    pub provenance_ref: Option<String>,
    pub blocked: bool,
    pub stale: bool,
    pub notes: Vec<String>,
}

impl TrustPlacementGate {
    pub fn blocked_status(&self) -> &'static str {
        if self.stale {
            "pending-attestation"
        } else {
            "pending-remediation"
        }
    }
}

#[derive(Debug, Clone)]
struct RemediationFailureSummary {
    reason: RemediationFailureReason,
    classification: RemediationFailureClassification,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct RemediationGateContext {
    active_run: Option<RuntimeVmRemediationRun>,
    awaiting_approval: bool,
    last_failure: Option<RemediationFailureSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrustLifecycleState {
    Suspect,
    Quarantined,
    Remediating,
    Restored,
}

impl TrustLifecycleState {
    fn as_str(&self) -> &'static str {
        match self {
            TrustLifecycleState::Suspect => "suspect",
            TrustLifecycleState::Quarantined => "quarantined",
            TrustLifecycleState::Remediating => "remediating",
            TrustLifecycleState::Restored => "restored",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "suspect" => Some(Self::Suspect),
            "quarantined" => Some(Self::Quarantined),
            "remediating" => Some(Self::Remediating),
            "restored" => Some(Self::Restored),
            _ => None,
        }
    }
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
            "lifecycle_state": self.lifecycle_state,
            "previous_lifecycle_state": self.previous_lifecycle_state,
            "freshness_expires_at": self.freshness_expires_at,
            "remediation_attempts": self.remediation_attempts,
            "provenance_ref": self.provenance_ref,
            "provenance": self.provenance,
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

    let registry_state = get_registry_state(pool, vm_instance_id).await?;
    let previous_lifecycle_state = registry_state
        .as_ref()
        .map(|state| state.lifecycle_state.clone());
    let previous_lifecycle = previous_lifecycle_state
        .as_deref()
        .and_then(TrustLifecycleState::from_str);
    let previous_attempts = registry_state
        .as_ref()
        .map(|state| state.remediation_attempts)
        .unwrap_or_default();
    let expected_version = registry_state.as_ref().map(|state| state.version);

    let lifecycle_state = lifecycle_for_attestation(&outcome.status, previous_lifecycle);
    let remediation_attempts = match outcome.status {
        AttestationStatus::Untrusted => previous_attempts.saturating_add(1),
        AttestationStatus::Trusted => 0,
        AttestationStatus::Unknown => previous_attempts,
    };
    let provenance_ref_value = format!("attestation:{}", outcome.attestation_kind.as_str());

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

    let registry = upsert_registry_state(
        pool,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: vm_instance_id,
            attestation_status: outcome.status.as_str(),
            lifecycle_state: lifecycle_state.as_str(),
            remediation_state: remediation_notes.first().map(String::as_str),
            remediation_attempts,
            freshness_deadline: outcome.freshness_deadline,
            provenance_ref: Some(provenance_ref_value.as_str()),
            provenance: metadata.as_ref(),
            expected_version,
        },
    )
    .await?;

    let registry_lifecycle = registry.lifecycle_state.clone();
    let registry_provenance_ref = registry.provenance_ref.clone();
    let registry_provenance = registry.provenance.clone();
    let registry_remediation_attempts = registry.remediation_attempts;

    let trust_event = insert_trust_event(
        pool,
        NewRuntimeVmTrustEvent {
            runtime_vm_instance_id: vm_instance_id,
            attestation_id: Some(attestation.id),
            previous_status: previous_status.as_deref(),
            current_status: outcome.status.as_str(),
            previous_lifecycle_state: previous_lifecycle_state.as_deref(),
            current_lifecycle_state: registry.lifecycle_state.as_str(),
            transition_reason: Some("attestation"),
            remediation_state: remediation_notes.first().map(String::as_str),
            remediation_attempts,
            freshness_deadline: outcome.freshness_deadline,
            provenance_ref: Some(provenance_ref_value.as_str()),
            provenance: metadata.as_ref(),
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
        previous_lifecycle_state,
        lifecycle_state: registry_lifecycle,
        posture_changed,
        freshness_expires_at: outcome.freshness_deadline,
        remediation_attempts: registry_remediation_attempts,
        provenance_ref: registry_provenance_ref,
        provenance: registry_provenance,
        attestation,
        trust_event,
    })
}

// key: trust-placement-gate -> runtime-orchestrator,policy-enforcement
pub async fn evaluate_placement_gate(
    pool: &PgPool,
    server_id: i32,
) -> Result<Option<TrustPlacementGate>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            vmi.id,
            vmi.attestation_status,
            registry.lifecycle_state,
            registry.remediation_state,
            registry.remediation_attempts,
            registry.freshness_deadline,
            registry.provenance_ref,
            registry.updated_at
        FROM runtime_vm_instances vmi
        LEFT JOIN runtime_vm_trust_registry registry
            ON registry.runtime_vm_instance_id = vmi.id
        WHERE vmi.server_id = $1
            AND vmi.terminated_at IS NULL
        ORDER BY vmi.created_at DESC
        LIMIT 1
        "#,
    )
    .bind(server_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let vm_instance_id: i64 = row.get("id");
    let attestation_status: Option<String> = row.try_get::<String, _>("attestation_status").ok();
    let lifecycle_state: Option<String> = row
        .try_get::<Option<String>, _>("lifecycle_state")
        .unwrap_or(None);
    let remediation_state: Option<String> = row
        .try_get::<Option<String>, _>("remediation_state")
        .unwrap_or(None);
    let remediation_attempts: i32 = row.try_get("remediation_attempts").unwrap_or(0);
    let freshness_deadline: Option<DateTime<Utc>> = row
        .try_get::<Option<DateTime<Utc>>, _>("freshness_deadline")
        .unwrap_or(None);
    let provenance_ref: Option<String> = row
        .try_get::<Option<String>, _>("provenance_ref")
        .unwrap_or(None);

    let now = Utc::now();
    let mut notes = Vec::new();
    let mut blocked = false;
    let mut stale = false;

    if let Some(status) = attestation_status.as_deref() {
        notes.push(format!("trust:attestation:{status}"));
    }

    if let Some(ref lifecycle) = lifecycle_state {
        notes.push(format!("trust:lifecycle:{lifecycle}"));
        if matches!(lifecycle.as_str(), "quarantined" | "remediating") {
            blocked = true;
        }
    }

    if let Some(deadline) = freshness_deadline {
        if deadline <= now {
            stale = true;
            blocked = true;
            notes.push(format!("trust:freshness-expired:{}", deadline.to_rfc3339()));
        } else {
            notes.push(format!(
                "trust:freshness-deadline:{}",
                deadline.to_rfc3339()
            ));
        }
    }

    if let Some(state) = remediation_state.as_deref() {
        notes.push(format!("trust:remediation-state:{state}"));
    }

    if remediation_attempts > 0 {
        notes.push(format!("trust:remediation-attempts:{remediation_attempts}"));
    }

    if let Some(ref provenance) = provenance_ref {
        notes.push(format!("trust:provenance:{provenance}"));
    }

    if let Some(context) = remediation_gate_for_instance(pool, vm_instance_id).await? {
        if let Some(active_run) = context.active_run.as_ref() {
            blocked = true;
            let status = active_run.status.as_str();
            notes.push(format!(
                "policy_hook:remediation_gate=active-run:{}:{}",
                active_run.id, status
            ));
            if context.awaiting_approval {
                notes.push(format!("remediation:awaiting-approval:{}", active_run.id));
            }
        } else if context.awaiting_approval {
            blocked = true;
            notes.push("policy_hook:remediation_gate=awaiting-approval".to_string());
        }

        if let Some(summary) = context.last_failure {
            let classification = summary.classification.as_str();
            notes.push(format!(
                "policy_hook:remediation_gate=failure:{}:{}",
                summary.reason.as_str(),
                classification
            ));
            if let Some(timestamp) = summary.completed_at {
                notes.push(format!(
                    "remediation:last-failure-at:{}:{}",
                    summary.reason.as_str(),
                    timestamp.to_rfc3339()
                ));
            }

            match summary.classification {
                RemediationFailureClassification::Structural
                | RemediationFailureClassification::Transient => {
                    blocked = true;
                }
                RemediationFailureClassification::Cancelled => {}
            }
        }
    }

    Ok(Some(TrustPlacementGate {
        vm_instance_id,
        attestation_status,
        lifecycle_state,
        remediation_state,
        remediation_attempts,
        freshness_deadline,
        provenance_ref,
        blocked,
        stale,
        notes,
    }))
}

pub fn remediation_notes_for_status(status: AttestationStatus) -> Vec<String> {
    match status {
        AttestationStatus::Trusted => vec!["remediation:none".to_string()],
        AttestationStatus::Unknown => vec!["remediation:monitor".to_string()],
        AttestationStatus::Untrusted => vec!["remediation:investigate".to_string()],
    }
}

// policy_hook: remediation_gate -> placement veto enrichment
async fn remediation_gate_for_instance(
    pool: &PgPool,
    vm_instance_id: i64,
) -> Result<Option<RemediationGateContext>, sqlx::Error> {
    let active_run = get_active_run_for_instance(pool, vm_instance_id).await?;
    let awaiting_approval = active_run
        .as_ref()
        .map(|run| run.approval_required && run.approval_state == "pending")
        .unwrap_or(false);

    let row = sqlx::query(
        r#"
        SELECT
            id,
            status,
            failure_reason,
            completed_at
        FROM runtime_vm_remediation_runs
        WHERE runtime_vm_instance_id = $1
        ORDER BY started_at DESC
        LIMIT 1
        "#,
    )
    .bind(vm_instance_id)
    .fetch_optional(pool)
    .await?;

    let last_failure = row.and_then(|row| {
        let status: String = row.get("status");
        let completed_at: Option<DateTime<Utc>> = row.try_get("completed_at").unwrap_or(None);
        match status.as_str() {
            "failed" => {
                let reason = row
                    .try_get::<Option<String>, _>("failure_reason")
                    .unwrap_or(None)
                    .and_then(|value| RemediationFailureReason::parse(value.as_str()));
                reason.map(|reason| RemediationFailureSummary {
                    classification: reason.classification(),
                    reason,
                    completed_at,
                })
            }
            "cancelled" => {
                let reason = RemediationFailureReason::Cancelled;
                Some(RemediationFailureSummary {
                    classification: reason.classification(),
                    reason,
                    completed_at,
                })
            }
            _ => None,
        }
    });

    if active_run.is_none() && !awaiting_approval && last_failure.is_none() {
        return Ok(None);
    }

    Ok(Some(RemediationGateContext {
        active_run,
        awaiting_approval,
        last_failure,
    }))
}

fn lifecycle_for_attestation(
    status: &AttestationStatus,
    previous: Option<TrustLifecycleState>,
) -> TrustLifecycleState {
    match status {
        AttestationStatus::Trusted => TrustLifecycleState::Restored,
        AttestationStatus::Unknown => previous.unwrap_or(TrustLifecycleState::Suspect),
        AttestationStatus::Untrusted => match previous {
            Some(TrustLifecycleState::Remediating) => TrustLifecycleState::Remediating,
            _ => TrustLifecycleState::Quarantined,
        },
    }
}
