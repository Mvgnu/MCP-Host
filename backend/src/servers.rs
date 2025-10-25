use crate::runtime::ContainerRuntime;
use crate::extractor::AuthUser;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::sse::{Event, Sse},
    Json,
};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use dashmap::DashMap;
use tokio::sync::broadcast;
use serde_json;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::convert::Infallible;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tracing::error;
use crate::error::{AppError, AppResult};
use crate::invocations::record_invocation;
use uuid::Uuid;
use reqwest;

#[derive(Serialize)]
pub struct Server {
    pub id: i32,
    pub name: String,
    pub server_type: String,
    pub status: String,
    pub use_gpu: bool,
    pub organization_id: Option<i32>,
}

#[derive(Deserialize)]
pub struct CreateServer {
    pub name: String,
    pub server_type: String,
    pub config: Option<serde_json::Value>,
    pub use_gpu: Option<bool>,
    pub organization_id: Option<i32>,
}

#[derive(Serialize)]
pub struct ServerInfo {
    pub id: i32,
    pub name: String,
    pub server_type: String,
    pub status: String,
    pub use_gpu: bool,
    pub organization_id: Option<i32>,
    pub api_key: String,
    pub webhook_secret: String,
    pub manifest: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct LogEntry {
    pub id: i32,
    pub collected_at: chrono::DateTime<chrono::Utc>,
    pub log_text: String,
}

#[derive(Serialize, Clone)]
pub struct Metric {
    pub id: i32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub event_type: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct MetricInput {
    pub event_type: String,
    pub details: Option<serde_json::Value>,
}

static METRIC_CHANNELS: Lazy<DashMap<i32, broadcast::Sender<Metric>>> = Lazy::new(DashMap::new);

#[derive(Serialize, Clone)]
pub struct StatusUpdate {
    pub id: i32,
    pub status: String,
}

static STATUS_CHANNELS: Lazy<DashMap<i32, broadcast::Sender<StatusUpdate>>> = Lazy::new(DashMap::new);

fn subscribe_status(user_id: i32) -> broadcast::Receiver<StatusUpdate> {
    use dashmap::mapref::entry::Entry;
    match STATUS_CHANNELS.entry(user_id) {
        Entry::Occupied(e) => e.get().subscribe(),
        Entry::Vacant(v) => {
            let (tx, rx) = broadcast::channel(16);
            v.insert(tx);
            rx
        }
    }
}

fn subscribe_metrics(server_id: i32) -> broadcast::Receiver<Metric> {
    use dashmap::mapref::entry::Entry;
    match METRIC_CHANNELS.entry(server_id) {
        Entry::Occupied(e) => e.get().subscribe(),
        Entry::Vacant(v) => {
            let (tx, rx) = broadcast::channel(16);
            v.insert(tx);
            rx
        }
    }
}

pub async fn set_status(pool: &PgPool, server_id: i32, status: &str) {
    if let Ok(row) = sqlx::query(
        "UPDATE mcp_servers SET status = $1 WHERE id = $2 RETURNING owner_id",
    )
    .bind(status)
    .bind(server_id)
    .fetch_one(pool)
    .await
    {
        let owner_id: i32 = row.get("owner_id");
        if let Some(tx) = STATUS_CHANNELS.get(&owner_id) {
            let _ = tx.send(StatusUpdate { id: server_id, status: status.into() });
        }
    } else {
        tracing::error!(server_id, status, "failed to update status");
    }
}

pub async fn list_servers(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, role }: AuthUser,
) -> AppResult<Json<Vec<Server>>> {
    let query = if role == "admin" {
        sqlx::query("SELECT id, name, server_type, status, use_gpu, organization_id FROM mcp_servers")
            .fetch_all(&pool)
    } else {
        sqlx::query("SELECT id, name, server_type, status, use_gpu, organization_id FROM mcp_servers WHERE owner_id = $1")
            .bind(user_id)
            .fetch_all(&pool)
    };
    let rows = query
        .await
        .map_err(|e| {
            error!(?e, "DB error listing servers");
            AppError::Db(e)
        })?;
    let servers = rows
        .into_iter()
        .map(|r| Server {
            id: r.get("id"),
            name: r.get("name"),
            server_type: r.get("server_type"),
            status: r.get("status"),
            use_gpu: r.get("use_gpu"),
            organization_id: r.try_get("organization_id").ok(),
        })
        .collect();
    Ok(Json(servers))
}

pub async fn create_server(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    AuthUser { user_id, role }: AuthUser,
    Json(payload): Json<CreateServer>,
) -> AppResult<Json<ServerInfo>> {
    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("Name is required".into()));
    }

    // enforce quota for non-admin users
    if role != "admin" {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_servers WHERE owner_id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error counting servers");
                AppError::Db(e)
            })?;
        let quota: i32 = sqlx::query_scalar("SELECT server_quota FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error fetching quota");
                AppError::Db(e)
            })?;
        if count as i32 >= quota {
            return Err(AppError::BadRequest("Server quota exceeded".into()));
        }
    }

    let api_key = Uuid::new_v4().to_string();
    let webhook_secret = Uuid::new_v4().to_string();
    let rec = sqlx::query(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key, webhook_secret, use_gpu, organization_id) \
         VALUES ($1, $2, $3, $4, 'creating', $5, $6, $7, $8) \
         RETURNING id, status, created_at",
    )
    .bind(user_id)
    .bind(&payload.name)
    .bind(&payload.server_type)
    .bind(&payload.config)
    .bind(&api_key)
    .bind(&webhook_secret)
    .bind(payload.use_gpu.unwrap_or(false))
    .bind(payload.organization_id)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error creating server");
        AppError::Db(e)
    })?;

    let id: i32 = rec.get("id");
    let status: String = rec.get("status");
    let created_at: chrono::DateTime<chrono::Utc> = rec.get("created_at");

    let info = ServerInfo {
        id,
        name: payload.name,
        server_type: payload.server_type.clone(),
        status: status.clone(),
        use_gpu: payload.use_gpu.unwrap_or(false),
        organization_id: payload.organization_id,
        api_key: api_key.clone(),
        webhook_secret: webhook_secret.clone(),
        manifest: None,
        created_at,
    };

    let job = Job::Start {
        server_id: id,
        server_type: payload.server_type,
        config: payload.config,
        api_key,
        use_gpu: payload.use_gpu.unwrap_or(false),
    };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;

    Ok(Json(info))
}

use crate::job_queue::{Job, enqueue_job};

pub async fn start_server(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<StatusCode> {
    let rec = sqlx::query(
            "SELECT server_type, config, api_key, status, use_gpu FROM mcp_servers WHERE id = $1 AND owner_id = $2"
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching server");
            AppError::Db(e)
        })?;
    let Some(rec) = rec else {
        return Err(AppError::NotFound);
    };

    let status: String = rec.get("status");
    if status == "running" {
        return Err(AppError::BadRequest("Server already running".into()));
    }

    let server_type: String = rec.get("server_type");
    let config: Option<serde_json::Value> = rec.try_get("config").ok();
    let api_key: String = rec.get("api_key");
    let use_gpu: bool = rec.get("use_gpu");

    set_status(&pool, id, "starting").await;

    let job = Job::Start {
        server_id: id,
        server_type,
        config,
        api_key,
        use_gpu,
    };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;

    Ok(StatusCode::ACCEPTED)
}

pub async fn stop_server(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<StatusCode> {
    let rec = sqlx::query("SELECT status FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching server");
            AppError::Db(e)
        })?;
    let Some(row) = rec else {
        return Err(AppError::NotFound);
    };
    let status: String = row.get("status");
    if status != "running" {
        return Err(AppError::BadRequest("Server not running".into()));
    }

    set_status(&pool, id, "stopping").await;

    let job = Job::Stop { server_id: id };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;

    Ok(StatusCode::ACCEPTED)
}

pub async fn delete_server(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<StatusCode> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching server status");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };

    let job = Job::Delete { server_id: id };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn redeploy_server(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<StatusCode> {
    let rec = sqlx::query(
            "SELECT server_type, config, api_key, use_gpu FROM mcp_servers WHERE id = $1 AND owner_id = $2",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching server");
            AppError::Db(e)
        })?;
    let Some(rec) = rec else {
        return Err(AppError::NotFound);
    };
    let server_type: String = rec.get("server_type");
    let config: Option<serde_json::Value> = rec.try_get("config").ok();
    let api_key: String = rec.get("api_key");
    let use_gpu: bool = rec.get("use_gpu");

    set_status(&pool, id, "redeploying").await;

    let job = Job::Start {
        server_id: id,
        server_type,
        config,
        api_key,
        use_gpu,
    };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;
    Ok(StatusCode::ACCEPTED)
}

pub async fn webhook_redeploy(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    Path(id): Path<i32>,
    headers: axum::http::HeaderMap,
) -> AppResult<StatusCode> {
    let rec = sqlx::query(
        "SELECT webhook_secret, server_type, config, api_key, use_gpu FROM mcp_servers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error fetching server for webhook");
        AppError::Db(e)
    })?;
    let Some(rec) = rec else {
        return Err(AppError::NotFound);
    };
    let secret: String = rec.get("webhook_secret");
    let provided = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if secret != provided {
        return Err(AppError::Unauthorized);
    }

    let server_type: String = rec.get("server_type");
    let config: Option<serde_json::Value> = rec.try_get("config").ok();
    let api_key: String = rec.get("api_key");
    let use_gpu: bool = rec.get("use_gpu");

    set_status(&pool, id, "redeploying").await;

    let job = Job::Start {
        server_id: id,
        server_type,
        config,
        api_key,
        use_gpu,
    };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;
    Ok(StatusCode::ACCEPTED)
}

/// Handle GitHub push webhooks using the stored secret for HMAC verification.
pub async fn github_webhook(
    Extension(pool): Extension<PgPool>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    Path(id): Path<i32>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> AppResult<StatusCode> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let rec = sqlx::query(
        "SELECT webhook_secret, server_type, config, api_key, use_gpu FROM mcp_servers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error fetching server for GitHub webhook");
        AppError::Db(e)
    })?;
    let Some(rec) = rec else {
        return Err(AppError::NotFound);
    };
    let secret: String = rec.get("webhook_secret");

    // Verify HMAC signature
    let sig_header = headers
        .get("x-hub-signature-256")
        .or_else(|| headers.get("x-hub-signature"))
        .ok_or(AppError::BadRequest("Missing signature".into()))?;
    let sig = sig_header
        .to_str()
        .map_err(|e| {
            error!(?e, "Signature parse error");
            AppError::BadRequest("Bad signature".into())
        })?;
    let expected = {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC can use any key length");
        mac.update(&body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    };
    if expected != sig {
        return Err(AppError::Unauthorized);
    }

    if headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        != Some("push")
    {
        return Ok(StatusCode::OK); // ignore other events
    }

    let server_type: String = rec.get("server_type");
    let config: Option<serde_json::Value> = rec.try_get("config").ok();
    let api_key: String = rec.get("api_key");
    let use_gpu: bool = rec.get("use_gpu");

    set_status(&pool, id, "redeploying").await;

    let job = Job::Start {
        server_id: id,
        server_type,
        config,
        api_key,
        use_gpu,
    };
    enqueue_job(&pool, &job).await;
    let _ = job_tx.send(job).await;
    Ok(StatusCode::ACCEPTED)
}

pub async fn server_logs(
    Extension(pool): Extension<PgPool>,
    Extension(runtime): Extension<std::sync::Arc<dyn ContainerRuntime>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<String> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };

    match runtime.fetch_logs(id).await {
        Ok(text) => {
            let _ = sqlx::query("INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)")
                .bind(id)
                .bind(&text)
                .execute(&pool)
                .await;
            Ok(text)
        }
        Err(_) => Err(AppError::Message("Failed to fetch logs".into())),
    }
}

pub async fn stored_logs(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Json<Vec<LogEntry>>> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };

    let rows = sqlx::query("SELECT id, collected_at, log_text FROM server_logs WHERE server_id = $1 ORDER BY collected_at DESC LIMIT 20")
        .bind(id)
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching logs");
            AppError::Db(e)
        })?;
    let logs = rows
        .into_iter()
        .map(|r| LogEntry {
            id: r.get("id"),
            collected_at: r.get("collected_at"),
            log_text: r.get("log_text"),
        })
        .collect();
    Ok(Json(logs))
}

pub async fn stream_logs(
    Extension(pool): Extension<PgPool>,
    Extension(runtime): Extension<std::sync::Arc<dyn ContainerRuntime>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>>
{
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };

    let Some(rx) = runtime.stream_logs_task(id, pool.clone()) else {
        return Err(AppError::Message("Docker error".into()));
    };
    let stream = ReceiverStream::new(rx).map(|line| Ok(Event::default().data(line)));
    Ok(Sse::new(stream))
}

pub async fn add_metric(
    pool: &PgPool,
    server_id: i32,
    event_type: &str,
    details: Option<&serde_json::Value>,
) -> Result<Metric, sqlx::Error> {
    let rec = sqlx::query(
        "INSERT INTO usage_metrics (server_id, event_type, details) VALUES ($1, $2, $3) RETURNING id, timestamp, event_type, details",
    )
    .bind(server_id)
    .bind(event_type)
    .bind(details)
    .fetch_one(pool)
    .await?;
    let metric = Metric {
        id: rec.get("id"),
        timestamp: rec.get("timestamp"),
        event_type: rec.get("event_type"),
        details: rec.try_get("details").ok(),
    };
    if let Some(sender) = METRIC_CHANNELS.get(&server_id) {
        let _ = sender.send(metric.clone());
    }
    Ok(metric)
}

pub async fn get_metrics(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Json<Vec<Metric>>> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };
    let rows = sqlx::query("SELECT id, timestamp, event_type, details FROM usage_metrics WHERE server_id = $1 ORDER BY timestamp DESC LIMIT 50")
        .bind(id)
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching metrics");
            AppError::Db(e)
        })?;
    let events = rows
        .into_iter()
        .map(|r| Metric {
            id: r.get("id"),
            timestamp: r.get("timestamp"),
            event_type: r.get("event_type"),
            details: r.try_get("details").ok(),
        })
        .collect();
    Ok(Json(events))
}

pub async fn post_metric(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<MetricInput>,
) -> AppResult<StatusCode> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };
    add_metric(&pool, id, &payload.event_type, payload.details.as_ref())
        .await
        .map_err(|e| {
            error!(?e, "DB error inserting metric");
            AppError::Db(e)
        })?;
    Ok(StatusCode::CREATED)
}

pub async fn stream_metrics(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(_) = rec else {
        return Err(AppError::NotFound);
    };
    let rx = subscribe_metrics(id);
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(metric) => {
                match serde_json::to_string(&metric) {
                    Ok(data) => Some(Ok(Event::default().data(data))),
                    Err(e) => {
                        tracing::error!(?e, "metric serialization failed");
                        None
                    }
                }
            }
            Err(_) => None,
        }
    });
    Ok(Sse::new(stream))
}

pub async fn stream_status(
    AuthUser { user_id, .. }: AuthUser,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = subscribe_status(user_id);
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(upd) => serde_json::to_string(&upd)
                .ok()
                .map(|d| Ok(Event::default().data(d))),
            Err(_) => None,
        }
    });
    Sse::new(stream)
}

/// Proxy a request to the running MCP server and return its response.
pub async fn invoke_server(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<serde_json::Value>,
) -> AppResult<String> {
    let rec = sqlx::query("SELECT api_key FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(rec) = rec else {
        return Err(AppError::NotFound);
    };
    let api_key: String = rec.get("api_key");

    let client = reqwest::Client::new();
    match client
        .post(format!("http://mcp-server-{id}:8080/invoke"))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&payload)
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(text) => {
                if let Err(e) = record_invocation(&pool, id, user_id, &payload, Some(&text)).await {
                    error!(?e, "failed to record invocation");
                }
                Ok(text)
            }
            Err(_) => Err(AppError::Message("Failed to read response".into())),
        },
        Err(_) => {
            if let Err(e) = record_invocation(&pool, id, user_id, &payload, None).await {
                error!(?e, "failed to record invocation");
            }
            Err(AppError::BadGateway("Container unreachable".into()))
        }
    }
}

/// Return the stored MCP manifest for a server if available.
pub async fn get_manifest(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Json<serde_json::Value>> {
    let rec = sqlx::query("SELECT manifest FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            AppError::Db(e)
        })?;
    let Some(rec) = rec else {
        return Err(AppError::NotFound);
    };
    let manifest: Option<serde_json::Value> = rec.try_get("manifest").ok();
    match manifest {
        Some(m) => Ok(Json(m)),
        None => Err(AppError::NotFound),
    }
}

/// Return a configuration snippet so agents can connect to this server easily.
pub async fn client_config(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query(
        "SELECT api_key, manifest FROM mcp_servers WHERE id = $1 AND owner_id = $2",
    )
    .bind(id)
    .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching API key");
            AppError::Db(e)
        })?;
    let Some(row) = row else {
        return Err(AppError::NotFound);
    };
    let api_key: String = row.get("api_key");
    let manifest: Option<serde_json::Value> = row.try_get("manifest").ok();

    let domain_row = sqlx::query(
        "SELECT domain FROM custom_domains WHERE server_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error fetching custom domain");
        AppError::Db(e)
    })?;
    let invoke_url = if let Some(domain_row) = domain_row {
        let domain: String = domain_row.get("domain");
        format!("https://{}/invoke", domain)
    } else {
        format!("/api/servers/{id}/invoke")
    };

    let mut obj = serde_json::Map::new();
    obj.insert("invoke_url".into(), serde_json::Value::String(invoke_url));
    obj.insert("api_key".into(), serde_json::Value::String(api_key));
    if let Some(m) = manifest {
        obj.insert("manifest".into(), m);
    }
    Ok(Json(serde_json::Value::Object(obj)))
}

/// Internal helper used by workflows to invoke a server and parse JSON output.
pub async fn invoke_server_internal(
    pool: &PgPool,
    user_id: i32,
    id: i32,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let rec = sqlx::query("SELECT api_key FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    let Some(rec) = rec else { return Err((StatusCode::NOT_FOUND, "Server not found".into())); };
    let api_key: String = rec.get("api_key");
    let client = reqwest::Client::new();
    match client
        .post(format!("http://mcp-server-{id}:8080/invoke"))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(payload)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => Ok(json),
            Err(_) => Err((StatusCode::BAD_GATEWAY, "Invalid response".into())),
        },
        Err(_) => Err((StatusCode::BAD_GATEWAY, "Container unreachable".into())),
    }
}
