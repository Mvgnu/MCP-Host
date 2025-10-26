use std::convert::Infallible;
use std::sync::Arc;

// key: governance-api
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use sqlx::PgPool;
use sqlx::Row;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info};

use serde_json::Value;

use crate::extractor::AuthUser;
use crate::job_queue::{enqueue_job, Job};
use crate::servers::set_status;

use super::{
    GovernanceEngine, GovernanceError, GovernanceRunDetail, GovernanceRunStatus,
    RunStatusUpdateRequest, StartWorkflowRunRequest,
};

pub fn routes() -> Router {
    Router::new()
        .route(
            "/api/governance/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/api/governance/workflows/:id/runs",
            post(start_workflow_run),
        )
        .route("/api/governance/runs/:id", get(get_run))
        .route("/api/governance/runs/:id/status", post(update_run_status))
        .route("/api/governance/runs/:id/stream", get(stream_run))
}

async fn list_workflows(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    AuthUser { user_id, .. }: AuthUser,
) -> Result<Json<Vec<super::GovernanceWorkflow>>, (StatusCode, String)> {
    engine
        .list_workflows(&pool, user_id)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn create_workflow(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<super::CreateGovernanceWorkflow>,
) -> Result<Json<super::GovernanceWorkflow>, (StatusCode, String)> {
    engine
        .create_workflow(&pool, user_id, payload)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn start_workflow_run(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
    Json(payload): Json<StartWorkflowRunRequest>,
) -> Result<Json<GovernanceRunDetail>, (StatusCode, String)> {
    engine
        .start_workflow_run(&pool, id, user_id, payload)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn get_run(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<GovernanceRunDetail>, (StatusCode, String)> {
    engine
        .fetch_run_detail(&pool, id, user_id)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn update_run_status(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    Extension(job_tx): Extension<tokio::sync::mpsc::Sender<Job>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<RunStatusUpdateRequest>,
) -> Result<Json<GovernanceRunDetail>, (StatusCode, String)> {
    let outcome = engine
        .update_run_status(&pool, id, user_id, payload)
        .await
        .map_err(map_error)?;

    if let Some(outcome) = outcome {
        if matches!(outcome.status, GovernanceRunStatus::Completed) {
            if let Some(decision_id) = outcome.policy_decision_id {
                if let Err(err) = trigger_runtime_retry(&pool, &job_tx, decision_id, user_id).await
                {
                    error!(
                        ?err,
                        "failed to trigger runtime retry after governance completion"
                    );
                }
            }
        }
        engine
            .fetch_run_detail(&pool, outcome.run_id, user_id)
            .await
            .map(Json)
            .map_err(map_error)
    } else {
        Err((StatusCode::NOT_FOUND, "workflow run not found".into()))
    }
}

async fn stream_run(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i64>,
) -> Result<Sse<ReceiverStream<Result<Event, Infallible>>>, (StatusCode, String)> {
    let detail = engine
        .fetch_run_detail(&pool, id, user_id)
        .await
        .map_err(map_error)?;

    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let event = Event::default()
        .json_data(&detail)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if tx.send(Ok(event)).await.is_err() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "stream send failed".into(),
        ));
    }
    drop(tx);
    Ok(Sse::new(ReceiverStream::new(rx)))
}

async fn trigger_runtime_retry(
    pool: &PgPool,
    job_tx: &tokio::sync::mpsc::Sender<Job>,
    decision_id: i32,
    owner_id: i32,
) -> Result<(), sqlx::Error> {
    let rec = sqlx::query(
        r#"
        SELECT d.server_id,
               s.server_type,
               s.config,
               s.api_key,
               s.use_gpu,
               s.owner_id
        FROM runtime_policy_decisions d
        JOIN mcp_servers s ON s.id = d.server_id
        WHERE d.id = $1
        "#,
    )
    .bind(decision_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = rec else {
        return Ok(());
    };
    let server_owner: i32 = row.get("owner_id");
    if server_owner != owner_id {
        return Ok(());
    }

    let server_id: i32 = row.get("server_id");
    let server_type: String = row.get("server_type");
    let config: Option<Value> = row.get("config");
    let api_key: String = row.get("api_key");
    let use_gpu: bool = row.get("use_gpu");

    set_status(pool, server_id, "redeploying").await.ok();

    let job = Job::Start {
        server_id,
        server_type,
        config,
        api_key,
        use_gpu,
    };
    enqueue_job(pool, &job).await;
    let _ = job_tx.send(job).await;
    info!(
        server_id,
        "governance workflow completed; retriggered deployment"
    );
    Ok(())
}

fn map_error(err: GovernanceError) -> (StatusCode, String) {
    match err {
        GovernanceError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
        GovernanceError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".into()),
        GovernanceError::Database(e) => {
            error!(?e, "governance database error");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
        }
    }
}
