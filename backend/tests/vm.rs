use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use backend::policy::{
    evaluate_vm_attestation_posture, PolicyDecision, RuntimeBackend, VmAttestationRecord,
};
use backend::runtime::vm::libvirt::{
    testing::InMemoryLibvirtDriver, LibvirtProvisioningConfig, LibvirtVmProvisioner,
};
use backend::runtime::vm::{HypervisorSnapshot, TpmAttestationVerifier, VmProvisioningResult};
use backend::runtime::{AttestationVerifier, VmProvisioner};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signer};
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

fn sample_config() -> LibvirtProvisioningConfig {
    LibvirtProvisioningConfig {
        connection_uri: "test:///default".to_string(),
        auth: None,
        default_isolation_tier: Some("coco".to_string()),
        default_memory_mib: 4096,
        default_vcpu_count: 4,
        log_tail: 200,
        network_template: json!({ "name": "default", "model": "virtio" }),
        volume_template: json!({
            "path": "/var/lib/libvirt/images/mcp.qcow2",
            "driver": "qcow2",
            "target_dev": "vda",
            "target_bus": "virtio"
        }),
        gpu_passthrough_policy: json!({ "enabled": false }),
        console_source: Some("pty".to_string()),
    }
}

#[tokio::test]
async fn libvirt_provisioner_covers_lifecycle() {
    let driver = Arc::new(InMemoryLibvirtDriver::new());
    let config = sample_config();
    let provisioner = LibvirtVmProvisioner::new(driver, config);
    let decision = sample_decision();

    let provisioning = provisioner
        .provision(7, &decision, None)
        .await
        .expect("provisioning should succeed");
    assert!(provisioning.hypervisor.is_some());
    assert_eq!(provisioning.isolation_tier.as_deref(), Some("confidential"));

    provisioner
        .start(&provisioning.instance_id)
        .await
        .expect("start should succeed");
    provisioner
        .stop(&provisioning.instance_id)
        .await
        .expect("stop should succeed");

    let logs = provisioner
        .fetch_logs(&provisioning.instance_id)
        .await
        .expect("logs should be available");
    assert!(logs.contains("vm-started"));
    assert!(logs.contains("vm-stopped"));

    let mut stream = provisioner
        .stream_logs(&provisioning.instance_id)
        .await
        .expect("stream creation should succeed")
        .expect("stream should be present");
    let first = stream.recv().await.expect("first log line present");
    assert_eq!(first, "vm-started");
}

#[tokio::test]
async fn libvirt_provisioner_surfaces_shutdown_errors() {
    let driver = Arc::new(InMemoryLibvirtDriver::new());
    let config = sample_config();
    let provisioner = LibvirtVmProvisioner::new(driver, config);

    let error = provisioner
        .stop("missing-domain")
        .await
        .expect_err("stop should fail for unknown domain");
    let message = format!("{error:#}");
    assert!(message.contains("domain not found"));
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

    let provisioning = VmProvisioningResult::new(
        "vm-123".to_string(),
        Some("coco".to_string()),
        Some(evidence.clone()),
        "ghcr.io/example/app:latest".to_string(),
        Some(HypervisorSnapshot::new(
            "test:///default".to_string(),
            None,
            None,
            None,
            None,
        )),
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
}

#[tokio::test]
async fn evaluate_attestation_record_marks_untrusted() {
    let record = VmAttestationRecord {
        status: "untrusted".to_string(),
        updated_at: Utc::now() - ChronoDuration::minutes(10),
        terminated_at: None,
    };

    let outcome = evaluate_vm_attestation_posture(
        Some(record),
        Utc::now(),
        ChronoDuration::minutes(15),
        RuntimeBackend::Docker,
    );

    assert_eq!(outcome.attestation_status.as_deref(), Some("untrusted"));
    assert_eq!(outcome.backend_override, Some(RuntimeBackend::Docker));
    assert!(outcome.evaluation_required);
}
