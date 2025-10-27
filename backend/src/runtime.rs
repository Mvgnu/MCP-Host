pub mod vm;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use chrono::Utc;
use dashmap::DashMap;
use sqlx::PgPool;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;

use crate::policy::{
    PolicyDecision, RuntimeBackend, RuntimeCapability, RuntimeExecutorDescriptor,
    RuntimePolicyEngine,
};
pub use vm::{
    AttestationVerifier, HttpHypervisorProvisioner, TpmAttestationVerifier, VirtualMachineExecutor,
    VmProvisioner,
};

#[async_trait]
pub trait ContainerRuntime: Send + Sync {
    fn spawn_server_task(
        &self,
        server_id: i32,
        server_type: String,
        config: Option<serde_json::Value>,
        api_key: String,
        use_gpu: bool,
        pool: PgPool,
    );

    fn stop_server_task(&self, server_id: i32, pool: PgPool);

    fn delete_server_task(&self, server_id: i32, pool: PgPool);

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error>;

    fn stream_logs_task(&self, server_id: i32, pool: PgPool) -> Option<Receiver<String>>;
}

#[async_trait]
pub trait RuntimeExecutor: Send + Sync {
    fn backend(&self) -> RuntimeBackend;

    fn spawn_server_task(
        &self,
        decision: PolicyDecision,
        server_id: i32,
        server_type: String,
        config: Option<serde_json::Value>,
        api_key: String,
        use_gpu: bool,
        pool: PgPool,
    );

    fn stop_server_task(&self, server_id: i32, pool: PgPool);

    fn delete_server_task(&self, server_id: i32, pool: PgPool);

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error>;

    fn stream_logs_task(&self, server_id: i32, pool: PgPool) -> Option<Receiver<String>>;
}

pub struct RuntimeOrchestrator {
    policy: Arc<RuntimePolicyEngine>,
    executors: Arc<HashMap<RuntimeBackend, Arc<dyn RuntimeExecutor>>>,
    assignments: Arc<DashMap<i32, RuntimeBackend>>,
    pool: PgPool,
}

impl RuntimeOrchestrator {
    pub fn new(
        policy: Arc<RuntimePolicyEngine>,
        pool: PgPool,
        executors: Vec<Arc<dyn RuntimeExecutor>>,
    ) -> Self {
        let mut map = HashMap::new();
        for executor in executors {
            map.insert(executor.backend(), executor);
        }
        Self {
            policy,
            executors: Arc::new(map),
            assignments: Arc::new(DashMap::new()),
            pool,
        }
    }

    fn executor_for(&self, backend: RuntimeBackend) -> Option<Arc<dyn RuntimeExecutor>> {
        self.executors.get(&backend).map(Arc::clone)
    }

    async fn resolve_backend_assignment(&self, server_id: i32) -> Option<RuntimeBackend> {
        if let Some(entry) = self.assignments.get(&server_id) {
            return Some(*entry);
        }

        match self.policy.resolve_backend_for(&self.pool, server_id).await {
            Ok(Some(backend)) => {
                self.assignments.insert(server_id, backend);
                Some(backend)
            }
            Ok(None) => None,
            Err(err) => {
                tracing::error!(?err, %server_id, "failed to resolve backend assignment");
                None
            }
        }
    }
}

#[async_trait]
impl ContainerRuntime for RuntimeOrchestrator {
    fn spawn_server_task(
        &self,
        server_id: i32,
        server_type: String,
        config: Option<serde_json::Value>,
        api_key: String,
        use_gpu: bool,
        pool: PgPool,
    ) {
        let policy = self.policy.clone();
        let executors = self.executors.clone();
        let assignments = self.assignments.clone();
        tokio::spawn(async move {
            let decision = match policy
                .decide_and_record(&pool, server_id, &server_type, config.as_ref(), use_gpu)
                .await
            {
                Ok(decision) => decision,
                Err(err) => {
                    tracing::error!(
                        ?err,
                        %server_id,
                        "failed to evaluate runtime policy for spawn",
                    );
                    if let Err(set_err) =
                        crate::servers::set_status(&pool, server_id, "error").await
                    {
                        tracing::error!(
                            ?set_err,
                            %server_id,
                            "failed to set server status after policy error",
                        );
                    }
                    return;
                }
            };

            if decision.governance_required {
                tracing::info!(
                    %server_id,
                    backend = %decision.backend.as_str(),
                    "governance workflow required before launch",
                );
                if let Err(set_err) =
                    crate::servers::set_status(&pool, server_id, "pending-governance").await
                {
                    tracing::error!(
                        ?set_err,
                        %server_id,
                        "failed to set server status after governance gate",
                    );
                }
                assignments.remove(&server_id);
                return;
            }

            if !decision.capabilities_satisfied {
                tracing::error!(
                    %server_id,
                    backend = %decision.backend.as_str(),
                    "policy decision failed capability enforcement; aborting launch",
                );
                if let Err(set_err) = crate::servers::set_status(&pool, server_id, "error").await {
                    tracing::error!(
                        ?set_err,
                        %server_id,
                        "failed to set server status after capability failure",
                    );
                }
                assignments.remove(&server_id);
                return;
            }

            let backend = decision.backend;
            let executor = match executors.get(&backend).map(Arc::clone) {
                Some(executor) => executor,
                None => {
                    tracing::error!(
                        %server_id,
                        backend = %backend.as_str(),
                        "no executor registered for backend",
                    );
                    if let Err(set_err) =
                        crate::servers::set_status(&pool, server_id, "error").await
                    {
                        tracing::error!(
                            ?set_err,
                            %server_id,
                            "failed to set server status after missing executor",
                        );
                    }
                    return;
                }
            };

            assignments.insert(server_id, backend);
            executor.spawn_server_task(
                decision,
                server_id,
                server_type,
                config,
                api_key,
                use_gpu,
                pool,
            );
        });
    }

    fn stop_server_task(&self, server_id: i32, pool: PgPool) {
        let assignments = self.assignments.clone();
        let policy = self.policy.clone();
        let executors = self.executors.clone();
        let history_pool = self.pool.clone();
        tokio::spawn(async move {
            let backend = if let Some(entry) = assignments.get(&server_id) {
                Some(*entry)
            } else {
                match policy.resolve_backend_for(&history_pool, server_id).await {
                    Ok(opt) => opt,
                    Err(err) => {
                        tracing::error!(?err, %server_id, "failed to lookup backend for stop");
                        None
                    }
                }
            };

            let Some(backend) = backend else {
                tracing::warn!(%server_id, "no backend assignment found for stop request");
                return;
            };

            if let Some(executor) = executors.get(&backend).map(Arc::clone) {
                executor.stop_server_task(server_id, pool);
            } else {
                tracing::error!(
                    %server_id,
                    backend = %backend.as_str(),
                    "stop requested but executor not registered",
                );
            }
        });
    }

    fn delete_server_task(&self, server_id: i32, pool: PgPool) {
        let assignments = self.assignments.clone();
        let policy = self.policy.clone();
        let executors = self.executors.clone();
        let history_pool = self.pool.clone();
        tokio::spawn(async move {
            let backend = if let Some((_, backend)) = assignments.remove(&server_id) {
                Some(backend)
            } else {
                match policy.resolve_backend_for(&history_pool, server_id).await {
                    Ok(opt) => opt,
                    Err(err) => {
                        tracing::error!(?err, %server_id, "failed to lookup backend for delete");
                        None
                    }
                }
            };

            let Some(backend) = backend else {
                tracing::warn!(%server_id, "no backend assignment found for delete request");
                return;
            };

            if let Some(executor) = executors.get(&backend).map(Arc::clone) {
                executor.delete_server_task(server_id, pool);
            } else {
                tracing::error!(
                    %server_id,
                    backend = %backend.as_str(),
                    "delete requested but executor not registered",
                );
            }
        });
    }

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error> {
        let backend = match self.resolve_backend_assignment(server_id).await {
            Some(backend) => backend,
            None => {
                return Err(bollard::errors::Error::IOError {
                    err: io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("no runtime executor recorded for server {server_id}"),
                    ),
                });
            }
        };

        if let Some(executor) = self.executor_for(backend) {
            executor.fetch_logs(server_id).await
        } else {
            Err(bollard::errors::Error::IOError {
                err: io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("executor for backend {} not registered", backend.as_str()),
                ),
            })
        }
    }

    fn stream_logs_task(&self, server_id: i32, pool: PgPool) -> Option<Receiver<String>> {
        let backend = self.assignments.get(&server_id).map(|entry| *entry);
        let Some(backend) = backend else {
            tracing::warn!(
                %server_id,
                "stream logs requested before backend assignment was recorded",
            );
            return None;
        };

        self.executor_for(backend)
            .and_then(|executor| executor.stream_logs_task(server_id, pool))
    }
}

pub struct DockerRuntime;

impl DockerRuntime {
    pub fn new() -> Self {
        Self
    }

    pub fn descriptor() -> RuntimeExecutorDescriptor {
        RuntimeExecutorDescriptor::new(
            RuntimeBackend::Docker,
            "Docker containers",
            [RuntimeCapability::ImageBuild],
        )
    }
}

#[async_trait]
impl RuntimeExecutor for DockerRuntime {
    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Docker
    }

    fn spawn_server_task(
        &self,
        decision: PolicyDecision,
        server_id: i32,
        _server_type: String,
        config: Option<serde_json::Value>,
        api_key: String,
        use_gpu: bool,
        pool: PgPool,
    ) {
        crate::docker::spawn_server_task(
            decision,
            server_id,
            _server_type,
            config,
            api_key,
            use_gpu,
            pool,
        );
    }

    fn stop_server_task(&self, server_id: i32, pool: PgPool) {
        crate::docker::stop_server_task(server_id, pool);
    }

    fn delete_server_task(&self, server_id: i32, pool: PgPool) {
        crate::docker::delete_server_task(server_id, pool);
    }

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error> {
        crate::docker::fetch_logs(server_id).await
    }

    fn stream_logs_task(&self, server_id: i32, pool: PgPool) -> Option<Receiver<String>> {
        crate::docker::stream_logs_task(server_id, pool)
    }
}

pub struct KubernetesRuntime {
    client: kube::Client,
}

impl KubernetesRuntime {
    pub async fn new() -> Result<Self, kube::Error> {
        let client = kube::Client::try_default().await?;
        Ok(Self { client })
    }

    pub fn descriptor() -> RuntimeExecutorDescriptor {
        RuntimeExecutorDescriptor::new(
            RuntimeBackend::Kubernetes,
            "Kubernetes clusters",
            [RuntimeCapability::ImageBuild, RuntimeCapability::Gpu],
        )
    }
}

#[async_trait]
impl RuntimeExecutor for KubernetesRuntime {
    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Kubernetes
    }

    fn spawn_server_task(
        &self,
        decision: PolicyDecision,
        server_id: i32,
        _server_type: String,
        config: Option<serde_json::Value>,
        api_key: String,
        use_gpu: bool,
        pool: PgPool,
    ) {
        use k8s_openapi::api::core::v1 as corev1;
        use kube::{
            api::{DeleteParams, PostParams},
            Api,
        };
        use std::collections::BTreeMap;

        let client = self.client.clone();
        let namespace = crate::config::K8S_NAMESPACE.clone();
        let cfg_clone = config.clone();
        tokio::spawn(async move {
            if !matches!(decision.backend, RuntimeBackend::Kubernetes) {
                tracing::warn!(
                    %server_id,
                    backend = %decision.backend.as_str(),
                    "policy selected non-kubernetes backend for kubernetes executor",
                );
            }

            let branch = cfg_clone
                .as_ref()
                .and_then(|v| v.get("branch"))
                .and_then(|v| v.as_str());

            let mut image = decision.image.clone();

            if decision.requires_build {
                let repo = cfg_clone
                    .as_ref()
                    .and_then(|v| v.get("repo_url"))
                    .and_then(|v| v.as_str());
                if repo.is_none() {
                    tracing::error!(
                        %server_id,
                        "policy requested git build but repo_url is missing for kubernetes runtime",
                    );
                    if let Err(set_err) =
                        crate::servers::set_status(&pool, server_id, "error").await
                    {
                        tracing::error!(?set_err, %server_id, "failed to set status after missing repo");
                    }
                    return;
                }
                let repo = repo.unwrap();
                if let Err(err) = crate::servers::set_status(&pool, server_id, "cloning").await {
                    tracing::error!(?err, %server_id, "failed to set status to cloning");
                }
                match crate::build::build_from_git(&pool, server_id, repo, branch).await {
                    Ok(Some(artifacts)) => {
                        let health_status = artifacts.credential_health_status.as_str();
                        let _ = sqlx::query(
                            "INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)",
                        )
                        .bind(server_id)
                        .bind(format!(
                            "Registry credential health status after build: {health_status}"
                        ))
                        .execute(&pool)
                        .await;
                        tracing::info!(
                            target: "registry.push",
                            %server_id,
                            credential_health_status = %health_status,
                            "recorded registry credential health outcome",
                        );
                        if let Some(remote_image) = artifacts.registry_image {
                            let secret_sync_required = (artifacts.auth_refresh_attempted
                                && artifacts.auth_refresh_succeeded)
                                || (artifacts.auth_rotation_attempted
                                    && artifacts.auth_rotation_succeeded);
                            if secret_sync_required {
                                if let (Some(secret_name), Some(config_path)) = (
                                    crate::config::K8S_REGISTRY_SECRET_NAME.as_ref(),
                                    crate::config::REGISTRY_AUTH_DOCKERCONFIG.as_ref(),
                                ) {
                                    match sync_image_pull_secret(
                                        client.clone(),
                                        &namespace,
                                        secret_name,
                                        config_path,
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            tracing::info!(
                                                target: "registry.push",
                                                %server_id,
                                                secret = %secret_name,
                                                "kubernetes pull secret synchronized after auth refresh",
                                            );
                                        }
                                        Err(err) => {
                                            tracing::error!(
                                                ?err,
                                                %server_id,
                                                secret = %secret_name,
                                                "failed to sync kubernetes registry secret",
                                            );
                                            let _ = sqlx::query(
                                                "INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)",
                                            )
                                            .bind(server_id)
                                            .bind("Registry credentials refreshed but Kubernetes secret sync failed")
                                            .execute(&pool)
                                            .await;
                                        }
                                    }
                                } else {
                                    tracing::warn!(
                                        %server_id,
                                        "registry credentials refreshed but K8S_REGISTRY_SECRET_NAME or REGISTRY_AUTH_DOCKERCONFIG not configured",
                                    );
                                }
                            } else if artifacts.auth_rotation_attempted
                                && !artifacts.auth_rotation_succeeded
                            {
                                let _ = sqlx::query(
                                    "INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)",
                                )
                                .bind(server_id)
                                .bind(
                                    "Proactive registry credential rotation failed; verify credentials manually",
                                )
                                .execute(&pool)
                                .await;
                            }
                            image = remote_image;
                        } else {
                            tracing::error!(
                                %server_id,
                                "kubernetes runtime requires registry push but no registry image was produced",
                            );
                            let _ = sqlx::query(
                                "INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)",
                            )
                            .bind(server_id)
                            .bind("Kubernetes runtime requires REGISTRY to be configured for git builds")
                            .execute(&pool)
                            .await;
                            if let Err(err) =
                                crate::servers::set_status(&pool, server_id, "error").await
                            {
                                tracing::error!(?err, %server_id, "failed to set error status after missing registry image");
                            }
                            return;
                        }
                    }
                    Ok(None) => {
                        return;
                    }
                    Err(err) => {
                        tracing::error!(
                            ?err,
                            %server_id,
                            "build failed to update status after git build"
                        );
                        return;
                    }
                }
            }

            let pods: Api<corev1::Pod> = Api::namespaced(client.clone(), &namespace);
            let pod_name = format!("mcp-server-{server_id}");

            let mut env_vars = vec![corev1::EnvVar {
                name: "MCP_API_KEY".into(),
                value: Some(api_key.clone()),
                ..Default::default()
            }];

            if let Some(cfg) = config.as_ref() {
                if let Some(obj) = cfg.as_object() {
                    for (k, v) in obj {
                        if k == "image" || k == "repo_url" {
                            continue;
                        }
                        env_vars.push(corev1::EnvVar {
                            name: format!("CFG_{}", k.to_uppercase()),
                            value: Some(v.to_string()),
                            ..Default::default()
                        });
                    }
                }
            }

            let storage_dir = format!("storage/{server_id}");
            if tokio::fs::create_dir_all(&storage_dir).await.is_err() {
                tracing::warn!(server_id, "failed to create storage dir");
            }

            let pod = corev1::Pod {
                metadata: kube::api::ObjectMeta {
                    name: Some(pod_name.clone()),
                    ..Default::default()
                },
                spec: Some(corev1::PodSpec {
                    containers: vec![corev1::Container {
                        name: "mcp".into(),
                        image: Some(image.clone()),
                        env: Some(env_vars),
                        volume_mounts: Some(vec![corev1::VolumeMount {
                            mount_path: "/data".into(),
                            name: "data".into(),
                            ..Default::default()
                        }]),
                        resources: {
                            use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
                            let mut limits = BTreeMap::new();
                            let mut requests = BTreeMap::new();

                            if let Some(cpu) = config
                                .as_ref()
                                .and_then(|v| v.get("cpu_limit"))
                                .and_then(|v| v.as_f64())
                            {
                                let q = Quantity(format!("{}", cpu));
                                limits.insert("cpu".into(), q.clone());
                                requests.insert("cpu".into(), q.clone());
                            }

                            if let Some(mem) = config
                                .as_ref()
                                .and_then(|v| v.get("memory_limit"))
                                .and_then(|v| v.as_u64())
                            {
                                let q = Quantity(format!("{}Mi", mem));
                                limits.insert("memory".into(), q.clone());
                                requests.insert("memory".into(), q.clone());
                            }

                            if use_gpu {
                                limits.insert("nvidia.com/gpu".into(), Quantity("1".into()));
                            }

                            if !limits.is_empty() || !requests.is_empty() {
                                Some(corev1::ResourceRequirements {
                                    limits: if limits.is_empty() {
                                        None
                                    } else {
                                        Some(limits)
                                    },
                                    requests: if requests.is_empty() {
                                        None
                                    } else {
                                        Some(requests)
                                    },
                                    ..Default::default()
                                })
                            } else {
                                None
                            }
                        },
                        ..Default::default()
                    }],
                    volumes: Some(vec![corev1::Volume {
                        name: "data".into(),
                        host_path: Some(corev1::HostPathVolumeSource {
                            path: std::fs::canonicalize(&storage_dir)
                                .unwrap_or_else(|_| std::path::PathBuf::from(&storage_dir))
                                .display()
                                .to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }]),
                    restart_policy: Some("Never".into()),
                    service_account_name: Some(crate::config::K8S_SERVICE_ACCOUNT.to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
            match pods.create(&PostParams::default(), &pod).await {
                Ok(_) => {
                    if let Err(err) = crate::servers::set_status(&pool, server_id, "starting").await
                    {
                        tracing::error!(?err, %server_id, "failed to update status to starting");
                    }
                    tracing::info!(%server_id, "kubernetes pod launched");
                }
                Err(err) => {
                    tracing::error!(?err, %server_id, "failed to create kubernetes pod");
                    if let Err(set_err) =
                        crate::servers::set_status(&pool, server_id, "error").await
                    {
                        tracing::error!(?set_err, %server_id, "failed to set status after pod error");
                    }
                }
            }
        });
    }

    fn stop_server_task(&self, server_id: i32, pool: PgPool) {
        use k8s_openapi::api::core::v1::Pod;
        use kube::{api::DeleteParams, Api};

        let client = self.client.clone();
        let namespace = crate::config::K8S_NAMESPACE.clone();
        tokio::spawn(async move {
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let pod_name = format!("mcp-server-{server_id}");
            let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
            if let Err(err) = crate::servers::set_status(&pool, server_id, "stopped").await {
                tracing::error!(?err, %server_id, "failed to set status to stopped");
            }
            let _ = crate::servers::add_metric(&pool, server_id, "stop", None).await;
            tracing::info!(%server_id, "kubernetes server stop requested");
        });
    }

    fn delete_server_task(&self, server_id: i32, pool: PgPool) {
        use k8s_openapi::api::core::v1::Pod;
        use kube::{api::DeleteParams, Api};

        let client = self.client.clone();
        let namespace = crate::config::K8S_NAMESPACE.clone();
        tokio::spawn(async move {
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let pod_name = format!("mcp-server-{server_id}");
            let _ = pods.delete(&pod_name, &DeleteParams::default()).await;
            let _ = sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
                .bind(server_id)
                .execute(&pool)
                .await;
            let _ = crate::servers::add_metric(&pool, server_id, "delete", None).await;
            let _ = tokio::fs::remove_dir_all(format!("storage/{server_id}")).await;
            tracing::info!(%server_id, "kubernetes server deleted");
        });
    }

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error> {
        use futures_util::{io::AsyncBufReadExt, StreamExt};
        use k8s_openapi::api::core::v1::Pod;
        use kube::{api::LogParams, Api};

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &crate::config::K8S_NAMESPACE);
        let stream = pods
            .log_stream(
                &format!("mcp-server-{server_id}"),
                &LogParams {
                    follow: false,
                    ..LogParams::default()
                },
            )
            .await
            .map_err(|err| bollard::errors::Error::IOError {
                err: io::Error::new(io::ErrorKind::Other, err.to_string()),
            })?;

        let mut out = String::new();
        let mut lines = stream.lines();
        while let Some(item) = lines.next().await {
            match item {
                Ok(line) => out.push_str(&line),
                Err(err) => {
                    tracing::error!(?err, %server_id, "failed to read kubernetes log chunk");
                }
            }
        }
        Ok(out)
    }

    fn stream_logs_task(&self, server_id: i32, pool: PgPool) -> Option<Receiver<String>> {
        use futures_util::{io::AsyncBufReadExt, StreamExt};
        use k8s_openapi::api::core::v1::Pod;
        use kube::{api::LogParams, Api};

        let client = self.client.clone();
        let namespace = crate::config::K8S_NAMESPACE.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let name = format!("mcp-server-{server_id}");
            match pods
                .log_stream(
                    &name,
                    &LogParams {
                        follow: true,
                        ..LogParams::default()
                    },
                )
                .await
            {
                Ok(stream) => {
                    let mut lines = stream.lines();
                    while let Some(item) = lines.next().await {
                        if let Ok(line) = item {
                            let _ = tx.send(line.clone()).await;
                            let _ = sqlx::query(
                                "INSERT INTO server_logs (server_id, log_text) VALUES ($1,$2)",
                            )
                            .bind(server_id)
                            .bind(&line)
                            .execute(&pool)
                            .await;
                        }
                    }
                }
                Err(e) => tracing::error!(?e, "k8s log stream failed"),
            }
        });
        Some(rx)
    }
}

async fn sync_image_pull_secret(
    client: kube::Client,
    namespace: &str,
    secret_name: &str,
    dockerconfig_path: &str,
) -> anyhow::Result<()> {
    use k8s_openapi::api::core::v1::Secret;
    use kube::api::{Api, Patch, PatchParams};

    let contents = tokio::fs::read(dockerconfig_path).await.map_err(|err| {
        anyhow::anyhow!(
            "failed to read docker config from {}: {}",
            dockerconfig_path,
            err
        )
    })?;
    let encoded = Base64Engine.encode(contents);
    let patch = serde_json::json!({
        "data": {
            ".dockerconfigjson": encoded,
        },
        "type": "kubernetes.io/dockerconfigjson",
        "metadata": {
            "annotations": {
                "mcp.anycontext.dev/registry-synced-at": Utc::now().to_rfc3339(),
            }
        }
    });

    let secrets: Api<Secret> = Api::namespaced(client, namespace);
    secrets
        .patch(secret_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}
