use crate::extractor::AuthUser;
use axum::{extract::{Extension, Path}, http::StatusCode, Json};
use tracing::error;
use serde::Serialize;
use sqlx::{PgPool, Row};

#[derive(Serialize)]
pub struct Capability {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
}

pub async fn list_capabilities(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> Result<Json<Vec<Capability>>, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error verifying server ownership");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let rows = sqlx::query(
        "SELECT id, name, description FROM server_capabilities WHERE server_id = $1 ORDER BY id",
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error fetching capabilities");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let caps = rows
        .into_iter()
        .map(|r| Capability {
            id: r.get("id"),
            name: r.get("name"),
            description: r.try_get("description").ok(),
        })
        .collect();
    Ok(Json(caps))
}

pub async fn sync_capabilities(
    pool: &PgPool,
    server_id: i32,
    manifest: &serde_json::Value,
) {
    if let Some(caps) = manifest.get("capabilities").and_then(|v| v.as_array()) {
        if let Ok(mut tx) = pool.begin().await {
            let _ = sqlx::query("DELETE FROM server_capabilities WHERE server_id = $1")
                .bind(server_id)
                .execute(&mut *tx)
                .await;
            for cap in caps {
                if let Some(name) = cap.get("name").and_then(|v| v.as_str()) {
                    let desc = cap.get("description").and_then(|v| v.as_str());
                    let _ = sqlx::query(
                        "INSERT INTO server_capabilities (server_id, name, description) VALUES ($1, $2, $3)",
                    )
                    .bind(server_id)
                    .bind(name)
                    .bind(desc)
                    .execute(&mut *tx)
                    .await;
                }
            }
            let _ = tx.commit().await;
        }
    }
}
