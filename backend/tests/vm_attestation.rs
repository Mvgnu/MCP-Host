use std::collections::HashSet;

use backend::policy::{PolicyDecision, RuntimeBackend};
use backend::runtime::vm::attestation::{
    detect_kind, normalize_evidence, sev_outcome_from_normalized, AttestationKind,
    AttestationStatus, NormalizedAttestation,
};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;

#[test]
fn normalize_identifies_amd_sev_reports() {
    let timestamp = Utc::now();
    let evidence = json!({
        "amd_sev_snp": {
            "measurement": "ABC123",
            "timestamp": timestamp.to_rfc3339(),
            "nonce": "nonce-42",
            "raw": Base64Engine.encode(b"raw-bytes"),
        }
    });

    let normalized = normalize_evidence(&evidence).expect("normalize");
    assert_eq!(normalized.kind, AttestationKind::AmdSevSnp);
    assert_eq!(normalized.measurement.as_deref(), Some("abc123"));
    assert_eq!(normalized.nonce.as_deref(), Some("nonce-42"));
    assert_eq!(
        normalized.timestamp.unwrap().timestamp(),
        timestamp.timestamp()
    );
}

#[test]
fn normalize_identifies_intel_tdx_reports() {
    let timestamp = Utc::now();
    let evidence = json!({
        "tdx_quote": {
            "mrseam": "FFEEDD",
            "timestamp": timestamp.to_rfc3339(),
            "report_data": "policy-nonce",
        }
    });

    let normalized = normalize_evidence(&evidence).expect("normalize");
    assert_eq!(normalized.kind, AttestationKind::IntelTdx);
    assert_eq!(normalized.measurement.as_deref(), Some("ffeedd"));
    assert_eq!(normalized.nonce.as_deref(), Some("policy-nonce"));
    assert_eq!(
        normalized.timestamp.unwrap().timestamp(),
        timestamp.timestamp()
    );
}

#[test]
fn sev_outcome_tracks_trust_and_freshness() {
    let timestamp = Utc::now();
    let normalized = NormalizedAttestation {
        kind: AttestationKind::AmdSevSnp,
        measurement: Some("trusted".to_string()),
        timestamp: Some(timestamp),
        nonce: None,
        claims: json!({ "measurement": "trusted" }),
        raw_quote: Some(b"raw-sev".to_vec()),
    };
    let trusted = sev_outcome_from_normalized(
        &sample_decision(),
        normalized.clone(),
        &HashSet::from(["trusted".to_string()]),
        ChronoDuration::minutes(5),
    );
    assert_eq!(trusted.status, AttestationStatus::Trusted);
    assert!(trusted
        .notes
        .iter()
        .any(|note| note.contains("attestation:measurement:trusted")));
    assert!(trusted.freshness_deadline.is_some());
    let expected_raw = Base64Engine.encode(b"raw-sev");
    assert_eq!(
        trusted
            .evidence
            .as_ref()
            .and_then(|value| value.get("raw"))
            .and_then(|value| value.as_str()),
        Some(expected_raw.as_str())
    );

    let stale = sev_outcome_from_normalized(
        &sample_decision(),
        NormalizedAttestation {
            timestamp: Some(timestamp - ChronoDuration::minutes(10)),
            ..normalized
        },
        &HashSet::from(["trusted".to_string()]),
        ChronoDuration::minutes(5),
    );
    assert_eq!(stale.status, AttestationStatus::Untrusted);
    assert!(stale.notes.iter().any(|note| note == "attestation:stale"));
}

#[test]
fn detect_kind_handles_unknown_payloads() {
    let evidence = json!({ "custom": { "field": "value" } });
    assert_eq!(detect_kind(&evidence), AttestationKind::Unknown);
}

fn sample_decision() -> PolicyDecision {
    PolicyDecision {
        backend: RuntimeBackend::VirtualMachine,
        candidate_backend: RuntimeBackend::VirtualMachine,
        image: "ghcr.io/example/app:latest".to_string(),
        requires_build: false,
        artifact_run_id: None,
        manifest_digest: None,
        policy_version: "runtime-policy-v0.1".to_string(),
        evaluation_required: false,
        governance_required: false,
        governance_run_id: None,
        tier: Some("confidential".to_string()),
        health_overall: Some("healthy".to_string()),
        capability_requirements: vec![],
        capabilities_satisfied: true,
        executor_name: None,
        notes: vec![],
        promotion_track_id: None,
        promotion_track_name: None,
        promotion_stage: None,
        promotion_status: None,
        promotion_notes: vec![],
        provider_key_posture: None,
    }
}
