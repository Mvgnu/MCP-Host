use crate::build;
use crate::capabilities;
use crate::proxy;
use crate::servers::{add_metric, set_status};
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use reqwest;
use serde_json::Value;
use sqlx::{PgPool, Row};
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

/// Spawn a background task to launch an MCP server container.
/// Updates the `mcp_servers` table with running/error status.
pub fn spawn_server_task(
    server_id: i32,
    server_type: String,
    config: Option<Value>,
    api_key: String,
    use_gpu: bool,
    pool: PgPool,
) {
    tokio::spawn(async move {
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
        // ensure any old container is removed so redeployments succeed
        let name = format!("mcp-server-{server_id}");
        let _ = docker
            .stop_container(&name, Some(StopContainerOptions { t: 5 }))
            .await;
        let _ = docker
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        let mut image = match server_type.as_str() {
            "PostgreSQL" => "ghcr.io/anycontext/postgres-mcp:latest".to_string(),
            "Slack" => "ghcr.io/anycontext/slack-mcp:latest".to_string(),
            "PDF Parser" => "ghcr.io/anycontext/pdf-mcp:latest".to_string(),
            "Notion" => "ghcr.io/anycontext/notion-mcp:latest".to_string(),
            "Router" => "ghcr.io/anycontext/router-mcp:latest".to_string(),
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
            set_status_with_context(&pool, server_id, "cloning", "preparing git build").await;
            if let Some(tag) = build::build_from_git(&pool, server_id, repo, branch).await {
                image = tag;
            } else {
                return;
            }
        }

        let container_name = format!("mcp-server-{server_id}");
        let create_opts = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };

        let mut env_vars = vec![format!("MCP_API_KEY={}", api_key)];
        if let Some(cfg) = config.as_ref() {
            if let Some(obj) = cfg.as_object() {
                for (k, v) in obj {
                    if k == "image" || k == "repo_url" {
                        continue;
                    }
                    env_vars.push(format!("CFG_{}={}", k.to_uppercase(), v));
                }
            }
        }

        // Attach environment variables for any service integrations
        if let Ok(rows) = sqlx::query(
            "SELECT service_type, config FROM service_integrations WHERE server_id = $1",
        )
        .bind(server_id)
        .fetch_all(&pool)
        .await
        {
            for row in rows {
                let service_type: String = row.get("service_type");
                let cfg: Option<serde_json::Value> = row.try_get("config").ok();
                match service_type.as_str() {
                    "Redis" => {
                        if let Some(obj) = cfg.as_ref().and_then(|v| v.as_object()) {
                            if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
                                env_vars.push(format!("REDIS_URL={}", url));
                            }
                        }
                    }
                    "S3" => {
                        if let Some(obj) = cfg.as_ref().and_then(|v| v.as_object()) {
                            if let Some(bucket) = obj.get("bucket").and_then(|v| v.as_str()) {
                                env_vars.push(format!("S3_BUCKET={}", bucket));
                            }
                            if let Some(region) = obj.get("region").and_then(|v| v.as_str()) {
                                env_vars.push(format!("S3_REGION={}", region));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Inject user-defined secrets
        if let Ok(rows) = sqlx::query("SELECT name, value FROM server_secrets WHERE server_id = $1")
            .bind(server_id)
            .fetch_all(&pool)
            .await
        {
            for row in rows {
                let name: String = row.get("name");
                let val: String = row.get("value");
                let value = if let Some(path) = val.strip_prefix("vault:") {
                    if let Some(vault) = crate::vault::VaultClient::from_env() {
                        match vault.read_secret(path).await {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::error!(?e, "vault read failed");
                                continue;
                            }
                        }
                    } else {
                        tracing::warn!("vault not configured");
                        continue;
                    }
                } else {
                    match sqlx::query("SELECT pgp_sym_decrypt($1::bytea, $2) as v")
                        .bind(val)
                        .bind(&crate::secrets::encryption_key())
                        .fetch_one(&pool)
                        .await
                    {
                        Ok(r) => r.get::<String, _>("v"),
                        Err(e) => {
                            tracing::error!(?e, "secret decrypt failed");
                            continue;
                        }
                    }
                };
                env_vars.push(format!("{}={}", name.to_uppercase(), value));
            }
        }

        // Prepare storage directory and mount for persistent files
        let storage_dir = format!("storage/{server_id}");
        if tokio::fs::create_dir_all(&storage_dir).await.is_err() {
            tracing::warn!("failed to create storage dir for server {server_id}");
        }
        let bind_path = match std::fs::canonicalize(&storage_dir) {
            Ok(p) => p,
            Err(_) => std::path::PathBuf::from(&storage_dir),
        };
        let volume = format!("{}:/data", bind_path.display());

        let cpu_limit = cfg_clone
            .as_ref()
            .and_then(|v| v.get("cpu_limit"))
            .and_then(|v| v.as_f64());
        let memory_limit = cfg_clone
            .as_ref()
            .and_then(|v| v.get("memory_limit"))
            .and_then(|v| v.as_u64());

        let host_cfg = HostConfig {
            auto_remove: Some(true),
            binds: Some(vec![volume]),
            device_requests: if use_gpu {
                Some(vec![bollard::models::DeviceRequest {
                    driver: Some("nvidia".into()),
                    count: Some(-1),
                    capabilities: Some(vec![vec!["gpu".into()]]),
                    ..Default::default()
                }])
            } else {
                None
            },
            nano_cpus: cpu_limit.map(|c| (c * 1_000_000_000.0) as i64),
            memory: memory_limit.map(|m| (m * 1024 * 1024) as i64),
            ..Default::default()
        };
        let container_config = ContainerConfig::<String> {
            image: Some(image.into()),
            env: Some(env_vars),
            host_config: Some(host_cfg),
            ..Default::default()
        };

        match docker
            .create_container(Some(create_opts), container_config)
            .await
        {
            Ok(container) => {
                if docker
                    .start_container(&container.id, None::<StartContainerOptions<String>>)
                    .await
                    .is_ok()
                {
                    set_status_with_context(&pool, server_id, "running", "container started").await;
                    let _ = add_metric(&pool, server_id, "start", None).await;
                    insert_log(&pool, server_id, "Container started").await;
                    proxy::rebuild_for_server(&pool, server_id).await;
                    if let Ok(resp) = reqwest::get(format!(
                        "http://mcp-server-{server_id}:8080/.well-known/mcp.json"
                    ))
                    .await
                    {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            let _ =
                                sqlx::query("UPDATE mcp_servers SET manifest = $1 WHERE id = $2")
                                    .bind(&json)
                                    .bind(server_id)
                                    .execute(&pool)
                                    .await;
                            capabilities::sync_capabilities(&pool, server_id, &json).await;
                        }
                    }
                    tracing::info!("server {server_id} started");
                    monitor_server_task(
                        server_id,
                        server_type.clone(),
                        config.clone(),
                        api_key.clone(),
                        use_gpu,
                        pool.clone(),
                    );
                } else {
                    tracing::error!("failed to start container {server_id}");
                    set_status_with_context(&pool, server_id, "error", "container start failure")
                        .await;
                    insert_log(&pool, server_id, "Failed to start container").await;
                }
            }
            Err(e) => {
                tracing::error!("container creation failed: {e:?}");
                set_status_with_context(&pool, server_id, "error", "container creation failure")
                    .await;
                insert_log(&pool, server_id, "Container creation failed").await;
            }
        }
    });
}

/// Stop the container for the specified server ID.
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
            .stop_container(&name, Some(StopContainerOptions { t: 5 }))
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
            .stop_container(&name, Some(StopContainerOptions { t: 5 }))
            .await;
        let _ = docker
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
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
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        let image = match db_type.as_str() {
            "chroma" => "ghcr.io/chroma-core/chroma:latest",
            _ => "ghcr.io/chroma-core/chroma:latest",
        };
        let create_opts = CreateContainerOptions {
            name: &name,
            platform: None,
        };
        let host_cfg = HostConfig {
            auto_remove: Some(true),
            ..Default::default()
        };
        let container_cfg = ContainerConfig::<String> {
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
                    .start_container(&info.id, None::<StartContainerOptions<String>>)
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
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
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
    use bollard::container::LogsOptions;
    use futures_util::StreamExt;

    let docker = Docker::connect_with_local_defaults()?;
    let name = format!("mcp-server-{server_id}");
    let mut stream = docker.logs(
        &name,
        Some(LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            timestamps: false,
            tail: "100".into(),
            ..Default::default()
        }),
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
        use bollard::container::LogsOptions;
        use futures_util::StreamExt;

        let mut stream = docker.logs(
            &format!("mcp-server-{server_id}"),
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                follow: true,
                timestamps: false,
                tail: "0".into(),
                ..Default::default()
            }),
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

/// Monitor a running container and restart it if it exits unexpectedly.
pub fn monitor_server_task(
    server_id: i32,
    server_type: String,
    config: Option<Value>,
    api_key: String,
    use_gpu: bool,
    pool: PgPool,
) {
    tokio::spawn(async move {
        use bollard::container::InspectContainerOptions;
        use std::time::Duration;
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to connect to Docker: {e:?}");
                return;
            }
        };
        let name = format!("mcp-server-{server_id}");
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let running = docker
                .inspect_container(&name, None::<InspectContainerOptions>)
                .await
                .ok()
                .and_then(|info| info.state)
                .and_then(|s| s.running)
                .unwrap_or(false);
            if !running {
                insert_log(&pool, server_id, "Container exited; restarting").await;
                set_status_with_context(&pool, server_id, "restarting", "container restart").await;
                let _ = add_metric(&pool, server_id, "restart", None).await;
                spawn_server_task(
                    server_id,
                    server_type.clone(),
                    config.clone(),
                    api_key.clone(),
                    use_gpu,
                    pool.clone(),
                );
                break;
            }
        }
    });
}
