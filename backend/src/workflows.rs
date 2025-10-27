use crate::extractor::AuthUser;
use crate::servers::invoke_server_internal; // internal helper
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tracing::error;

#[derive(Serialize)]
pub struct Workflow {
    pub id: i32,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateWorkflow {
    pub name: String,
    pub steps: Vec<i32>, // server ids
}

pub async fn list_workflows(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> Result<Json<Vec<Workflow>>, (StatusCode, String)> {
    let rows =
        sqlx::query("SELECT id, name, created_at FROM workflows WHERE owner_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error listing workflows");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
            })?;
    let wf = rows
        .into_iter()
        .map(|r| Workflow {
            id: r.get("id"),
            name: r.get("name"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(wf))
}

pub async fn create_workflow(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<CreateWorkflow>,
) -> Result<Json<Workflow>, (StatusCode, String)> {
    let rec = sqlx::query(
        "INSERT INTO workflows (owner_id, name) VALUES ($1,$2) RETURNING id, created_at",
    )
    .bind(user_id)
    .bind(&payload.name)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error creating workflow");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let wf_id: i32 = rec.get("id");
    for (pos, sid) in payload.steps.iter().enumerate() {
        if let Err(e) = sqlx::query(
            "INSERT INTO workflow_steps (workflow_id, position, server_id) VALUES ($1,$2,$3)",
        )
        .bind(wf_id)
        .bind((pos + 1) as i32)
        .bind(*sid)
        .execute(&pool)
        .await
        {
            error!(?e, "DB error inserting step");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()));
        }
    }
    Ok(Json(Workflow {
        id: wf_id,
        name: payload.name,
        created_at: rec.get("created_at"),
    }))
}

pub async fn delete_workflow(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    let res = sqlx::query("DELETE FROM workflows WHERE id=$1 AND owner_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error deleting workflow");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if res.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Workflow not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct InvokeInput {
    pub input: serde_json::Value,
}

pub async fn invoke_workflow(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<InvokeInput>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let steps =
        sqlx::query("SELECT server_id FROM workflow_steps WHERE workflow_id=$1 ORDER BY position")
            .bind(id)
            .fetch_all(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error fetching steps");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
            })?;
    if steps.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Workflow has no steps".into()));
    }
    let mut data = payload.input;
    for row in steps {
        let sid: i32 = row.get("server_id");
        // call server internally; assumes same user ownership enforced in invoke_server_internal
        match invoke_server_internal(&pool, user_id, sid, &data).await {
            Ok(out) => data = out,
            Err(e) => return Err(e),
        }
    }
    Ok(Json(data))
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/workflows", get(list_workflows).post(create_workflow))
        .route("/api/workflows/:id", delete(delete_workflow))
        .route("/api/workflows/:id/invoke", post(invoke_workflow))
}
