use crate::capabilities;
use crate::policy::{PolicyDecision, RuntimeBackend};
use crate::proxy;
use crate::servers::{add_metric, set_status};
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    StopContainerOptionsBuilder,
};
use bollard::Docker;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::mpsc::{self, Receiver};

async fn insert_log(pool: &PgPool, server_id: i32, text: &str) {
    let _ = sqlx::query("INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)")
        .bind(server_id)
        .bind(text)
        .execute(pool)
        .await;
}

async fn set_status_with_context(pool: &PgPool, server_id: i32, status: &str, context: &str) {
    if let Err(err) = set_status(pool, server_id, status).await {
        tracing::error!(
            ?err,
            %server_id,
            status = %status,
            context,
            "failed to update server status"
        );
    }
}

fn build_env_vars(api_key: &str, config: Option<&Value>) -> Vec<String> {
    let mut env = vec![format!("MCP_API_KEY={}", api_key)];
    if let Some(cfg) = config {
        if let Some(obj) = cfg.as_object() {
            for (key, value) in obj {
                if key == "image" || key == "repo_url" {
                    continue;
                }
                env.push(format!("CFG_{}={}", key.to_uppercase(), value));
            }
        }
    }
    env
}

/// Spawn a background task to launch an MCP server container.
/// Updates the `mcp_servers` table with running/error status.
pub fn spawn_server_task(
    decision: PolicyDecision,
    server_id: i32,
    _server_type: String,
    config: Option<Value>,
    api_key: String,
    use_gpu: bool,
    pool: PgPool,
) {
    tokio::spawn(async move {
        if !matches!(decision.backend, RuntimeBackend::Docker) {
            tracing::warn!(
                %server_id,
                backend = %decision.backend.as_str(),
                "runtime policy selected a backend different from docker runtime",
            );
        }

        let cfg_clone = config.clone();
        let branch = cfg_clone
            .as_ref()
            .and_then(|v| v.get("branch"))
            .and_then(|v| v.as_str());
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to connect to Docker: {e:?}");
                set_status_with_context(&pool, server_id, "error", "initial docker connection")
                    .await;
                return;
            }
        };
        let name = format!("mcp-server-{server_id}");
        let _ = docker
            .stop_container(
                &name,
                Some(StopContainerOptionsBuilder::default().t(5).build()),
            )
            .await;
        let _ = docker
            .remove_container(
                &name,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;

        let mut image = decision.image.clone();

        if decision.requires_build {
            let repo = cfg_clone
                .as_ref()
                .and_then(|v| v.get("repo_url"))
                .and_then(|v| v.as_str());
            if repo.is_none() {
                tracing::error!(
                    %server_id,
                    "policy requested git build but repo_url is missing",
                );
                set_status_with_context(
                    &pool,
                    server_id,
                    "error",
                    "runtime policy build precondition",
                )
                .await;
                return;
            }
            let repo = repo.unwrap();
            if let Err(err) = set_status(&pool, server_id, "cloning").await {
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
                        image = remote_image;
                    } else {
                        tracing::warn!(
                            %server_id,
                            "build succeeded but no registry image available; using local image",
                        );
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

        let create_opts = CreateContainerOptionsBuilder::default().name(&name).build();
        let mut host_cfg = HostConfig {
            auto_remove: Some(true),
            ..Default::default()
        };
        if use_gpu {
            host_cfg.device_requests = Some(vec![bollard::models::DeviceRequest {
                driver: Some("nvidia".into()),
                count: Some(1),
                capabilities: Some(vec![vec!["gpu".into(), "nvidia".into()]]),
                ..Default::default()
            }]);
        }
        let container_cfg = ContainerCreateBody {
            image: Some(image.clone()),
            host_config: Some(host_cfg),
            env: Some(build_env_vars(&api_key, config.as_ref())),
            ..Default::default()
        };
        match docker
            .create_container(Some(create_opts), container_cfg)
            .await
        {
            Ok(info) => {
                if docker
                    .start_container(
                        &info.id,
                        None::<bollard::query_parameters::StartContainerOptions>,
                    )
                    .await
                    .is_ok()
                {
                    insert_log(
                        &pool,
                        server_id,
                        &format!("Container started with image {image}"),
                    )
                    .await;
                    let _ = set_status(&pool, server_id, "running").await;
                    let _ = add_metric(&pool, server_id, "start", None).await;
                    if let Some(cfg) = config.as_ref() {
                        capabilities::sync_capabilities(&pool, server_id, cfg).await;
                    }
                    proxy::rebuild_for_server(&pool, server_id).await;
                } else {
                    tracing::error!(
                        %server_id,
                        "docker container failed to start",
                    );
                    set_status_with_context(&pool, server_id, "error", "start failure").await;
                }
            }
            Err(e) => {
                tracing::error!(?e, %server_id, "Failed to create container");
                set_status_with_context(&pool, server_id, "error", "create failure").await;
            }
        }
    });
}
/// Stop a running container and update status/metrics.
pub fn stop_server_task(server_id: i32, pool: PgPool) {
    tokio::spawn(async move {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to connect to Docker: {e:?}");
                return;
            }
        };

        let name = format!("mcp-server-{server_id}");
        let _ = docker
            .stop_container(
                &name,
                Some(StopContainerOptionsBuilder::default().t(5).build()),
            )
            .await;

        set_status_with_context(&pool, server_id, "stopped", "container stopped").await;
        let _ = add_metric(&pool, server_id, "stop", None).await;
        insert_log(&pool, server_id, "Container stopped").await;
        proxy::rebuild_for_server(&pool, server_id).await;
        tracing::info!("server {server_id} stopped");
    });
}

/// Remove the container and delete the database record.
pub fn delete_server_task(server_id: i32, pool: PgPool) {
    tokio::spawn(async move {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to connect to Docker: {e:?}");
                return;
            }
        };

        let name = format!("mcp-server-{server_id}");
        let _ = docker
            .stop_container(
                &name,
                Some(StopContainerOptionsBuilder::default().t(5).build()),
            )
            .await;
        let _ = docker
            .remove_container(
                &name,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;

        let _ = sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
            .bind(server_id)
            .execute(&pool)
            .await;
        let _ = add_metric(&pool, server_id, "delete", None).await;
        let _ = tokio::fs::remove_dir_all(format!("storage/{server_id}")).await;
        insert_log(&pool, server_id, "Server deleted").await;
        proxy::rebuild_for_server(&pool, server_id).await;
        tracing::info!("server {server_id} deleted");
    });
}

/// Spawn a simple Chroma vector database container.
pub fn spawn_vector_db_task(id: i32, db_type: String, pool: PgPool) {
    tokio::spawn(async move {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(?e, "Failed to connect to Docker");
                return;
            }
        };
        let name = format!("mcp-vectordb-{id}");
        let _ = docker
            .remove_container(
                &name,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        let image = match db_type.as_str() {
            "chroma" => "ghcr.io/chroma-core/chroma:latest",
            _ => "ghcr.io/chroma-core/chroma:latest",
        };
        let create_opts = CreateContainerOptionsBuilder::default().name(&name).build();
        let host_cfg = HostConfig {
            auto_remove: Some(true),
            ..Default::default()
        };
        let container_cfg = ContainerCreateBody {
            image: Some(image.into()),
            host_config: Some(host_cfg),
            ..Default::default()
        };
        match docker
            .create_container(Some(create_opts), container_cfg)
            .await
        {
            Ok(info) => {
                if docker
                    .start_container(
                        &info.id,
                        None::<bollard::query_parameters::StartContainerOptions>,
                    )
                    .await
                    .is_ok()
                {
                    let url = format!("http://{name}:8000");
                    let _ = sqlx::query(
                        "UPDATE vector_dbs SET container_id = $1, url = $2 WHERE id = $3",
                    )
                    .bind(&info.id)
                    .bind(&url)
                    .bind(id)
                    .execute(&pool)
                    .await;
                } else {
                    tracing::error!("failed to start vector db container {id}");
                }
            }
            Err(e) => {
                tracing::error!(?e, "failed to create vector db container");
            }
        }
    });
}

pub fn delete_vector_db_task(id: i32, pool: PgPool) {
    tokio::spawn(async move {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(?e, "Failed to connect to Docker");
                return;
            }
        };
        let name = format!("mcp-vectordb-{id}");
        let _ = docker
            .remove_container(
                &name,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        let _ = sqlx::query("DELETE FROM vector_dbs WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await;
        let _ = tokio::fs::remove_dir_all(format!("storage/vector-{id}")).await;
    });
}

/// Fetch the latest logs for a container.
pub async fn fetch_logs(server_id: i32) -> Result<String, bollard::errors::Error> {
    use futures_util::StreamExt;

    let docker = Docker::connect_with_local_defaults()?;
    let name = format!("mcp-server-{server_id}");
    let mut stream = docker.logs(
        &name,
        Some(
            LogsOptionsBuilder::default()
                .stdout(true)
                .stderr(true)
                .follow(false)
                .timestamps(false)
                .tail("100")
                .build(),
        ),
    );

    let mut out = String::new();
    while let Some(item) = stream.next().await {
        if let Ok(chunk) = item {
            out.push_str(&chunk.to_string());
        }
    }
    Ok(out)
}

/// Start streaming logs for a container. Returns a channel receiver with lines.
pub fn stream_logs_task(server_id: i32, pool: PgPool) -> Option<Receiver<String>> {
    let (tx, rx) = mpsc::channel(16);
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to connect to Docker: {e:?}");
            return None;
        }
    };

    tokio::spawn(async move {
        use futures_util::StreamExt;

        let mut stream = docker.logs(
            &format!("mcp-server-{server_id}"),
            Some(
                LogsOptionsBuilder::default()
                    .stdout(true)
                    .stderr(true)
                    .follow(true)
                    .timestamps(false)
                    .tail("0")
                    .build(),
            ),
        );

        while let Some(item) = stream.next().await {
            if let Ok(chunk) = item {
                let line = chunk.to_string();
                let _ = tx.send(line.clone()).await;
                let _ =
                    sqlx::query("INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)")
                        .bind(server_id)
                        .bind(&line)
                        .execute(&pool)
                        .await;
            }
        }
    });

    Some(rx)
}
