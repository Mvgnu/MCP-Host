use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::mpsc::Receiver;

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

pub struct DockerRuntime;

#[async_trait]
impl ContainerRuntime for DockerRuntime {
    fn spawn_server_task(
        &self,
        server_id: i32,
        server_type: String,
        config: Option<serde_json::Value>,
        api_key: String,
        use_gpu: bool,
        pool: PgPool,
    ) {
        crate::docker::spawn_server_task(server_id, server_type, config, api_key, use_gpu, pool);
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
}

#[async_trait]
impl ContainerRuntime for KubernetesRuntime {
    fn spawn_server_task(
        &self,
        server_id: i32,
        server_type: String,
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
        tokio::spawn(async move {
            let cfg_clone = config.clone();
            let branch = cfg_clone
                .as_ref()
                .and_then(|v| v.get("branch"))
                .and_then(|v| v.as_str());

            let mut image = match server_type.as_str() {
                "PostgreSQL" => "ghcr.io/anycontext/postgres-mcp:latest".to_string(),
                "Slack" => "ghcr.io/anycontext/slack-mcp:latest".to_string(),
                "Custom" => config
                    .as_ref()
                    .and_then(|v| v.get("image"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("ghcr.io/anycontext/default-mcp:latest")
                    .to_string(),
                _ => "ghcr.io/anycontext/default-mcp:latest".to_string(),
            };

            // Build from git repo if provided
            if let Some(repo) = cfg_clone
                .as_ref()
                .and_then(|v| v.get("repo_url"))
                .and_then(|v| v.as_str())
            {
                if let Err(err) = crate::servers::set_status(&pool, server_id, "cloning").await {
                    tracing::error!(?err, %server_id, "failed to set status to cloning");
                }
                if let Some(tag) =
                    crate::build::build_from_git(&pool, server_id, repo, branch).await
                {
                    image = tag;
                } else {
                    return;
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

            let _ = pods.delete(&pod_name, &DeleteParams::default()).await; // cleanup any old pod
            match pods.create(&PostParams::default(), &pod).await {
                Ok(_) => {
                    if let Err(err) = crate::servers::set_status(&pool, server_id, "running").await
                    {
                        tracing::error!(?err, %server_id, "failed to set status to running");
                    }
                    let _ = crate::servers::add_metric(&pool, server_id, "start", None).await;
                    crate::proxy::rebuild_for_server(&pool, server_id).await;
                }
                Err(e) => {
                    tracing::error!(?e, "failed to create pod");
                    if let Err(err) = crate::servers::set_status(&pool, server_id, "error").await {
                        tracing::error!(?err, %server_id, "failed to set status to error after runtime failure");
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
            let name = format!("mcp-server-{server_id}");
            let _ = pods.delete(&name, &DeleteParams::default()).await;
            if let Err(err) = crate::servers::set_status(&pool, server_id, "stopped").await {
                tracing::error!(?err, %server_id, "failed to set status to stopped");
            }
            let _ = crate::servers::add_metric(&pool, server_id, "stop", None).await;
            crate::proxy::rebuild_for_server(&pool, server_id).await;
        });
    }

    fn delete_server_task(&self, server_id: i32, pool: PgPool) {
        use k8s_openapi::api::core::v1::Pod;
        use kube::{api::DeleteParams, Api};
        let client = self.client.clone();
        let namespace = crate::config::K8S_NAMESPACE.clone();
        tokio::spawn(async move {
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let name = format!("mcp-server-{server_id}");
            let _ = pods.delete(&name, &DeleteParams::default()).await;
            let _ = sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
                .bind(server_id)
                .execute(&pool)
                .await;
            let _ = crate::servers::add_metric(&pool, server_id, "delete", None).await;
            let _ = tokio::fs::remove_dir_all(format!("storage/{server_id}")).await;
            crate::proxy::rebuild_for_server(&pool, server_id).await;
        });
    }

    async fn fetch_logs(&self, server_id: i32) -> Result<String, bollard::errors::Error> {
        use k8s_openapi::api::core::v1::Pod;
        use kube::{api::LogParams, Api};
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &crate::config::K8S_NAMESPACE);
        let name = format!("mcp-server-{server_id}");
        match pods
            .logs(
                &name,
                &LogParams {
                    tail_lines: Some(100),
                    ..LogParams::default()
                },
            )
            .await
        {
            Ok(s) => Ok(s),
            Err(e) => Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 500,
                message: e.to_string(),
            }),
        }
    }

    fn stream_logs_task(&self, server_id: i32, pool: PgPool) -> Option<Receiver<String>> {
        use futures_util::io::AsyncBufReadExt;
        use futures_util::StreamExt;
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
                    while let Some(Ok(line)) = lines.next().await {
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
                Err(e) => tracing::error!(?e, "k8s log stream failed"),
            }
        });
        Some(rx)
    }
}
