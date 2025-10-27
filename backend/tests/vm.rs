use std::collections::HashSet;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::policy::{
    evaluate_vm_attestation_posture, PolicyDecision, RuntimeBackend, VmAttestationRecord,
};
use backend::runtime::vm::libvirt::LibvirtVmProvisioner;
use backend::runtime::vm::{
    libvirt::testing::InMemoryLibvirtDriver, AttestationStatus, HypervisorSnapshot,
    TpmAttestationVerifier, VmProvisioningResult,
};
use backend::runtime::{AttestationVerifier, LibvirtProvisioningConfig, VmProvisioner};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signer};
use once_cell::sync::Lazy;
use serde_json::json;

static ENV_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn with_env<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let guard = ENV_GUARD.lock().expect("env guard poisoned");
    let mut previous = Vec::with_capacity(vars.len());
    for (key, value) in vars {
        previous.push(((*key).to_string(), std::env::var(key).ok()));
        match value {
            Some(val) => std::env::set_var(key, val),
            None => std::env::remove_var(key),
        }
    }

    let result = catch_unwind(AssertUnwindSafe(f));

    for (key, old) in previous.into_iter() {
        if let Some(val) = old {
            std::env::set_var(&key, val);
        } else {
            std::env::remove_var(&key);
        }
    }

    drop(guard);

    match result {
        Ok(value) => value,
        Err(panic) => resume_unwind(panic),
    }
}

fn load_libvirt_config(vars: &[(&str, Option<&str>)]) -> LibvirtProvisioningConfig {
    with_env(vars, || backend::libvirt_provisioning_config_from_env())
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
    }
}

fn sample_keypair() -> Keypair {
    let secret_bytes = [0x11u8; 32];
    let secret = SecretKey::from_bytes(&secret_bytes).expect("secret key");
    let public = PublicKey::from(&secret);
    Keypair { secret, public }
}

fn build_tpm_evidence(
    keypair: &Keypair,
    measurement: &str,
    timestamp: chrono::DateTime<Utc>,
    nonce: Option<&str>,
) -> serde_json::Value {
    let mut report = json!({
        "measurement": measurement,
        "timestamp": timestamp.to_rfc3339(),
    });
    if let Some(nonce_value) = nonce {
        report["nonce"] = json!(nonce_value);
    }

    let message = serde_json::to_vec(&report).expect("serialize report");
    let signature = keypair.sign(&message);

    json!({
        "quote": {
            "report": report,
            "signature": Base64Engine.encode(signature.to_bytes()),
            "public_key": Base64Engine.encode(keypair.public.as_bytes()),
        }
    })
}

fn sample_config() -> LibvirtProvisioningConfig {
    let network_json = json!({ "name": "default", "model": "virtio" }).to_string();
    let volume_json = json!({
        "path": "/var/lib/libvirt/images/mcp.qcow2",
        "driver": "qcow2",
        "target_dev": "vda",
        "target_bus": "virtio"
    })
    .to_string();
    let gpu_json = json!({ "enabled": false }).to_string();
    let overrides = vec![
        ("LIBVIRT_CONNECTION_URI", Some("test:///default")),
        ("LIBVIRT_DEFAULT_ISOLATION_TIER", Some("coco")),
        ("LIBVIRT_DEFAULT_MEMORY_MIB", Some("4096")),
        ("LIBVIRT_DEFAULT_VCPU_COUNT", Some("4")),
        ("LIBVIRT_LOG_TAIL", Some("200")),
        ("LIBVIRT_NETWORK_TEMPLATE", Some(network_json.as_str())),
        ("LIBVIRT_VOLUME_TEMPLATE", Some(volume_json.as_str())),
        ("LIBVIRT_GPU_POLICY", Some(gpu_json.as_str())),
        ("LIBVIRT_CONSOLE_SOURCE", Some("pty")),
        ("LIBVIRT_USERNAME", None),
        ("LIBVIRT_PASSWORD", None),
        ("LIBVIRT_PASSWORD_FILE", None),
    ];
    load_libvirt_config(&overrides)
}

#[tokio::test]
async fn libvirt_provisioner_respects_log_tail_limits() {
    let overrides = vec![
        ("LIBVIRT_CONNECTION_URI", Some("test:///default")),
        ("LIBVIRT_LOG_TAIL", Some("1")),
        ("LIBVIRT_USERNAME", None),
        ("LIBVIRT_PASSWORD", None),
        ("LIBVIRT_PASSWORD_FILE", None),
    ];
    let config = load_libvirt_config(&overrides);
    let driver = Arc::new(InMemoryLibvirtDriver::default());
    let provisioner = LibvirtVmProvisioner::new(driver, config);
    let decision = sample_decision();

    let provisioning = provisioner
        .provision(21, &decision, None)
        .await
        .expect("provisioning should succeed");

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
    assert_eq!(logs.trim(), "vm-stopped");

    let mut stream = provisioner
        .stream_logs(&provisioning.instance_id)
        .await
        .expect("stream creation should succeed")
        .expect("stream should be present");
    let first = stream.recv().await.expect("first log line present");
    assert_eq!(first, "vm-started");
}

#[test]
fn libvirt_config_defaults_apply() {
    let overrides = vec![
        ("LIBVIRT_CONNECTION_URI", Some("test:///minimal")),
        ("LIBVIRT_USERNAME", None),
        ("LIBVIRT_PASSWORD", None),
        ("LIBVIRT_PASSWORD_FILE", None),
        ("LIBVIRT_DEFAULT_ISOLATION_TIER", None),
        ("LIBVIRT_DEFAULT_MEMORY_MIB", None),
        ("LIBVIRT_DEFAULT_VCPU_COUNT", None),
        ("LIBVIRT_LOG_TAIL", None),
        ("LIBVIRT_NETWORK_TEMPLATE", None),
        ("LIBVIRT_VOLUME_TEMPLATE", None),
        ("LIBVIRT_GPU_POLICY", None),
        ("LIBVIRT_CONSOLE_SOURCE", None),
    ];
    let config = load_libvirt_config(&overrides);

    assert_eq!(config.connection_uri, "test:///minimal");
    assert!(config.auth.is_none());
    assert!(config.default_isolation_tier.is_none());
    assert_eq!(config.default_memory_mib, 4096);
    assert_eq!(config.default_vcpu_count, 4);
    assert_eq!(config.log_tail, *backend::VM_LOG_TAIL_LINES);
    assert_eq!(
        config.network_template,
        json!({ "name": "default", "model": "virtio" })
    );
    assert_eq!(
        config.volume_template,
        json!({
            "path": "/var/lib/libvirt/images/mcp.qcow2",
            "driver": "qcow2",
            "target_dev": "vda",
            "target_bus": "virtio"
        })
    );
    assert_eq!(config.gpu_passthrough_policy, json!({ "enabled": false }));
    assert!(config.console_source.is_none());
}

#[test]
fn libvirt_config_overrides_apply() {
    let network_json = json!({ "name": "isolated", "model": "virtio" }).to_string();
    let volume_json = json!({
        "path": "/var/lib/libvirt/images/custom.qcow2",
        "driver": "qcow2",
        "target_dev": "vdb",
        "target_bus": "virtio"
    })
    .to_string();
    let gpu_json = json!({
        "enabled": true,
        "devices": [{
            "domain": "0x0000",
            "bus": "0x65",
            "slot": "0x00",
            "function": "0x0"
        }]
    })
    .to_string();
    let overrides = vec![
        ("LIBVIRT_CONNECTION_URI", Some("qemu+ssh://libvirt/system")),
        ("LIBVIRT_USERNAME", Some("operator")),
        ("LIBVIRT_PASSWORD", Some("secret-token")),
        ("LIBVIRT_PASSWORD_FILE", None),
        ("LIBVIRT_DEFAULT_ISOLATION_TIER", Some("sev")),
        ("LIBVIRT_DEFAULT_MEMORY_MIB", Some("8192")),
        ("LIBVIRT_DEFAULT_VCPU_COUNT", Some("6")),
        ("LIBVIRT_LOG_TAIL", Some("42")),
        ("LIBVIRT_NETWORK_TEMPLATE", Some(network_json.as_str())),
        ("LIBVIRT_VOLUME_TEMPLATE", Some(volume_json.as_str())),
        ("LIBVIRT_GPU_POLICY", Some(gpu_json.as_str())),
        ("LIBVIRT_CONSOLE_SOURCE", Some("serial")),
    ];
    let config = load_libvirt_config(&overrides);

    assert_eq!(config.connection_uri, "qemu+ssh://libvirt/system");
    let auth = config.auth.expect("auth should be configured");
    assert_eq!(auth.username.as_deref(), Some("operator"));
    assert_eq!(auth.password.as_deref(), Some("secret-token"));
    assert_eq!(config.default_isolation_tier.as_deref(), Some("sev"));
    assert_eq!(config.default_memory_mib, 8192);
    assert_eq!(config.default_vcpu_count, 6);
    assert_eq!(config.log_tail, 42);
    assert_eq!(
        config.network_template,
        serde_json::from_str::<serde_json::Value>(&network_json)
            .expect("network json should parse")
    );
    assert_eq!(
        config.volume_template,
        serde_json::from_str::<serde_json::Value>(&volume_json).expect("volume json should parse")
    );
    assert_eq!(
        config.gpu_passthrough_policy,
        serde_json::from_str::<serde_json::Value>(&gpu_json).expect("gpu json should parse")
    );
    assert_eq!(config.console_source.as_deref(), Some("serial"));
}

#[tokio::test]
async fn tpm_attestation_happy_path_trusts_measurement() {
    let keypair = sample_keypair();
    let measurement = "trusted-image";
    let evidence = build_tpm_evidence(&keypair, measurement, Utc::now(), Some("nonce-123"));

    let verifier = TpmAttestationVerifier::new(
        HashSet::from([measurement.to_string()]),
        vec![keypair.public],
        Duration::from_secs(600),
    );

    let provisioning = VmProvisioningResult::new(
        "vm-123".to_string(),
        Some("confidential".to_string()),
        Some(evidence.clone()),
        "ghcr.io/example/app:latest".to_string(),
        Some(HypervisorSnapshot::new(
            "test".to_string(),
            None,
            None,
            None,
            None,
        )),
    );

    let outcome = verifier
        .verify(7, &sample_decision(), &provisioning, None)
        .await
        .expect("attestation should succeed");

    assert_eq!(outcome.status, AttestationStatus::Trusted);
    assert!(outcome
        .notes
        .iter()
        .any(|note| note == "attestation:kind:tpm"));
    assert!(outcome
        .notes
        .iter()
        .any(|note| note.contains("attestation:measurement:trusted-image")));
    assert!(outcome.freshness_deadline.is_some());
    assert_eq!(outcome.evidence, Some(evidence));
}

#[tokio::test]
async fn tpm_attestation_rejects_stale_reports() {
    let keypair = sample_keypair();
    let measurement = "trusted-image";
    let stale_time = Utc::now() - ChronoDuration::minutes(15);
    let evidence = build_tpm_evidence(&keypair, measurement, stale_time, None);

    let verifier = TpmAttestationVerifier::new(
        HashSet::from([measurement.to_string()]),
        vec![keypair.public],
        Duration::from_secs(60),
    );

    let provisioning = VmProvisioningResult::new(
        "vm-456".to_string(),
        None,
        Some(evidence.clone()),
        "ghcr.io/example/app:latest".to_string(),
        None,
    );

    let outcome = verifier
        .verify(9, &sample_decision(), &provisioning, None)
        .await
        .expect("attestation should complete");

    assert_eq!(outcome.status, AttestationStatus::Untrusted);
    assert!(outcome.notes.iter().any(|note| note == "attestation:stale"));
    assert_eq!(outcome.evidence, Some(evidence));
}

#[tokio::test]
async fn tpm_attestation_rejects_signature_mismatch() {
    let keypair = sample_keypair();
    let measurement = "trusted-image";
    let evidence = build_tpm_evidence(&keypair, measurement, Utc::now(), None);

    // Tamper with the measurement after signing to invalidate signature.
    let mut tampered = evidence.clone();
    tampered["quote"]["report"]["measurement"] = json!("tampered-image");

    let verifier = TpmAttestationVerifier::new(
        HashSet::from([measurement.to_string()]),
        vec![keypair.public],
        Duration::from_secs(600),
    );

    let provisioning = VmProvisioningResult::new(
        "vm-789".to_string(),
        None,
        Some(tampered.clone()),
        "ghcr.io/example/app:latest".to_string(),
        None,
    );

    let outcome = verifier
        .verify(5, &sample_decision(), &provisioning, None)
        .await
        .expect("attestation should complete");

    assert_eq!(outcome.status, AttestationStatus::Untrusted);
    assert!(outcome
        .notes
        .iter()
        .any(|note| note == "attestation:signature-invalid"));
    assert_eq!(outcome.evidence, Some(tampered));
}

#[tokio::test]
async fn tpm_attestation_blocks_untrusted_measurement() {
    let keypair = sample_keypair();
    let measurement = "untrusted-image";
    let evidence = build_tpm_evidence(&keypair, measurement, Utc::now(), None);

    let verifier = TpmAttestationVerifier::new(
        HashSet::from(["trusted-image".to_string()]),
        vec![keypair.public],
        Duration::from_secs(600),
    );

    let provisioning = VmProvisioningResult::new(
        "vm-321".to_string(),
        None,
        Some(evidence.clone()),
        "ghcr.io/example/app:latest".to_string(),
        None,
    );

    let outcome = verifier
        .verify(3, &sample_decision(), &provisioning, None)
        .await
        .expect("attestation should complete");

    assert_eq!(outcome.status, AttestationStatus::Untrusted);
    assert!(outcome
        .notes
        .iter()
        .any(|note| note.contains("attestation:measurement:untrusted")));
    assert_eq!(outcome.evidence, Some(evidence));
}

#[tokio::test]
async fn libvirt_provisioner_covers_lifecycle() {
    let driver = Arc::new(InMemoryLibvirtDriver::default());
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
    let driver = Arc::new(InMemoryLibvirtDriver::default());
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
