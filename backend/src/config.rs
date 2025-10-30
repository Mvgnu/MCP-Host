use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::fs;

use crate::runtime::{LibvirtAuthConfig, LibvirtProvisioningConfig};
use serde_json::{json, Value};

/// Secret used for JWT signing. Must be set via the `JWT_SECRET` env variable.
pub static JWT_SECRET: Lazy<String> =
    Lazy::new(|| std::env::var("JWT_SECRET").expect("JWT_SECRET must be set"));

/// Container runtime backend. Defaults to `docker`.
pub static CONTAINER_RUNTIME: Lazy<String> =
    Lazy::new(|| std::env::var("CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".to_string()));

/// Namespace used by the Kubernetes runtime. Defaults to `default`.
pub static K8S_NAMESPACE: Lazy<String> =
    Lazy::new(|| std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "default".to_string()));

/// Service account name used by the Kubernetes runtime. Defaults to `default`.
pub static K8S_SERVICE_ACCOUNT: Lazy<String> =
    Lazy::new(|| std::env::var("K8S_SERVICE_ACCOUNT").unwrap_or_else(|_| "default".to_string()));

/// Optional image pull secret used by the Kubernetes runtime when refreshing registry credentials.
pub static K8S_REGISTRY_SECRET_NAME: Lazy<Option<String>> = Lazy::new(|| {
    std::env::var("K8S_REGISTRY_SECRET_NAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
});

/// Path to a dockerconfigjson file containing registry credentials.
pub static REGISTRY_AUTH_DOCKERCONFIG: Lazy<Option<String>> = Lazy::new(|| {
    std::env::var("REGISTRY_AUTH_DOCKERCONFIG")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
});

/// Architectures that should be targeted when building/publishing images. Provide a
/// comma-separated list such as `linux/amd64,linux/arm64` via `REGISTRY_ARCH_TARGETS`.
/// Defaults to just `linux/amd64` so existing single-arch builds continue to function.
pub static REGISTRY_ARCH_TARGETS: Lazy<Vec<String>> = Lazy::new(|| {
    std::env::var("REGISTRY_ARCH_TARGETS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|raw| {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect::<Vec<_>>()
        })
        .filter(|targets| !targets.is_empty())
        .unwrap_or_else(|| vec!["linux/amd64".to_string()])
});

/// Address the HTTP server should bind to. Defaults to `0.0.0.0`.
pub static BIND_ADDRESS: Lazy<String> =
    Lazy::new(|| std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_string()));

/// Port the HTTP server should listen on. Defaults to `3000`.
pub static BIND_PORT: Lazy<u16> = Lazy::new(|| {
    std::env::var("BIND_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000)
});

/// When set to a truthy value, allows the application to continue running even if database
/// migrations fail. Defaults to `false`.
pub static ALLOW_MIGRATION_FAILURE: Lazy<bool> = Lazy::new(|| {
    std::env::var("ALLOW_MIGRATION_FAILURE")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes")
        })
        .unwrap_or(false)
});

/// key: billing-config -> renewal scan cadence
pub static BILLING_RENEWAL_SCAN_INTERVAL_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("BILLING_RENEWAL_SCAN_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(300)
});

/// key: billing-config -> grace window before suspension/downgrade
pub static BILLING_PAST_DUE_GRACE_DAYS: Lazy<i64> = Lazy::new(|| {
    std::env::var("BILLING_PAST_DUE_GRACE_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(3)
});

/// key: billing-config -> optional fallback plan code for automatic downgrades
pub static BILLING_FALLBACK_PLAN_CODE: Lazy<Option<String>> =
    Lazy::new(|| read_optional_env("BILLING_FALLBACK_PLAN_CODE"));

/// Base URL used to contact the confidential VM hypervisor control plane.
pub static VM_HYPERVISOR_ENDPOINT: Lazy<String> = Lazy::new(|| {
    std::env::var("VM_HYPERVISOR_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8686".to_string())
});

/// Optional bearer token presented to the hypervisor control plane.
pub static VM_HYPERVISOR_TOKEN: Lazy<Option<String>> = Lazy::new(|| {
    std::env::var("VM_HYPERVISOR_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
});

/// Number of log lines to request when fetching VM logs.
pub static VM_LOG_TAIL_LINES: Lazy<usize> = Lazy::new(|| {
    std::env::var("VM_LOG_TAIL_LINES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 64)
        .unwrap_or(500)
});

/// Allowed attestation measurements for trusted workloads.
pub static VM_ATTESTATION_MEASUREMENTS: Lazy<HashSet<String>> = Lazy::new(|| {
    std::env::var("VM_ATTESTATION_MEASUREMENTS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| {
                    let trimmed = item.trim().to_ascii_lowercase();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                })
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
});

/// Base64-encoded Ed25519 public keys accepted for attestation signatures.
pub static VM_ATTESTATION_TRUST_ROOTS: Lazy<Vec<String>> = Lazy::new(|| {
    std::env::var("VM_ATTESTATION_TRUST_ROOTS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| {
                    let trimmed = item.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
});

/// Maximum age (seconds) for attestation evidence before remediation is required.
pub static VM_ATTESTATION_MAX_AGE_SECONDS: Lazy<u64> = Lazy::new(|| {
    std::env::var("VM_ATTESTATION_MAX_AGE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(300)
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmProvisionerDriver {
    Http,
    Libvirt,
}

impl VmProvisionerDriver {
    pub fn as_str(&self) -> &'static str {
        match self {
            VmProvisionerDriver::Http => "http",
            VmProvisionerDriver::Libvirt => "libvirt",
        }
    }
}

fn parse_vm_provisioner_driver() -> VmProvisionerDriver {
    match std::env::var("VM_PROVISIONER_DRIVER") {
        Ok(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "" | "http" => VmProvisionerDriver::Http,
                "libvirt" => VmProvisionerDriver::Libvirt,
                other => panic!(
                    "unsupported VM_PROVISIONER_DRIVER value '{other}'; expected 'http' or 'libvirt'"
                ),
            }
        }
        Err(_) => VmProvisionerDriver::Http,
    }
}

pub static VM_PROVISIONER_DRIVER: Lazy<VmProvisionerDriver> =
    Lazy::new(parse_vm_provisioner_driver);

fn read_optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_secret_env(value_key: &str, file_key: &str) -> Option<String> {
    if let Some(path) = read_optional_env(file_key) {
        match fs::read_to_string(&path) {
            Ok(contents) => {
                let trimmed = contents.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
            Err(err) => panic!("failed to read {file_key} from {path}: {err}"),
        }
    }

    read_optional_env(value_key)
}

fn json_from_env(var: &str, default_value: Value) -> Value {
    match std::env::var(var) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                default_value
            } else {
                serde_json::from_str(trimmed)
                    .unwrap_or_else(|err| panic!("failed to parse {var} as JSON: {err}"))
            }
        }
        Err(_) => default_value,
    }
}

pub fn libvirt_provisioning_config_from_env() -> LibvirtProvisioningConfig {
    let connection_uri = std::env::var("LIBVIRT_CONNECTION_URI")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "qemu:///system".to_string());

    let auth_username = read_optional_env("LIBVIRT_USERNAME");
    let auth_password = read_secret_env("LIBVIRT_PASSWORD", "LIBVIRT_PASSWORD_FILE");
    let auth = if auth_username.is_some() || auth_password.is_some() {
        Some(LibvirtAuthConfig {
            username: auth_username,
            password: auth_password,
        })
    } else {
        None
    };

    let default_isolation_tier = read_optional_env("LIBVIRT_DEFAULT_ISOLATION_TIER");
    let default_memory_mib = std::env::var("LIBVIRT_DEFAULT_MEMORY_MIB")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(4096);
    let default_vcpu_count = std::env::var("LIBVIRT_DEFAULT_VCPU_COUNT")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(4);
    let log_tail = std::env::var("LIBVIRT_LOG_TAIL")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| *VM_LOG_TAIL_LINES);

    let network_template = json_from_env(
        "LIBVIRT_NETWORK_TEMPLATE",
        json!({
            "name": "default",
            "model": "virtio"
        }),
    );
    let volume_template = json_from_env(
        "LIBVIRT_VOLUME_TEMPLATE",
        json!({
            "path": "/var/lib/libvirt/images/mcp.qcow2",
            "driver": "qcow2",
            "target_dev": "vda",
            "target_bus": "virtio"
        }),
    );
    let gpu_passthrough_policy = json_from_env("LIBVIRT_GPU_POLICY", json!({ "enabled": false }));
    let console_source = read_optional_env("LIBVIRT_CONSOLE_SOURCE");

    LibvirtProvisioningConfig {
        connection_uri,
        auth,
        default_isolation_tier,
        default_memory_mib,
        default_vcpu_count,
        log_tail,
        network_template,
        volume_template,
        gpu_passthrough_policy,
        console_source,
    }
}

pub static LIBVIRT_PROVISIONING_CONFIG: Lazy<LibvirtProvisioningConfig> =
    Lazy::new(|| libvirt_provisioning_config_from_env());
