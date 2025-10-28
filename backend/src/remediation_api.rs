use std::convert::Infallible;

use axum::{
    extract::{Extension, Path, Query},
    response::sse::{Event, Sse},
    Json,
};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio_stream::wrappers::BroadcastStream;

use crate::db::runtime_vm_remediation_artifacts::{
    list_artifacts as list_run_artifacts, RuntimeVmRemediationArtifact,
};
use crate::db::runtime_vm_remediation_playbooks::{
    create_playbook, delete_playbook, get_by_id as get_playbook_by_id,
    get_by_key as get_playbook_by_key, list_playbooks, update_playbook,
    CreateRuntimeVmRemediationPlaybook, RuntimeVmRemediationPlaybook,
    UpdateRuntimeVmRemediationPlaybook,
};
use crate::db::runtime_vm_remediation_runs::{
    ensure_remediation_run, get_run_by_id, list_runs, update_approval_state,
    EnsureRemediationRunRequest, ListRuntimeVmRemediationRuns, RuntimeVmRemediationRun,
    UpdateApprovalState,
};
use crate::error::{AppError, AppResult};
use crate::extractor::AuthUser;
use crate::remediation::subscribe_remediation_events;

// key: remediation_surface -> http-handlers
#[derive(Debug, Deserialize)]
pub struct PlaybookCreateRequest {
    pub playbook_key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_executor_type")]
    pub executor_type: String,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub sla_duration_seconds: Option<i32>,
    #[serde(default)]
    pub metadata: Value,
}

fn default_executor_type() -> String {
    "shell".to_string()
}

#[derive(Debug, Deserialize)]
pub struct PlaybookUpdateRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub executor_type: Option<String>,
    #[serde(default)]
    pub approval_required: Option<bool>,
    #[serde(default)]
    pub sla_duration_seconds: Option<Option<i32>>,
    #[serde(default)]
    pub metadata: Option<Value>,
    pub expected_version: i64,
}

#[derive(Debug, Deserialize)]
pub struct RunCreateRequest {
    pub runtime_vm_instance_id: i64,
    pub playbook: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub automation_payload: Option<Value>,
    #[serde(default)]
    pub assigned_owner_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RunApprovalRequest {
    pub new_state: String,
    #[serde(default)]
    pub approval_notes: Option<String>,
    pub expected_version: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub runtime_vm_instance_id: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub run_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunEnqueueResponse {
    pub created: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<RuntimeVmRemediationRun>,
}

pub async fn list_all_playbooks(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
) -> AppResult<Json<Vec<RuntimeVmRemediationPlaybook>>> {
    let records = list_playbooks(&pool).await?;
    Ok(Json(records))
}

pub async fn create_playbook_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Json(request): Json<PlaybookCreateRequest>,
) -> AppResult<Json<RuntimeVmRemediationPlaybook>> {
    let record = create_playbook(
        &pool,
        CreateRuntimeVmRemediationPlaybook {
            playbook_key: &request.playbook_key,
            display_name: &request.display_name,
            description: request.description.as_deref(),
            executor_type: &request.executor_type,
            owner_id: user.user_id,
            approval_required: request.approval_required,
            sla_duration_seconds: request.sla_duration_seconds,
            metadata: &request.metadata,
        },
    )
    .await?;
    Ok(Json(record))
}

pub async fn get_playbook_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(playbook_id): Path<i64>,
) -> AppResult<Json<RuntimeVmRemediationPlaybook>> {
    let Some(record) = get_playbook_by_id(&pool, playbook_id).await? else {
        return Err(AppError::NotFound);
    };
    Ok(Json(record))
}

pub async fn update_playbook_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(playbook_id): Path<i64>,
    Json(request): Json<PlaybookUpdateRequest>,
) -> AppResult<Json<RuntimeVmRemediationPlaybook>> {
    let update = UpdateRuntimeVmRemediationPlaybook {
        display_name: request.display_name.as_deref(),
        description: request.description.as_deref(),
        executor_type: request.executor_type.as_deref(),
        owner_id: None,
        approval_required: request.approval_required,
        sla_duration_seconds: request.sla_duration_seconds,
        metadata: request.metadata.as_ref(),
        expected_version: request.expected_version,
    };

    let Some(record) = update_playbook(&pool, playbook_id, update).await? else {
        return Err(AppError::Conflict("version mismatch".into()));
    };
    Ok(Json(record))
}

pub async fn delete_playbook_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(playbook_id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let removed = delete_playbook(&pool, playbook_id).await?;
    if !removed {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "deleted": true })))
}

pub async fn list_runs_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Query(query): Query<RunsQuery>,
) -> AppResult<Json<Vec<RuntimeVmRemediationRun>>> {
    let records = list_runs(
        &pool,
        ListRuntimeVmRemediationRuns {
            runtime_vm_instance_id: query.runtime_vm_instance_id,
            status: query.status.as_deref(),
        },
    )
    .await?;
    Ok(Json(records))
}

pub async fn get_run_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(run_id): Path<i64>,
) -> AppResult<Json<RuntimeVmRemediationRun>> {
    let Some(record) = get_run_by_id(&pool, run_id).await? else {
        return Err(AppError::NotFound);
    };
    Ok(Json(record))
}

pub async fn enqueue_run_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Json(request): Json<RunCreateRequest>,
) -> AppResult<Json<RunEnqueueResponse>> {
    let playbook = match get_playbook_by_key(&pool, &request.playbook).await? {
        Some(record) => record,
        None => {
            return Err(AppError::BadRequest(format!(
                "unknown playbook {}",
                request.playbook
            )))
        }
    };

    let created = ensure_remediation_run(
        &pool,
        EnsureRemediationRunRequest {
            runtime_vm_instance_id: request.runtime_vm_instance_id,
            playbook_key: &playbook.playbook_key,
            playbook_id: Some(playbook.id),
            metadata: Some(&request.metadata),
            automation_payload: request.automation_payload.as_ref(),
            approval_required: playbook.approval_required,
            assigned_owner_id: request.assigned_owner_id.or(Some(user.user_id)),
            sla_duration_seconds: playbook.sla_duration_seconds,
        },
    )
    .await?;

    if created.is_none() {
        return Err(AppError::Conflict(
            "remediation run already active for instance".into(),
        ));
    }

    Ok(Json(RunEnqueueResponse {
        created: true,
        run: created,
    }))
}

pub async fn update_approval_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(run_id): Path<i64>,
    Json(request): Json<RunApprovalRequest>,
) -> AppResult<Json<RuntimeVmRemediationRun>> {
    let new_state = match request.new_state.as_str() {
        "approved" | "rejected" => request.new_state,
        other => {
            return Err(AppError::BadRequest(format!(
                "invalid approval state {other}"
            )))
        }
    };

    let Some(record) = update_approval_state(
        &pool,
        UpdateApprovalState {
            run_id,
            new_state: &new_state,
            approval_notes: request.approval_notes.as_deref(),
            decided_at: Utc::now(),
            expected_version: request.expected_version,
        },
    )
    .await?
    else {
        return Err(AppError::Conflict("approval version mismatch".into()));
    };

    Ok(Json(record))
}

pub async fn list_artifacts_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(run_id): Path<i64>,
) -> AppResult<Json<Vec<RuntimeVmRemediationArtifact>>> {
    let records = list_run_artifacts(&pool, run_id).await?;
    Ok(Json(records))
}

pub async fn stream_remediation_events(
    Extension(_pool): Extension<PgPool>,
    _user: AuthUser,
    Query(params): Query<StreamQuery>,
) -> AppResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>> {
    let filter_run_id = params.run_id;
    let stream = BroadcastStream::new(subscribe_remediation_events()).filter_map(move |entry| {
        let filter_run_id = filter_run_id;
        async move {
            match entry {
                Ok(message) => {
                    if let Some(run_id) = filter_run_id {
                        if run_id != message.run_id {
                            return None;
                        }
                    }
                    match Event::default().json_data(&message) {
                        Ok(event) => Some(Ok(event)),
                        Err(err) => {
                            tracing::error!(?err, "failed to serialize remediation event");
                            None
                        }
                    }
                }
                Err(_) => None,
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
