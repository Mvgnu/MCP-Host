use crate::{extractor::AuthUser, proxy};
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use tracing::error;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Serialize)]
pub struct Domain {
    pub id: i32,
    pub domain: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateDomain {
    pub domain: String,
}

pub async fn list_domains(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> Result<Json<Vec<Domain>>, (StatusCode, String)> {
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
        "SELECT id, domain, created_at FROM custom_domains WHERE server_id = $1 ORDER BY id",
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error fetching domains");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let domains = rows
        .into_iter()
        .map(|r| Domain {
            id: r.get("id"),
            domain: r.get("domain"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(domains))
}

pub async fn create_domain(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
    Json(payload): Json<CreateDomain>,
) -> Result<StatusCode, (StatusCode, String)> {
    if payload.domain.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Domain required".into()));
    }
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
        "INSERT INTO custom_domains (server_id, domain) VALUES ($1, $2)",
    )
    .bind(server_id)
    .bind(&payload.domain)
    .execute(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error inserting domain");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    proxy::rebuild_for_server(&pool, server_id).await;
    Ok(StatusCode::CREATED)
}

pub async fn delete_domain(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((server_id, domain_id)): Path<(i32, i32)>,
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
        "DELETE FROM custom_domains WHERE id = $1 AND server_id = $2",
    )
    .bind(domain_id)
    .bind(server_id)
    .execute(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error deleting domain");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Domain not found".into()));
    }
    proxy::rebuild_for_server(&pool, server_id).await;
    Ok(StatusCode::NO_CONTENT)
}
