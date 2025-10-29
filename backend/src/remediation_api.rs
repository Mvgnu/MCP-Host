use std::collections::HashSet;
use std::convert::Infallible;

use axum::{
    extract::{Extension, Path, Query},
    response::sse::{Event, Sse},
    Json,
};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
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
    ensure_remediation_run, get_active_run_for_instance, get_run_by_id, list_runs,
    update_approval_state, update_run_workspace_linkage, EnsureRemediationRunRequest,
    ListRuntimeVmRemediationRuns, RuntimeVmRemediationRun, UpdateApprovalState,
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
use crate::remediation::{
    broadcast_promotion_refresh, subscribe_remediation_events, PromotionAutomationRefresh,
};
use tracing::{trace, warn};

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
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub workspace_revision_id: Option<i64>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promotion_runs: Vec<RuntimeVmRemediationRun>,
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
            promotion_runs: Vec::new(),
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
    #[serde(default = "default_gate_context")]
    pub gate_context: Value,
    pub expected_workspace_version: i64,
    pub expected_revision_version: i64,
}

#[derive(Debug, Clone)]
struct PromotionAutomationTarget {
    instance_id: i64,
    playbook_key: Option<String>,
    target_snapshot: Value,
    automation_payload: Option<Value>,
}

fn parse_instance_id(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.parse::<i64>().ok(),
        _ => None,
    }
}

fn collect_target_entries(value: Option<&Value>, allow_root_targets: bool) -> Vec<Value> {
    let mut targets = Vec::new();
    if let Some(value) = value {
        let context = Map::new();
        collect_target_entries_recursive(value, &context, &mut targets, allow_root_targets);
    }
    targets
}

fn collect_target_entries_recursive(
    value: &Value,
    context: &Map<String, Value>,
    targets: &mut Vec<Value>,
    allow_current: bool,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_target_entries_recursive(item, context, targets, true);
            }
        }
        Value::Object(map) => {
            let has_instance =
                map.contains_key("runtime_vm_instance_id") || map.contains_key("instance_id");

            if has_instance && allow_current {
                let mut snapshot = Map::new();
                for (key, value) in map {
                    snapshot.insert(key.clone(), value.clone());
                }
                for (key, value) in context {
                    snapshot.entry(key.clone()).or_insert_with(|| value.clone());
                }
                targets.push(Value::Object(snapshot));
                return;
            }

            let mut next_context = context.clone();

            for (key, child) in map {
                if key == "targets" {
                    continue;
                }
                if should_propagate_context(child) && !next_context.contains_key(key) {
                    next_context.insert(key.clone(), child.clone());
                }
            }

            if let Some(targets_value) = map.get("targets") {
                collect_target_entries_recursive(targets_value, &next_context, targets, true);
            }

            for (key, child) in map {
                if key == "targets" {
                    continue;
                }
                collect_target_entries_recursive(child, &next_context, targets, true);
            }
        }
        _ => {}
    }
}

fn should_propagate_context(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

fn extract_promotion_targets(
    workspace: &RuntimeVmRemediationWorkspace,
    revision: &RuntimeVmRemediationWorkspaceRevision,
) -> Vec<PromotionAutomationTarget> {
    let mut entries = Vec::new();
    entries.extend(collect_target_entries(revision.plan.get("targets"), true));
    entries.extend(collect_target_entries(workspace.metadata.get("targets"), false));
    entries.extend(collect_target_entries(revision.metadata.get("targets"), false));

    let mut deduped_entries = Vec::new();
    let mut seen_instance_ids = HashSet::new();

    for entry in entries {
        let instance_id = entry
            .get("runtime_vm_instance_id")
            .or_else(|| entry.get("instance_id"))
            .and_then(parse_instance_id);

        if let Some(id) = instance_id {
            if seen_instance_ids.insert(id) {
                deduped_entries.push(entry);
            }
        } else {
            deduped_entries.push(entry);
        }
    }

    if deduped_entries.is_empty() {
        if let Some(id) = revision
            .metadata
            .get("runtime_vm_instance_id")
            .and_then(parse_instance_id)
            .or_else(|| {
                workspace
                    .metadata
                    .get("runtime_vm_instance_id")
                    .and_then(parse_instance_id)
            })
        {
            deduped_entries.push(json!({
                "runtime_vm_instance_id": id,
                "source": "workspace-default",
            }));
        }
    }

    let default_playbook = revision
        .plan
        .get("playbooks")
        .and_then(|value| value.as_array())
        .and_then(|array| array.first())
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    let mut targets = Vec::new();
    trace!(target_count = deduped_entries.len(), "processing promotion targets");

    for entry in deduped_entries {
        let instance_id = entry
            .get("runtime_vm_instance_id")
            .or_else(|| entry.get("instance_id"))
            .and_then(parse_instance_id);

        let Some(instance_id) = instance_id else {
            warn!(target_entry = ?entry, "skipping workspace promotion target without runtime_vm_instance_id");
            continue;
        };

        let playbook_key = entry
            .get("playbook")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| default_playbook.clone());

        let automation_payload = entry
            .get("automation_payload")
            .cloned()
            .or_else(|| entry.get("payload").cloned());

        let playbook_for_log = playbook_key.clone();

        targets.push(PromotionAutomationTarget {
            instance_id,
            playbook_key,
            target_snapshot: entry,
            automation_payload,
        });

        trace!(instance_id, playbook = ?playbook_for_log, "workspace promotion target parsed");
    }

    targets
}

fn build_promotion_metadata(
    workspace: &RuntimeVmRemediationWorkspace,
    revision: &RuntimeVmRemediationWorkspaceRevision,
    target_snapshot: &Value,
    gate_context: &Value,
    notes: &[String],
    requested_by: i32,
    existing_metadata: Option<&Value>,
) -> Value {
    let mut metadata = json!({
        "workspace": {
            "id": workspace.id,
            "key": workspace.workspace_key,
            "display_name": workspace.display_name,
            "owner_id": workspace.owner_id,
            "lineage_tags": workspace.lineage_tags.clone(),
            "lineage_labels": revision.lineage_labels.clone(),
            "metadata": workspace.metadata.clone(),
        },
        "revision": {
            "id": revision.id,
            "number": revision.revision_number,
            "plan": revision.plan.clone(),
            "metadata": revision.metadata.clone(),
        },
        "promotion": {
            "notes": notes.to_vec(),
            "gate_context": gate_context.clone(),
            "requested_by": requested_by,
            "recorded_at": Utc::now(),
        },
        "target": target_snapshot.clone(),
    });

    if let Some(existing) = existing_metadata {
        if let Some(object) = metadata.as_object_mut() {
            object.insert("previous_metadata".to_string(), existing.clone());
        }
    }

    metadata
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample_workspace(metadata_targets: Value) -> RuntimeVmRemediationWorkspace {
        RuntimeVmRemediationWorkspace {
            id: 77,
            workspace_key: "workspace.test".to_string(),
            display_name: "Workspace Test".to_string(),
            description: None,
            owner_id: 42,
            lifecycle_state: "draft".to_string(),
            active_revision_id: Some(88),
            metadata: json!({"targets": metadata_targets}),
            lineage_tags: vec!["test".to_string()],
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            version: 0,
        }
    }

    fn sample_revision(plan_targets: Value) -> RuntimeVmRemediationWorkspaceRevision {
        RuntimeVmRemediationWorkspaceRevision {
            id: 88,
            workspace_id: 77,
            revision_number: 3,
            previous_revision_id: Some(66),
            created_by: 42,
            plan: json!({
                "playbooks": ["vm.restart"],
                "targets": plan_targets,
            }),
            schema_status: "succeeded".to_string(),
            schema_errors: Vec::new(),
            policy_status: "approved".to_string(),
            policy_veto_reasons: Vec::new(),
            simulation_status: "succeeded".to_string(),
            promotion_status: "pending".to_string(),
            metadata: json!({"targets": {"metadata_only": {"runtime_vm_instance_id": 404}}}),
            lineage_labels: vec!["alpha".to_string()],
            schema_validated_at: None,
            policy_evaluated_at: None,
            simulated_at: None,
            promoted_at: None,
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            version: 1,
        }
    }

    #[test]
    fn extract_targets_flattens_nested_lanes_and_defaults_playbooks() {
        let plan_targets = json!({
            "lanes": [
                {
                    "lane": "blue",
                    "stage": "canary",
                    "targets": [
                        {
                            "instance_id": "101",
                            "automation_payload": {"path": "lane-blue"}
                        },
                        {
                            "runtime_vm_instance_id": 202,
                            "playbook": "vm.redeploy"
                        }
                    ]
                }
            ],
            "direct": [
                {
                    "runtime_vm_instance_id": 303,
                    "automation_payload": {"path": "direct"}
                }
            ]
        });
        let metadata_targets = json!({
            "fallback": {"runtime_vm_instance_id": 404, "source": "workspace"}
        });
        let workspace = sample_workspace(metadata_targets);
        let revision = sample_revision(plan_targets);

        let mut targets = extract_promotion_targets(&workspace, &revision);
        targets.sort_by_key(|target| target.instance_id);

        assert_eq!(targets.len(), 4);
        assert_eq!(targets[0].instance_id, 101);
        assert_eq!(targets[0].playbook_key.as_deref(), Some("vm.restart"));
        assert_eq!(
            targets[0]
                .target_snapshot
                .get("lane")
                .and_then(Value::as_str),
            Some("blue")
        );
        assert_eq!(
            targets[0]
                .target_snapshot
                .get("stage")
                .and_then(Value::as_str),
            Some("canary")
        );
        assert!(targets[0].automation_payload.is_some());

        assert_eq!(targets[1].instance_id, 202);
        assert_eq!(targets[1].playbook_key.as_deref(), Some("vm.redeploy"));
        assert_eq!(
            targets[1]
                .target_snapshot
                .get("lane")
                .and_then(Value::as_str),
            Some("blue")
        );

        assert_eq!(targets[2].instance_id, 303);
        assert_eq!(targets[2].playbook_key.as_deref(), Some("vm.restart"));
        assert_eq!(
            targets[2]
                .automation_payload
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(Value::as_str),
            Some("direct")
        );

        assert_eq!(targets[3].instance_id, 404);
        assert_eq!(targets[3].playbook_key.as_deref(), Some("vm.restart"));
        assert_eq!(
            targets[3]
                .target_snapshot
                .get("source")
                .and_then(Value::as_str),
            Some("workspace")
        );
    }

    #[test]
    fn extract_targets_falls_back_to_revision_metadata_when_no_targets() {
        let workspace = sample_workspace(json!({"runtime_vm_instance_id": 707}));
        let mut revision = sample_revision(Value::Null);
        revision.plan = json!({"playbooks": ["vm.restart"]});
        revision.metadata = json!({"runtime_vm_instance_id": 808});

        let targets = extract_promotion_targets(&workspace, &revision);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].instance_id, 808);
    }
}

async fn stage_workspace_promotion_runs(
    pool: &PgPool,
    workspace: &RuntimeVmRemediationWorkspace,
    revision: &RuntimeVmRemediationWorkspaceRevision,
    gate_context: &Value,
    notes: &[String],
    requested_by: i32,
) -> Result<Vec<RuntimeVmRemediationRun>, AppError> {
    const DEFAULT_PLAYBOOK: &str = "default-vm-remediation";

    let targets = extract_promotion_targets(workspace, revision);
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let mut staged = Vec::new();
    for target in targets {
        let playbook_key = target
            .playbook_key
            .clone()
            .unwrap_or_else(|| DEFAULT_PLAYBOOK.to_string());
        let playbook = get_playbook_by_key(pool, &playbook_key).await?;

        let automation_payload_value = target.automation_payload.clone().unwrap_or(Value::Null);
        let automation_payload_for_insert =
            if automation_payload_value.is_null() && target.automation_payload.is_none() {
                None
            } else {
                Some(&automation_payload_value)
            };

        let metadata_value = build_promotion_metadata(
            workspace,
            revision,
            &target.target_snapshot,
            gate_context,
            notes,
            requested_by,
            None,
        );

        let request = EnsureRemediationRunRequest {
            runtime_vm_instance_id: target.instance_id,
            playbook_key: &playbook_key,
            playbook_id: playbook.as_ref().map(|record| record.id),
            metadata: Some(&metadata_value),
            automation_payload: automation_payload_for_insert,
            approval_required: playbook
                .as_ref()
                .map(|record| record.approval_required)
                .unwrap_or(false),
            assigned_owner_id: playbook
                .as_ref()
                .map(|record| record.owner_id)
                .or(Some(requested_by)),
            sla_duration_seconds: playbook
                .as_ref()
                .and_then(|record| record.sla_duration_seconds),
            workspace_id: Some(workspace.id),
            workspace_revision_id: Some(revision.id),
            promotion_gate_context: Some(gate_context),
        };

        match ensure_remediation_run(pool, request).await? {
            Some(run) => {
                let updated = update_run_workspace_linkage(
                    pool,
                    run.id,
                    workspace.id,
                    revision.id,
                    gate_context,
                    Some(&automation_payload_value),
                    Some(&metadata_value),
                )
                .await?
                .unwrap_or(run);
                ingest_accelerator_posture(pool, updated.runtime_vm_instance_id, &metadata_value)
                    .await?;
                broadcast_promotion_refresh(&updated, PromotionAutomationRefresh::Created);
                staged.push(updated);
            }
            None => {
                if let Some(existing) =
                    get_active_run_for_instance(pool, target.instance_id).await?
                {
                    let merged_metadata = build_promotion_metadata(
                        workspace,
                        revision,
                        &target.target_snapshot,
                        gate_context,
                        notes,
                        requested_by,
                        Some(&existing.metadata),
                    );
                    let updated = update_run_workspace_linkage(
                        pool,
                        existing.id,
                        workspace.id,
                        revision.id,
                        gate_context,
                        Some(&automation_payload_value),
                        Some(&merged_metadata),
                    )
                    .await?
                    .unwrap_or(existing);
                    ingest_accelerator_posture(
                        pool,
                        updated.runtime_vm_instance_id,
                        &merged_metadata,
                    )
                    .await?;
                    broadcast_promotion_refresh(&updated, PromotionAutomationRefresh::Refreshed);
                    staged.push(updated);
                } else {
                    warn!(
                        instance_id = target.instance_id,
                        workspace_id = workspace.id,
                        revision_id = revision.id,
                        "no active remediation run found to update after promotion"
                    );
                }
            }
        }
    }

    Ok(staged)
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

    let mut envelope =
        map_workspace_update_result(&pool, workspace_id, Some(revision_id), result).await?;

    if matches!(request.promotion_status.as_str(), "approved" | "completed") {
        if let Some(revision_envelope) = envelope
            .revisions
            .iter()
            .find(|entry| entry.revision.id == revision_id)
        {
            let runs = stage_workspace_promotion_runs(
                &pool,
                &envelope.workspace,
                &revision_envelope.revision,
                &request.gate_context,
                &request.notes,
                user.user_id,
            )
            .await?;
            if runs.is_empty() {
                trace!(
                    workspace_id,
                    revision_id,
                    "promotion completed without remediation targets"
                );
            } else {
                trace!(
                    workspace_id,
                    revision_id,
                    run_count = runs.len(),
                    "promotion triggered remediation orchestration"
                );
            }
            envelope.promotion_runs = runs;
        } else {
            warn!(
                workspace_id,
                revision_id, "promotion staging skipped because revision was missing from envelope"
            );
        }
    }

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
            workspace_id: query.workspace_id,
            workspace_revision_id: query.workspace_revision_id,
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
            workspace_id: None,
            workspace_revision_id: None,
            promotion_gate_context: None,
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
