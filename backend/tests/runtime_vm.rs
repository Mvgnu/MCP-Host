use std::collections::HashSet;
use std::time::Duration;

use backend::policy::{
    evaluate_vm_attestation_posture, PolicyDecision, RuntimeBackend, VmAttestationRecord,
};
use backend::runtime::vm::{HttpHypervisorProvisioner, TpmAttestationVerifier};
use backend::runtime::{AttestationVerifier, VmProvisioner};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signer};
use httpmock::prelude::*;
use serde_json::json;

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
    }
}

#[tokio::test]
async fn http_hypervisor_provisioner_covers_lifecycle() {
    let server = MockServer::start_async().await;

    let provision_mock = server.mock(|when, then| {
        when.method(POST).path("/instances");
        then.status(200).json_body(json!({
            "instance_id": "vm-123",
            "isolation_tier": "coco",
            "attestation": {
                "quote": {
                    "report": {
                        "measurement": "valid",
                        "timestamp": Utc::now().to_rfc3339(),
                        "nonce": "abc",
                    },
                    "signature": "",
                }
            },
            "image": "ghcr.io/example/app:latest",
        }));
    });

    let start_mock = server.mock(|when, then| {
        when.method(POST).path("/instances/vm-123/start");
        then.status(202);
    });

    let stop_mock = server.mock(|when, then| {
        when.method(POST).path("/instances/vm-123/stop");
        then.status(202);
    });

    let teardown_mock = server.mock(|when, then| {
        when.method(DELETE).path("/instances/vm-123");
        then.status(204);
    });

    let logs_mock = server.mock(|when, then| {
        when.method(GET)
            .path("/instances/vm-123/logs")
            .query_param("tail", "200");
        then.status(200).body("alpha\nbeta\n");
    });

    let stream_mock = server.mock(|when, then| {
        when.method(GET).path("/instances/vm-123/logs/stream");
        then.status(200).body("line-one\nline-two\n");
    });

    let provisioner = HttpHypervisorProvisioner::new(server.base_url(), None, 200).unwrap();
    let decision = sample_decision();
    let provisioning = provisioner
        .provision(7, &decision, None)
        .await
        .expect("provisioning should succeed");
    assert_eq!(provisioning.instance_id, "vm-123");
    assert_eq!(provisioning.isolation_tier.as_deref(), Some("coco"));

    provisioner.start(&provisioning.instance_id).await.unwrap();
    provisioner.stop(&provisioning.instance_id).await.unwrap();
    provisioner
        .teardown(&provisioning.instance_id)
        .await
        .unwrap();

    let fetched = provisioner
        .fetch_logs(&provisioning.instance_id)
        .await
        .unwrap();
    assert!(fetched.contains("alpha"));

    let mut stream = provisioner
        .stream_logs(&provisioning.instance_id)
        .await
        .unwrap()
        .expect("stream should be available");
    let first = stream.recv().await.expect("first line present");
    let second = stream.recv().await.expect("second line present");
    assert_eq!(first, "line-one");
    assert_eq!(second, "line-two");

    provision_mock.assert();
    start_mock.assert();
    stop_mock.assert();
    teardown_mock.assert();
    logs_mock.assert();
    stream_mock.assert();
}

#[tokio::test]
async fn http_hypervisor_provisioner_surfaces_provision_errors() {
    let server = MockServer::start_async().await;

    let provision_mock = server.mock(|when, then| {
        when.method(POST).path("/instances");
        then.status(503);
    });

    let provisioner = HttpHypervisorProvisioner::new(server.base_url(), None, 200).unwrap();
    let decision = sample_decision();
    let error = provisioner
        .provision(99, &decision, None)
        .await
        .expect_err("provisioning should fail when hypervisor rejects the call");

    let message = format!("{error:#}");
    assert!(
        message.contains("hypervisor rejected provisioning request"),
        "unexpected error message: {message}"
    );

    provision_mock.assert();
}

#[tokio::test]
async fn tpm_attestor_rejects_mismatched_measurement() {
    let secret = SecretKey::from_bytes(&[3u8; 32]).unwrap();
    let public: PublicKey = (&secret).into();
    let signing_key = Keypair { secret, public };
    let report = json!({
        "measurement": "bad-measurement",
        "timestamp": Utc::now().to_rfc3339(),
        "nonce": "nonce-42",
    });
    let report_bytes = serde_json::to_vec(&report).unwrap();
    let signature = signing_key.sign(&report_bytes);
    let evidence = json!({
        "quote": {
            "report": report,
            "signature": Base64Engine.encode(signature.to_bytes()),
            "public_key": Base64Engine.encode(signing_key.public.to_bytes()),
        }
    });

    let mut measurements = HashSet::new();
    measurements.insert("expected-measurement".to_string());
    let attestor = TpmAttestationVerifier::new(
        measurements,
        vec![signing_key.public],
        Duration::from_secs(300),
    );

    let provisioning = backend::runtime::vm::VmProvisioningResult::new(
        "vm-123".to_string(),
        Some("coco".to_string()),
        Some(evidence.clone()),
        "ghcr.io/example/app:latest".to_string(),
        None,
    );

    let outcome = attestor
        .verify(
            7,
            &sample_decision(),
            &provisioning,
            Some(&json!({ "attestation": { "nonce": "nonce-42" } })),
        )
        .await
        .expect("verification should return");
    assert_eq!(
        outcome.status,
        backend::runtime::vm::AttestationStatus::Untrusted
    );
    assert!(outcome
        .notes
        .iter()
        .any(|note| note.contains("measurement:untrusted")));
}

#[test]
fn vm_posture_falls_back_on_stale_pending() {
    let record = VmAttestationRecord {
        status: "pending".to_string(),
        updated_at: Utc::now() - ChronoDuration::minutes(15),
        terminated_at: None,
    };
    let outcome = evaluate_vm_attestation_posture(
        Some(record),
        Utc::now(),
        ChronoDuration::minutes(5),
        RuntimeBackend::Docker,
    );
    assert!(outcome.evaluation_required);
    assert_eq!(outcome.backend_override, Some(RuntimeBackend::Docker));
    assert!(outcome
        .notes
        .iter()
        .any(|note| note == "vm:attestation:stale"));
}
