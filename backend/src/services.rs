use crate::extractor::AuthUser;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use tracing::error;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Serialize)]
pub struct Service {
    pub id: i32,
    pub service_type: String,
    pub config: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateService {
    pub service_type: String,
    pub config: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct UpdateService {
    pub config: Option<serde_json::Value>,
}

pub async fn list_services(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> Result<Json<Vec<Service>>, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let rows = sqlx::query(
        "SELECT id, service_type, config, created_at FROM service_integrations WHERE server_id = $1 ORDER BY id",
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error fetching services");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let services = rows
        .into_iter()
        .map(|r| Service {
            id: r.get("id"),
            service_type: r.get("service_type"),
            config: r.try_get("config").ok(),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(services))
}

pub async fn create_service(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
    Json(payload): Json<CreateService>,
) -> Result<StatusCode, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    sqlx::query(
        "INSERT INTO service_integrations (server_id, service_type, config) VALUES ($1, $2, $3)",
    )
    .bind(server_id)
    .bind(&payload.service_type)
    .bind(&payload.config)
    .execute(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error inserting service");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    Ok(StatusCode::CREATED)
}

pub async fn update_service(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((server_id, service_id)): Path<(i32, i32)>,
    Json(payload): Json<UpdateService>,
) -> Result<StatusCode, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let result = sqlx::query(
        "UPDATE service_integrations SET config = $1 WHERE id = $2 AND server_id = $3",
    )
    .bind(&payload.config)
    .bind(service_id)
    .bind(server_id)
    .execute(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error updating service");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Service not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_service(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((server_id, service_id)): Path<(i32, i32)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let result = sqlx::query(
        "DELETE FROM service_integrations WHERE id = $1 AND server_id = $2",
    )
    .bind(service_id)
    .bind(server_id)
    .execute(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error deleting service");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Service not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
