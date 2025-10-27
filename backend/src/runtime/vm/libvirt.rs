use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "libvirt-executor")]
use anyhow::Context;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc::Receiver;
use tokio::sync::{mpsc, Mutex};
#[cfg(feature = "libvirt-executor")]
use tokio::task::spawn_blocking;

use super::{HypervisorSnapshot, VmProvisioner, VmProvisioningResult};
use crate::policy::PolicyDecision;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibvirtAuthConfig {
    pub username: Option<String>,
    pub password: Option<String>,
}

impl LibvirtAuthConfig {
    pub fn snapshot(&self) -> Value {
        json!({
            "username": self.username,
            "secret_provided": self.password.as_ref().map(|_| true).unwrap_or(false),
            "method": "password",
        })
    }
}

#[derive(Debug, Clone)]
pub struct LibvirtProvisioningConfig {
    pub connection_uri: String,
    pub auth: Option<LibvirtAuthConfig>,
    pub default_isolation_tier: Option<String>,
    pub default_memory_mib: u64,
    pub default_vcpu_count: u32,
    pub log_tail: usize,
    pub network_template: Value,
    pub volume_template: Value,
    pub gpu_passthrough_policy: Value,
    pub console_source: Option<String>,
}

impl LibvirtProvisioningConfig {
    pub fn sanitized_snapshot(&self) -> Option<Value> {
        self.auth.as_ref().map(|auth| auth.snapshot())
    }
}

#[derive(Debug, Clone)]
pub struct LibvirtProvisionSpec {
    pub domain_name: String,
    pub image: String,
    pub memory_mib: u64,
    pub vcpu_count: u32,
    pub network_template: Value,
    pub volume_template: Value,
    pub gpu_policy: Value,
    pub isolation_tier: Option<String>,
    pub user_config: Option<Value>,
    pub attestation_hint: Option<Value>,
}

impl LibvirtProvisionSpec {
    #[cfg(not(feature = "libvirt-executor"))]
    fn mark_unused_fields(&self) {
        let _ = (
            self.memory_mib,
            self.vcpu_count,
            &self.network_template,
            &self.volume_template,
            &self.gpu_policy,
            &self.user_config,
            &self.attestation_hint,
        );
    }
}

#[derive(Debug, Clone)]
pub struct LibvirtProvisionedDomain {
    pub instance_id: String,
    pub isolation_tier: Option<String>,
    pub attestation_evidence: Option<Value>,
}

#[async_trait]
pub trait LibvirtDriver: Send + Sync {
    async fn provision_domain(
        &self,
        spec: &LibvirtProvisionSpec,
    ) -> Result<LibvirtProvisionedDomain>;

    async fn start_domain(&self, name: &str) -> Result<()>;

    async fn shutdown_domain(&self, name: &str) -> Result<()>;

    async fn destroy_domain(&self, name: &str) -> Result<()>;

    async fn fetch_console(&self, name: &str, tail: usize) -> Result<String>;

    async fn stream_console(&self, name: &str) -> Result<Option<Receiver<String>>>;
}

pub struct LibvirtVmProvisioner {
    driver: Arc<dyn LibvirtDriver>,
    config: LibvirtProvisioningConfig,
}

impl LibvirtVmProvisioner {
    #[cfg_attr(not(feature = "libvirt-executor"), allow(dead_code))]
    pub fn new(driver: Arc<dyn LibvirtDriver>, config: LibvirtProvisioningConfig) -> Self {
        Self { driver, config }
    }

    fn plan_spec(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        config: Option<&Value>,
    ) -> (LibvirtProvisionSpec, HypervisorSnapshot) {
        let vm_overrides = config.and_then(|cfg| cfg.get("vm"));
        let memory_mib = vm_overrides
            .and_then(|vm| vm.get("memory_mib").and_then(|value| value.as_u64()))
            .unwrap_or(self.config.default_memory_mib);
        let vcpu_count = vm_overrides
            .and_then(|vm| vm.get("vcpu_count").and_then(|value| value.as_u64()))
            .map(|value| value as u32)
            .unwrap_or(self.config.default_vcpu_count);

        let image = vm_overrides
            .and_then(|vm| vm.get("image").and_then(|value| value.as_str()))
            .map(|value| value.to_string())
            .unwrap_or_else(|| decision.image.clone());

        let isolation_tier = vm_overrides
            .and_then(|vm| vm.get("isolation_tier").and_then(|value| value.as_str()))
            .map(|value| value.to_string())
            .or_else(|| decision.tier.clone())
            .or_else(|| self.config.default_isolation_tier.clone());

        let attestation_hint = config.and_then(|cfg| cfg.get("attestation")).cloned();

        let domain_name = format!("mcp-vm-{}-{}", server_id, Utc::now().format("%Y%m%d%H%M%S"));

        let snapshot = HypervisorSnapshot::new(
            self.config.connection_uri.clone(),
            self.config.sanitized_snapshot(),
            Some(self.config.network_template.clone()),
            Some(self.config.volume_template.clone()),
            Some(self.config.gpu_passthrough_policy.clone()),
        );

        let spec = LibvirtProvisionSpec {
            domain_name,
            image,
            memory_mib,
            vcpu_count,
            network_template: self.config.network_template.clone(),
            volume_template: self.config.volume_template.clone(),
            gpu_policy: self.config.gpu_passthrough_policy.clone(),
            isolation_tier,
            user_config: config.cloned(),
            attestation_hint,
        };

        #[cfg(not(feature = "libvirt-executor"))]
        spec.mark_unused_fields();

        #[cfg(not(feature = "libvirt-executor"))]
        let _ = &self.config.console_source;

        (spec, snapshot)
    }
}

#[async_trait]
impl VmProvisioner for LibvirtVmProvisioner {
    async fn provision(
        &self,
        server_id: i32,
        decision: &PolicyDecision,
        config: Option<&Value>,
    ) -> Result<VmProvisioningResult> {
        let (spec, snapshot) = self.plan_spec(server_id, decision, config);
        let provisioned = self.driver.provision_domain(&spec).await?;

        let isolation_tier = provisioned
            .isolation_tier
            .or_else(|| spec.isolation_tier.clone());

        let attestation = if provisioned.attestation_evidence.is_some() {
            provisioned.attestation_evidence.clone()
        } else {
            spec.attestation_hint.clone()
        };

        let mut result = VmProvisioningResult::new(
            provisioned.instance_id,
            isolation_tier,
            attestation,
            spec.image,
            Some(snapshot),
        );
        result.attestation_hint = spec.attestation_hint.clone();
        Ok(result)
    }

    async fn start(&self, instance_id: &str) -> Result<()> {
        self.driver.start_domain(instance_id).await
    }

    async fn stop(&self, instance_id: &str) -> Result<()> {
        self.driver.shutdown_domain(instance_id).await
    }

    async fn teardown(&self, instance_id: &str) -> Result<()> {
        self.driver.destroy_domain(instance_id).await
    }

    async fn fetch_logs(&self, instance_id: &str) -> Result<String> {
        self.driver
            .fetch_console(instance_id, self.config.log_tail)
            .await
    }

    async fn stream_logs(&self, instance_id: &str) -> Result<Option<Receiver<String>>> {
        self.driver.stream_console(instance_id).await
    }
}

#[cfg(feature = "libvirt-executor")]
#[derive(Clone)]
pub struct RealLibvirtDriver {
    uri: String,
    auth: Option<LibvirtAuthConfig>,
    console_source: Option<String>,
}

#[cfg(feature = "libvirt-executor")]
impl RealLibvirtDriver {
    pub fn new(
        uri: impl Into<String>,
        auth: Option<LibvirtAuthConfig>,
        console_source: Option<String>,
    ) -> Self {
        Self {
            uri: uri.into(),
            auth,
            console_source,
        }
    }

    fn connect(&self) -> Result<virt::connect::Connect> {
        if let Some(auth) = &self.auth {
            let mut creds = Vec::new();
            if auth.username.is_some() {
                creds.push(virt::sys::VIR_CRED_AUTHNAME);
            }
            if auth.password.is_some() {
                creds.push(virt::sys::VIR_CRED_PASSPHRASE);
            }
            let username = auth.username.clone();
            let password = auth.password.clone();
            let mut callback = virt::connect::ConnectAuth::new(creds, move |credentials| {
                for cred in credentials.iter_mut() {
                    match cred.typed {
                        x if x == virt::sys::VIR_CRED_AUTHNAME as i32 => {
                            if let Some(user) = &username {
                                cred.result = Some(user.clone());
                            }
                        }
                        x if x == virt::sys::VIR_CRED_PASSPHRASE as i32 => {
                            if let Some(secret) = &password {
                                cred.result = Some(secret.clone());
                            }
                        }
                        _ => {}
                    }
                }
            });
            virt::connect::Connect::open_auth(Some(&self.uri), &mut callback, 0)
                .map_err(|err| anyhow!(err.to_string()))
        } else {
            virt::connect::Connect::open(Some(&self.uri)).map_err(|err| anyhow!(err.to_string()))
        }
    }

    fn build_domain_xml(&self, spec: &LibvirtProvisionSpec) -> String {
        let disk_path = spec
            .volume_template
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("/var/lib/libvirt/images/mcp.qcow2");
        let disk_driver = spec
            .volume_template
            .get("driver")
            .and_then(|value| value.as_str())
            .unwrap_or("qcow2");
        let target_dev = spec
            .volume_template
            .get("target_dev")
            .and_then(|value| value.as_str())
            .unwrap_or("vda");
        let target_bus = spec
            .volume_template
            .get("target_bus")
            .and_then(|value| value.as_str())
            .unwrap_or("virtio");
        let network_name = spec
            .network_template
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("default");
        let network_model = spec
            .network_template
            .get("model")
            .and_then(|value| value.as_str())
            .unwrap_or("virtio");

        let mut gpu_devices = String::new();
        if spec
            .gpu_policy
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            if let Some(devices) = spec
                .gpu_policy
                .get("devices")
                .and_then(|value| value.as_array())
            {
                for device in devices {
                    let domain = device
                        .get("domain")
                        .and_then(|value| value.as_str())
                        .unwrap_or("0x0000");
                    let bus = device
                        .get("bus")
                        .and_then(|value| value.as_str())
                        .unwrap_or("0x00");
                    let slot = device
                        .get("slot")
                        .and_then(|value| value.as_str())
                        .unwrap_or("0x00");
                    let function = device
                        .get("function")
                        .and_then(|value| value.as_str())
                        .unwrap_or("0x0");
                    gpu_devices.push_str(&format!(
                        "            <hostdev mode='subsystem' type='pci' managed='yes'>\n                <source>\n                    <address domain='{domain}' bus='{bus}' slot='{slot}' function='{function}'/>\n                </source>\n            </hostdev>\n"
                    ));
                }
            }
        }

        let console_type = self.console_source.as_deref().unwrap_or("pty");

        format!(
            "<domain type='kvm'>\n    <name>{name}</name>\n    <memory unit='MiB'>{memory}</memory>\n    <vcpu>{vcpu}</vcpu>\n    <os>\n        <type arch='x86_64' machine='pc-q35-6.2'>hvm</type>\n    </os>\n    <devices>\n        <disk type='file' device='disk'>\n            <driver name='qemu' type='{driver}'/>\n            <source file='{path}'/>\n            <target dev='{target_dev}' bus='{target_bus}'/>\n        </disk>\n        <interface type='network'>\n            <source network='{network}'/>\n            <model type='{network_model}'/>\n        </interface>\n        <serial type='{console_type}'>\n            <target port='0'/>\n        </serial>\n        <console type='{console_type}'>\n            <target type='serial' port='0'/>\n        </console>\n{gpu_devices}    </devices>\n</domain>",
            name = spec.domain_name,
            memory = spec.memory_mib,
            vcpu = spec.vcpu_count,
            driver = disk_driver,
            path = disk_path,
            target_dev = target_dev,
            target_bus = target_bus,
            network = network_name,
            network_model = network_model,
            console_type = console_type,
            gpu_devices = gpu_devices
        )
    }

    fn read_console_once(&self, name: &str) -> Result<String> {
        let mut conn = self.connect().context("failed to connect to libvirt")?;
        let domain = virt::domain::Domain::lookup_by_name(&conn, name)
            .map_err(|err| anyhow!(err.to_string()))?;
        let stream = virt::stream::Stream::new(&conn, 0).map_err(|err| anyhow!(err.to_string()))?;
        domain
            .open_console(None, &stream, 0)
            .map_err(|err| anyhow!(err.to_string()))?;
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.recv(&mut chunk) {
                Ok(0) => break,
                Ok(len) => buffer.extend_from_slice(&chunk[..len]),
                Err(err) => {
                    return Err(anyhow!(err.to_string()));
                }
            }
        }
        stream.finish().ok();
        let content = String::from_utf8_lossy(&buffer).to_string();
        Ok(content)
    }

    fn read_console_stream(&self, name: &str, mut tx: mpsc::Sender<String>) -> Result<()> {
        let mut conn = self.connect().context("failed to connect to libvirt")?;
        let domain = virt::domain::Domain::lookup_by_name(&conn, name)
            .map_err(|err| anyhow!(err.to_string()))?;
        let stream = virt::stream::Stream::new(&conn, 0).map_err(|err| anyhow!(err.to_string()))?;
        domain
            .open_console(None, &stream, 0)
            .map_err(|err| anyhow!(err.to_string()))?;
        let mut chunk = [0u8; 1024];
        let mut buffer = Vec::new();
        loop {
            match stream.recv(&mut chunk) {
                Ok(0) => break,
                Ok(len) => {
                    buffer.extend_from_slice(&chunk[..len]);
                    while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                        let line = buffer.drain(..=pos).collect::<Vec<u8>>();
                        let clean = String::from_utf8_lossy(&line[..line.len().saturating_sub(1)])
                            .to_string();
                        if tx.blocking_send(clean).is_err() {
                            stream.finish().ok();
                            return Ok(());
                        }
                    }
                }
                Err(err) => {
                    stream.finish().ok();
                    return Err(anyhow!(err.to_string()));
                }
            }
        }
        if !buffer.is_empty() {
            let trailing = String::from_utf8_lossy(&buffer).to_string();
            let _ = tx.blocking_send(trailing);
        }
        stream.finish().ok();
        Ok(())
    }
}

#[cfg(feature = "libvirt-executor")]
#[async_trait]
impl LibvirtDriver for RealLibvirtDriver {
    async fn provision_domain(
        &self,
        spec: &LibvirtProvisionSpec,
    ) -> Result<LibvirtProvisionedDomain> {
        let driver = self.clone();
        let spec = spec.clone();
        spawn_blocking(move || {
            let xml = driver.build_domain_xml(&spec);
            let mut conn = driver.connect().context("failed to connect to libvirt")?;
            virt::domain::Domain::define_xml(&conn, &xml)
                .map_err(|err| anyhow!(err.to_string()))?;
            Ok::<_, anyhow::Error>(LibvirtProvisionedDomain {
                instance_id: spec.domain_name.clone(),
                isolation_tier: spec.isolation_tier.clone(),
                attestation_evidence: spec.attestation_hint.clone(),
            })
        })
        .await
        .context("failed to join libvirt provisioning task")??
    }

    async fn start_domain(&self, name: &str) -> Result<()> {
        let driver = self.clone();
        let name = name.to_string();
        spawn_blocking(move || {
            let mut conn = driver.connect().context("failed to connect to libvirt")?;
            let domain = virt::domain::Domain::lookup_by_name(&conn, &name)
                .map_err(|err| anyhow!(err.to_string()))?;
            domain.create().map_err(|err| anyhow!(err.to_string()))?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("failed to join libvirt start task")??
    }

    async fn shutdown_domain(&self, name: &str) -> Result<()> {
        let driver = self.clone();
        let name = name.to_string();
        spawn_blocking(move || {
            let mut conn = driver.connect().context("failed to connect to libvirt")?;
            let domain = virt::domain::Domain::lookup_by_name(&conn, &name)
                .map_err(|err| anyhow!(err.to_string()))?;
            domain.shutdown().map_err(|err| anyhow!(err.to_string()))?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("failed to join libvirt shutdown task")??
    }

    async fn destroy_domain(&self, name: &str) -> Result<()> {
        let driver = self.clone();
        let name = name.to_string();
        spawn_blocking(move || {
            let mut conn = driver.connect().context("failed to connect to libvirt")?;
            let mut domain = virt::domain::Domain::lookup_by_name(&conn, &name)
                .map_err(|err| anyhow!(err.to_string()))?;
            domain.destroy().ok();
            domain.undefine().ok();
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("failed to join libvirt destroy task")??
    }

    async fn fetch_console(&self, name: &str, tail: usize) -> Result<String> {
        let driver = self.clone();
        let name = name.to_string();
        spawn_blocking(move || {
            let content = driver.read_console_once(&name)?;
            let mut lines = content
                .lines()
                .map(|line| line.to_string())
                .collect::<Vec<_>>();
            if tail > 0 && lines.len() > tail {
                lines = lines.split_off(lines.len() - tail);
            }
            Ok::<_, anyhow::Error>(lines.join("\n"))
        })
        .await
        .context("failed to join libvirt console task")??
    }

    async fn stream_console(&self, name: &str) -> Result<Option<Receiver<String>>> {
        let driver = self.clone();
        let name = name.to_string();
        let (tx, rx) = mpsc::channel(256);
        spawn_blocking(move || driver.read_console_stream(&name, tx))
            .await
            .context("failed to join libvirt console stream")?
            .map_err(|err| anyhow!(err.to_string()))?;
        Ok(Some(rx))
    }
}

pub mod testing {
    use super::*;

    #[derive(Default)]
    struct DomainState {
        isolation_tier: Option<String>,
        attestation: Option<Value>,
        running: bool,
        log: Vec<String>,
    }

    #[derive(Default)]
    pub struct InMemoryLibvirtDriver {
        domains: Mutex<HashMap<String, DomainState>>,
    }

    #[async_trait]
    impl LibvirtDriver for InMemoryLibvirtDriver {
        async fn provision_domain(
            &self,
            spec: &LibvirtProvisionSpec,
        ) -> Result<LibvirtProvisionedDomain> {
            let mut guard = self.domains.lock().await;
            guard.insert(
                spec.domain_name.clone(),
                DomainState {
                    isolation_tier: spec.isolation_tier.clone(),
                    attestation: spec.attestation_hint.clone(),
                    running: false,
                    log: Vec::new(),
                },
            );
            Ok(LibvirtProvisionedDomain {
                instance_id: spec.domain_name.clone(),
                isolation_tier: spec.isolation_tier.clone(),
                attestation_evidence: spec.attestation_hint.clone(),
            })
        }

        async fn start_domain(&self, name: &str) -> Result<()> {
            let mut guard = self.domains.lock().await;
            let entry = guard
                .get_mut(name)
                .ok_or_else(|| anyhow!("domain not found: {name}"))?;
            entry.running = true;
            entry.log.push("vm-started".to_string());
            Ok(())
        }

        async fn shutdown_domain(&self, name: &str) -> Result<()> {
            let mut guard = self.domains.lock().await;
            let entry = guard
                .get_mut(name)
                .ok_or_else(|| anyhow!("domain not found: {name}"))?;
            entry.running = false;
            entry.log.push("vm-stopped".to_string());
            Ok(())
        }

        async fn destroy_domain(&self, name: &str) -> Result<()> {
            let mut guard = self.domains.lock().await;
            guard.remove(name);
            Ok(())
        }

        async fn fetch_console(&self, name: &str, tail: usize) -> Result<String> {
            let guard = self.domains.lock().await;
            let entry = guard
                .get(name)
                .ok_or_else(|| anyhow!("domain not found: {name}"))?;
            let _ = (&entry.isolation_tier, &entry.attestation);
            let logs = if tail == 0 || entry.log.len() <= tail {
                entry.log.clone()
            } else {
                entry.log[entry.log.len() - tail..].to_vec()
            };
            Ok(logs.join("\n"))
        }

        async fn stream_console(&self, name: &str) -> Result<Option<Receiver<String>>> {
            let guard = self.domains.lock().await;
            let entry = guard
                .get(name)
                .ok_or_else(|| anyhow!("domain not found: {name}"))?;
            let (tx, rx) = mpsc::channel(16);
            let lines = entry.log.clone();
            tokio::spawn(async move {
                for line in lines {
                    if tx.send(line).await.is_err() {
                        return;
                    }
                }
            });
            Ok(Some(rx))
        }
    }
}
