use axum::{extract::{Extension, Path}, Json};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use crate::extractor::AuthUser;
use crate::error::{AppError, AppResult};

#[derive(Serialize)]
pub struct InvocationTrace {
    pub id: i32,
    pub input_json: serde_json::Value,
    pub output_text: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn list_invocations(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> AppResult<Json<Vec<InvocationTrace>>> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await?;
    if rec.is_none() { return Err(AppError::NotFound); }
    let rows = sqlx::query(
        "SELECT id, input_json, output_text, created_at FROM invocation_traces WHERE server_id = $1 ORDER BY id DESC LIMIT 50"
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await?;
    let traces = rows.into_iter().map(|r| InvocationTrace {
        id: r.get("id"),
        input_json: r.get("input_json"),
        output_text: r.get("output_text"),
        created_at: r.get("created_at"),
    }).collect();
    Ok(Json(traces))
}

pub async fn record_invocation(
    pool: &PgPool,
    server_id: i32,
    user_id: i32,
    input_json: &serde_json::Value,
    output_text: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO invocation_traces (server_id, user_id, input_json, output_text) VALUES ($1,$2,$3,$4)"
    )
    .bind(server_id)
    .bind(user_id)
    .bind(input_json)
    .bind(output_text)
    .execute(pool)
    .await?;
    Ok(())
}
