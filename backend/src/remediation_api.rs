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

use crate::db::runtime_vm_accelerator_posture::{replace_instance_posture, NewAcceleratorPosture};
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
use crate::db::runtime_vm_remediation_workspaces::{
    apply_policy_feedback, apply_promotion, apply_sandbox_simulation, apply_schema_validation,
    create_revision as create_workspace_revision, create_workspace as create_workspace_record,
    get_workspace, list_workspace_details, CreateWorkspace, CreateWorkspaceRevision,
    PolicyFeedbackUpdate, PromotionUpdate, RuntimeVmRemediationWorkspace,
    RuntimeVmRemediationWorkspaceRevision, RuntimeVmRemediationWorkspaceSandboxExecution,
    RuntimeVmRemediationWorkspaceValidationSnapshot, SandboxSimulationUpdate,
    SchemaValidationUpdate, WorkspaceDetails,
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

fn default_metadata() -> Value {
    Value::Object(serde_json::Map::new())
}

fn default_gate_context() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Serialize)]
pub struct WorkspaceGateSummary {
    pub schema_status: String,
    pub policy_status: String,
    pub simulation_status: String,
    pub promotion_status: String,
    pub policy_veto_reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceRevisionEnvelope {
    pub revision: RuntimeVmRemediationWorkspaceRevision,
    pub gate_summary: WorkspaceGateSummary,
    pub sandbox_executions: Vec<RuntimeVmRemediationWorkspaceSandboxExecution>,
    pub validation_snapshots: Vec<RuntimeVmRemediationWorkspaceValidationSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceEnvelope {
    pub workspace: RuntimeVmRemediationWorkspace,
    pub revisions: Vec<WorkspaceRevisionEnvelope>,
}

impl From<WorkspaceDetails> for WorkspaceEnvelope {
    fn from(details: WorkspaceDetails) -> Self {
        let workspace = details.workspace;
        let revisions = details
            .revisions
            .into_iter()
            .map(|revision_details| {
                let gate_summary = WorkspaceGateSummary {
                    schema_status: revision_details.revision.schema_status.clone(),
                    policy_status: revision_details.revision.policy_status.clone(),
                    simulation_status: revision_details.revision.simulation_status.clone(),
                    promotion_status: revision_details.revision.promotion_status.clone(),
                    policy_veto_reasons: revision_details.revision.policy_veto_reasons.clone(),
                };
                WorkspaceRevisionEnvelope {
                    revision: revision_details.revision,
                    gate_summary,
                    sandbox_executions: revision_details.sandbox_executions,
                    validation_snapshots: revision_details.validation_snapshots,
                }
            })
            .collect();
        WorkspaceEnvelope {
            workspace,
            revisions,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceCreateRequest {
    pub workspace_key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub plan: Value,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
    #[serde(default)]
    pub lineage_tags: Vec<String>,
    #[serde(default)]
    pub lineage_labels: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRevisionCreateRequest {
    pub plan: Value,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
    #[serde(default)]
    pub lineage_labels: Vec<String>,
    pub expected_workspace_version: i64,
    #[serde(default)]
    pub previous_revision_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceSchemaValidationRequest {
    pub result_status: String,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default = "default_gate_context")]
    pub gate_context: Value,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
    pub expected_revision_version: i64,
}

#[derive(Debug, Deserialize)]
pub struct WorkspacePolicyFeedbackRequest {
    pub policy_status: String,
    #[serde(default)]
    pub veto_reasons: Vec<String>,
    #[serde(default = "default_gate_context")]
    pub gate_context: Value,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
    pub expected_revision_version: i64,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceSimulationRequest {
    pub simulator_kind: String,
    pub execution_state: String,
    #[serde(default = "default_gate_context")]
    pub gate_context: Value,
    #[serde(default)]
    pub diff_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Value,
    pub expected_revision_version: i64,
}

#[derive(Debug, Deserialize)]
pub struct WorkspacePromotionRequest {
    pub promotion_status: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub expected_workspace_version: i64,
    pub expected_revision_version: i64,
}

pub async fn list_all_playbooks(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
) -> AppResult<Json<Vec<RuntimeVmRemediationPlaybook>>> {
    let records = list_playbooks(&pool).await?;
    Ok(Json(records))
}

pub async fn list_workspaces_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
) -> AppResult<Json<Vec<WorkspaceEnvelope>>> {
    let records = list_workspace_details(&pool).await?;
    let payload = records.into_iter().map(WorkspaceEnvelope::from).collect();
    Ok(Json(payload))
}

pub async fn create_workspace_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Json(request): Json<WorkspaceCreateRequest>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let lineage_tags: Vec<&str> = request.lineage_tags.iter().map(String::as_str).collect();
    let lineage_labels: Vec<&str> = request.lineage_labels.iter().map(String::as_str).collect();

    let details = create_workspace_record(
        &pool,
        CreateWorkspace {
            workspace_key: &request.workspace_key,
            display_name: &request.display_name,
            description: request.description.as_deref(),
            owner_id: user.user_id,
            plan: &request.plan,
            metadata: Some(&request.metadata),
            lineage_tags: &lineage_tags,
            lineage_labels: &lineage_labels,
        },
    )
    .await?;

    Ok(Json(WorkspaceEnvelope::from(details)))
}

pub async fn get_workspace_handler(
    Extension(pool): Extension<PgPool>,
    _user: AuthUser,
    Path(workspace_id): Path<i64>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let Some(details) = get_workspace(&pool, workspace_id).await? else {
        return Err(AppError::NotFound);
    };
    Ok(Json(WorkspaceEnvelope::from(details)))
}

pub async fn create_workspace_revision_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspaceRevisionCreateRequest>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let lineage_labels: Vec<&str> = request.lineage_labels.iter().map(String::as_str).collect();

    let result = create_workspace_revision(
        &pool,
        CreateWorkspaceRevision {
            workspace_id,
            previous_revision_id: request.previous_revision_id,
            created_by: user.user_id,
            plan: &request.plan,
            metadata: Some(&request.metadata),
            lineage_labels: &lineage_labels,
            expected_workspace_version: request.expected_workspace_version,
        },
    )
    .await?;

    let envelope = map_workspace_update_result(&pool, workspace_id, None, result).await?;
    Ok(Json(envelope))
}

pub async fn apply_workspace_schema_validation_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Path((workspace_id, revision_id)): Path<(i64, i64)>,
    Json(request): Json<WorkspaceSchemaValidationRequest>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let errors: Vec<&str> = request.errors.iter().map(String::as_str).collect();

    let result = apply_schema_validation(
        &pool,
        SchemaValidationUpdate {
            workspace_id,
            revision_id,
            validator_id: user.user_id,
            result_status: &request.result_status,
            errors: &errors,
            gate_context: &request.gate_context,
            metadata: Some(&request.metadata),
            expected_revision_version: request.expected_revision_version,
        },
    )
    .await?;

    let envelope =
        map_workspace_update_result(&pool, workspace_id, Some(revision_id), result).await?;
    Ok(Json(envelope))
}

pub async fn apply_workspace_policy_feedback_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Path((workspace_id, revision_id)): Path<(i64, i64)>,
    Json(request): Json<WorkspacePolicyFeedbackRequest>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let veto_reasons: Vec<&str> = request.veto_reasons.iter().map(String::as_str).collect();

    let result = apply_policy_feedback(
        &pool,
        PolicyFeedbackUpdate {
            workspace_id,
            revision_id,
            reviewer_id: user.user_id,
            policy_status: &request.policy_status,
            veto_reasons: &veto_reasons,
            gate_context: &request.gate_context,
            metadata: Some(&request.metadata),
            expected_revision_version: request.expected_revision_version,
        },
    )
    .await?;

    let envelope =
        map_workspace_update_result(&pool, workspace_id, Some(revision_id), result).await?;
    Ok(Json(envelope))
}

pub async fn apply_workspace_simulation_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Path((workspace_id, revision_id)): Path<(i64, i64)>,
    Json(request): Json<WorkspaceSimulationRequest>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let result = apply_sandbox_simulation(
        &pool,
        SandboxSimulationUpdate {
            workspace_id,
            revision_id,
            simulator_kind: &request.simulator_kind,
            requested_by: user.user_id,
            execution_state: &request.execution_state,
            gate_context: &request.gate_context,
            diff_snapshot: request.diff_snapshot.as_ref(),
            metadata: Some(&request.metadata),
            expected_revision_version: request.expected_revision_version,
        },
    )
    .await?;

    let envelope =
        map_workspace_update_result(&pool, workspace_id, Some(revision_id), result).await?;
    Ok(Json(envelope))
}

pub async fn apply_workspace_promotion_handler(
    Extension(pool): Extension<PgPool>,
    user: AuthUser,
    Path((workspace_id, revision_id)): Path<(i64, i64)>,
    Json(request): Json<WorkspacePromotionRequest>,
) -> AppResult<Json<WorkspaceEnvelope>> {
    let notes: Vec<&str> = request.notes.iter().map(String::as_str).collect();

    let result = apply_promotion(
        &pool,
        PromotionUpdate {
            workspace_id,
            revision_id,
            requested_by: user.user_id,
            promotion_status: &request.promotion_status,
            notes: &notes,
            expected_workspace_version: request.expected_workspace_version,
            expected_revision_version: request.expected_revision_version,
        },
    )
    .await?;

    let envelope =
        map_workspace_update_result(&pool, workspace_id, Some(revision_id), result).await?;
    Ok(Json(envelope))
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

    if let Some(run) = created.as_ref() {
        ingest_accelerator_posture(&pool, run.runtime_vm_instance_id, &request.metadata).await?;
    }

    Ok(Json(RunEnqueueResponse {
        created: true,
        run: created,
    }))
}

#[derive(Debug, Clone)]
struct AcceleratorSpec {
    accelerator_id: String,
    accelerator_type: String,
    posture: String,
    policy_feedback: Vec<String>,
    metadata: Value,
}

fn extract_accelerator_specs(metadata: &Value) -> Vec<AcceleratorSpec> {
    let mut specs = Vec::new();
    let Some(entries) = metadata
        .get("accelerators")
        .and_then(|value| value.as_array())
    else {
        return specs;
    };

    for entry in entries {
        let Some(accelerator_id) = entry.get("id").and_then(|value| value.as_str()) else {
            continue;
        };
        let accelerator_type = entry
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let posture = entry
            .get("posture")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let policy_feedback = entry
            .get("policy_feedback")
            .and_then(|value| value.as_array())
            .map(|feedback| {
                feedback
                    .iter()
                    .filter_map(|item| item.as_str().map(|value| value.trim().to_string()))
                    .filter(|value| !value.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let metadata_payload = entry
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| entry.clone());

        specs.push(AcceleratorSpec {
            accelerator_id: accelerator_id.to_string(),
            accelerator_type: accelerator_type.to_string(),
            posture: posture.to_string(),
            policy_feedback,
            metadata: metadata_payload,
        });
    }

    specs
}

async fn ingest_accelerator_posture(
    pool: &PgPool,
    runtime_vm_instance_id: i64,
    metadata: &Value,
) -> AppResult<()> {
    let specs = extract_accelerator_specs(metadata);
    if specs.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let upserts: Vec<NewAcceleratorPosture> = specs
        .iter()
        .map(|spec| NewAcceleratorPosture {
            runtime_vm_instance_id,
            accelerator_id: spec.accelerator_id.as_str(),
            accelerator_type: spec.accelerator_type.as_str(),
            posture: spec.posture.as_str(),
            policy_feedback: spec.policy_feedback.as_slice(),
            metadata: &spec.metadata,
        })
        .collect();

    replace_instance_posture(&mut tx, runtime_vm_instance_id, &upserts).await?;
    tx.commit().await?;
    Ok(())
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

async fn map_workspace_update_result(
    pool: &PgPool,
    workspace_id: i64,
    revision_id: Option<i64>,
    result: Option<WorkspaceDetails>,
) -> AppResult<WorkspaceEnvelope> {
    if let Some(details) = result {
        return Ok(WorkspaceEnvelope::from(details));
    }

    let Some(existing) = get_workspace(pool, workspace_id).await? else {
        return Err(AppError::NotFound);
    };

    if let Some(revision_id) = revision_id {
        let has_revision = existing
            .revisions
            .iter()
            .any(|item| item.revision.id == revision_id);
        if has_revision {
            return Err(AppError::Conflict("version mismatch".into()));
        }
        return Err(AppError::NotFound);
    }

    Err(AppError::Conflict("workspace version mismatch".into()))
}
