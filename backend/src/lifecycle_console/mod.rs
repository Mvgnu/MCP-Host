use std::collections::{HashMap, HashSet};
use std::convert::Infallible;

use axum::{
    extract::{Extension, Query},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{query_as, PgPool, QueryBuilder};
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_stream::wrappers::ReceiverStream;

use crate::db::runtime_vm_remediation_runs::RuntimeVmRemediationRun;
use crate::db::runtime_vm_remediation_workspaces::{
    RuntimeVmRemediationWorkspace, RuntimeVmRemediationWorkspaceRevision,
    RuntimeVmRemediationWorkspaceValidationSnapshot,
};
use crate::db::runtime_vm_trust_registry::RuntimeVmTrustRegistryState;
use crate::error::{AppError, AppResult};

// key: lifecycle-console -> aggregation,data-plane

#[derive(Debug, Clone, Deserialize)]
pub struct LifecycleConsoleQuery {
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub lifecycle_state: Option<String>,
    #[serde(default)]
    pub owner_id: Option<i32>,
    #[serde(default)]
    pub workspace_key: Option<String>,
    #[serde(default)]
    pub workspace_search: Option<String>,
    #[serde(default)]
    pub promotion_lane: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub run_limit: Option<u32>,
}

impl Default for LifecycleConsoleQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: None,
            lifecycle_state: None,
            owner_id: None,
            workspace_key: None,
            workspace_search: None,
            promotion_lane: None,
            severity: None,
            run_limit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleConsolePage {
    pub workspaces: Vec<LifecycleWorkspaceSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleConsoleEventType {
    Snapshot,
    Heartbeat,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleConsoleEventEnvelope {
    #[serde(rename = "type")]
    pub event_type: LifecycleConsoleEventType,
    pub emitted_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<LifecycleConsolePage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<LifecycleDelta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LifecycleStreamQuery {
    #[serde(flatten)]
    pub query: LifecycleConsoleQuery,
    #[serde(default)]
    pub heartbeat_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleWorkspaceSnapshot {
    pub workspace: RuntimeVmRemediationWorkspace,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_revision: Option<LifecycleWorkspaceRevision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_runs: Vec<LifecycleRunSnapshot>,
    #[serde(default)]
    pub promotion_postures: Vec<LifecyclePromotionPosture>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleWorkspaceRevision {
    pub revision: RuntimeVmRemediationWorkspaceRevision,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_snapshots: Vec<RuntimeVmRemediationWorkspaceValidationSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunSnapshot {
    pub run: RuntimeVmRemediationRun,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<RuntimeVmTrustRegistryState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intelligence: Vec<IntelligenceScoreOverview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<MarketplaceReadiness>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecyclePromotionPosture {
    pub promotion_id: i64,
    pub manifest_digest: String,
    pub stage: String,
    pub status: String,
    pub track_id: i32,
    pub track_name: String,
    pub track_tier: String,
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub veto_reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediation_hooks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signals: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleDelta {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<LifecycleWorkspaceDelta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleWorkspaceDelta {
    pub workspace_id: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_deltas: Vec<LifecycleRunDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_run_ids: Vec<i64>,
    #[serde(default)]
    pub promotion_posture_deltas: Vec<LifecyclePromotionPostureDelta>,
    #[serde(default)]
    pub removed_promotion_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunDelta {
    pub run_id: i64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trust_changes: Vec<LifecycleFieldChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intelligence_changes: Vec<LifecycleFieldChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marketplace_changes: Vec<LifecycleFieldChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecyclePromotionPostureDelta {
    pub promotion_id: i64,
    pub manifest_digest: String,
    pub stage: String,
    pub status: String,
    pub track_id: i32,
    pub track_name: String,
    pub track_tier: String,
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub veto_reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediation_hooks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signals: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleFieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntelligenceScoreOverview {
    pub capability: String,
    pub backend: Option<String>,
    pub tier: Option<String>,
    pub score: f32,
    pub status: String,
    pub confidence: f32,
    pub last_observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceReadiness {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RuntimeVmInstanceRow {
    pub id: i64,
    pub server_id: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct IntelligenceRow {
    pub server_id: i32,
    pub capability: String,
    pub backend: Option<String>,
    pub tier: Option<String>,
    pub score: f32,
    pub status: String,
    pub confidence: f32,
    pub last_observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MarketplaceRow {
    pub server_id: i32,
    pub status: String,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TrustRegistryRow {
    pub runtime_vm_instance_id: i64,
    pub attestation_status: String,
    pub lifecycle_state: String,
    pub remediation_state: Option<String>,
    pub remediation_attempts: i32,
    pub freshness_deadline: Option<DateTime<Utc>>,
    pub provenance_ref: Option<String>,
    pub provenance: Option<Value>,
    pub version: i64,
    pub updated_at: DateTime<Utc>,
}

pub async fn list_snapshots(
    Extension(pool): Extension<PgPool>,
    Query(query): Query<LifecycleConsoleQuery>,
) -> AppResult<Json<LifecycleConsolePage>> {
    let page = fetch_page(&pool, &query).await?;
    Ok(Json(page))
}

// key: lifecycle-console -> sse,streaming
pub async fn stream_snapshots(
    Extension(pool): Extension<PgPool>,
    Query(params): Query<LifecycleStreamQuery>,
    headers: HeaderMap,
) -> AppResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>> {
    let poll_ms = params.heartbeat_ms.unwrap_or(5_000).clamp(1_000, 60_000);
    let poll_interval = Duration::from_millis(poll_ms);

    let mut query = params.query;
    if let Some(value) = headers.get("last-event-id") {
        if let Ok(text) = value.to_str() {
            if let Ok(cursor) = text.parse::<i64>() {
                query.cursor = Some(cursor);
            }
        }
    }

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        let mut cursor = query.cursor;
        let mut interval = tokio::time::interval(poll_interval);
        let mut initial = true;
        let mut last_snapshots: HashMap<i64, LifecycleWorkspaceSnapshot> = HashMap::new();
        loop {
            if initial {
                initial = false;
            } else {
                interval.tick().await;
            }

            let mut request = query.clone();
            request.cursor = cursor;

            match fetch_page(&pool_clone, &request).await {
                Ok(page) => {
                    if page.workspaces.is_empty() {
                        let envelope = LifecycleConsoleEventEnvelope {
                            event_type: LifecycleConsoleEventType::Heartbeat,
                            emitted_at: Utc::now(),
                            cursor,
                            page: None,
                            error: None,
                            delta: None,
                        };
                        match Event::default()
                            .event("lifecycle-heartbeat")
                            .json_data(&envelope)
                        {
                            Ok(event) => {
                                if tx.send(Ok(event)).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                tracing::error!(?err, "failed to encode lifecycle heartbeat");
                            }
                        }
                        continue;
                    }

                    let event_cursor = page
                        .workspaces
                        .last()
                        .map(|snapshot| snapshot.workspace.id)
                        .or(cursor);
                    let delta = compute_delta(&last_snapshots, &page);
                    for snapshot in &page.workspaces {
                        last_snapshots.insert(snapshot.workspace.id, snapshot.clone());
                    }
                    let envelope = LifecycleConsoleEventEnvelope {
                        event_type: LifecycleConsoleEventType::Snapshot,
                        emitted_at: Utc::now(),
                        cursor: event_cursor,
                        page: Some(page.clone()),
                        error: None,
                        delta,
                    };

                    match Event::default()
                        .event("lifecycle-snapshot")
                        .json_data(&envelope)
                    {
                        Ok(mut event) => {
                            if let Some(id) = event_cursor {
                                event = event.id(id.to_string());
                                cursor = Some(id);
                            }
                            if tx.send(Ok(event)).await.is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            tracing::error!(?err, "failed to encode lifecycle snapshot");
                        }
                    }
                }
                Err(err) => {
                    let envelope = LifecycleConsoleEventEnvelope {
                        event_type: LifecycleConsoleEventType::Error,
                        emitted_at: Utc::now(),
                        cursor,
                        page: None,
                        error: Some(err.to_string()),
                        delta: None,
                    };
                    match Event::default()
                        .event("lifecycle-error")
                        .json_data(&envelope)
                    {
                        Ok(event) => {
                            if tx.send(Ok(event)).await.is_err() {
                                break;
                            }
                        }
                        Err(encode_err) => {
                            tracing::error!(?encode_err, "failed to encode lifecycle error");
                        }
                    }
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(poll_interval)
            .text(":keep-alive\n\n"),
    ))
}

pub async fn fetch_page(
    pool: &PgPool,
    query: &LifecycleConsoleQuery,
) -> Result<LifecycleConsolePage, AppError> {
    let limit = query.limit.unwrap_or(25).min(100) as i64;
    let run_limit = query.run_limit.unwrap_or(5).min(10) as usize;

    let mut builder = QueryBuilder::new(
        "SELECT id, workspace_key, display_name, description, owner_id, lifecycle_state, \
             active_revision_id, metadata, lineage_tags, created_at, updated_at, version \
         FROM runtime_vm_remediation_workspaces",
    );

    let mut has_where = false;
    if let Some(state) = query.lifecycle_state.as_ref() {
        builder.push(" WHERE lifecycle_state = ");
        builder.push_bind(state);
        has_where = true;
    }

    if let Some(owner) = query.owner_id {
        builder.push(if has_where {
            " AND owner_id = "
        } else {
            " WHERE owner_id = "
        });
        builder.push_bind(owner);
        has_where = true;
    }

    if let Some(key) = query.workspace_key.as_ref() {
        builder.push(if has_where {
            " AND workspace_key = "
        } else {
            " WHERE workspace_key = "
        });
        builder.push_bind(key);
        has_where = true;
    }

    if let Some(search) = query.workspace_search.as_ref() {
        builder.push(if has_where {
            " AND (workspace_key ILIKE "
        } else {
            " WHERE (workspace_key ILIKE "
        });
        builder.push_bind(format!("%{}%", search));
        builder.push(" OR display_name ILIKE ");
        builder.push_bind(format!("%{}%", search));
        builder.push(")");
        has_where = true;
    }

    if let Some(lane) = query.promotion_lane.as_ref() {
        builder.push(if has_where {
            " AND metadata->>'promotion_lane' = "
        } else {
            " WHERE metadata->>'promotion_lane' = "
        });
        builder.push_bind(lane);
        has_where = true;
    }

    if let Some(severity) = query.severity.as_ref() {
        builder.push(if has_where {
            " AND metadata->>'severity' = "
        } else {
            " WHERE metadata->>'severity' = "
        });
        builder.push_bind(severity);
        has_where = true;
    }

    if let Some(cursor) = query.cursor {
        builder.push(if has_where {
            " AND id > "
        } else {
            " WHERE id > "
        });
        builder.push_bind(cursor);
    }

    builder.push(" ORDER BY id ASC LIMIT ");
    builder.push_bind(limit);

    let workspaces: Vec<RuntimeVmRemediationWorkspace> =
        builder.build_query_as().fetch_all(pool).await?;

    let next_cursor = workspaces.last().map(|w| w.id);

    if workspaces.is_empty() {
        return Ok(LifecycleConsolePage {
            workspaces: Vec::new(),
            next_cursor,
        });
    }

    let workspace_ids: Vec<i64> = workspaces.iter().map(|w| w.id).collect();
    let revision_ids: Vec<i64> = workspaces
        .iter()
        .filter_map(|w| w.active_revision_id)
        .collect();

    let revisions = load_revisions(pool, &revision_ids).await?;
    let gate_snapshots = load_gate_snapshots(pool, &revision_ids).await?;
    let runs = load_runs(pool, &workspace_ids, run_limit).await?;

    let mut instance_ids = HashSet::new();
    for run_list in runs.values() {
        for run in run_list {
            instance_ids.insert(run.runtime_vm_instance_id);
        }
    }

    let instance_rows = load_runtime_instances(pool, &instance_ids).await?;
    let trust_states = load_trust_states(pool, &instance_ids).await?;

    let mut server_ids = HashSet::new();
    for row in instance_rows.values() {
        server_ids.insert(row.server_id);
    }

    let intelligence_scores = load_intelligence_scores(pool, &server_ids).await?;
    let marketplace = load_marketplace(pool, &server_ids).await?;

    let mut snapshots = Vec::with_capacity(workspaces.len());
    let mut workspace_manifest_index: HashMap<i64, HashSet<String>> = HashMap::new();

    for workspace in workspaces {
        let revision = workspace
            .active_revision_id
            .and_then(|id| revisions.get(&id).cloned())
            .map(|revision| LifecycleWorkspaceRevision {
                gate_snapshots: gate_snapshots
                    .get(&revision.id)
                    .cloned()
                    .unwrap_or_default(),
                revision,
            });

        let mut workspace_runs = runs.get(&workspace.id).cloned().unwrap_or_default();
        workspace_runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        workspace_runs.truncate(run_limit);

        let mut run_snapshots = Vec::with_capacity(workspace_runs.len());
        for run in workspace_runs {
            let instance_id = run.runtime_vm_instance_id;
            let trust = trust_states.get(&instance_id).cloned();

            let intelligence = instance_rows
                .get(&instance_id)
                .and_then(|row| intelligence_scores.get(&row.server_id).cloned())
                .unwrap_or_default();

            let marketplace_state = instance_rows
                .get(&instance_id)
                .and_then(|row| marketplace.get(&row.server_id).cloned());

            run_snapshots.push(LifecycleRunSnapshot {
                trust,
                intelligence,
                marketplace: marketplace_state,
                run,
            });
        }

        let manifest_digests =
            collect_workspace_manifest_digests(&workspace, revision.as_ref(), &run_snapshots);
        if !manifest_digests.is_empty() {
            workspace_manifest_index.insert(workspace.id, manifest_digests);
        }

        snapshots.push(LifecycleWorkspaceSnapshot {
            workspace,
            active_revision: revision,
            recent_runs: run_snapshots,
            promotion_postures: Vec::new(),
        });
    }

    if !workspace_manifest_index.is_empty() {
        let mut manifest_digests = HashSet::new();
        for digests in workspace_manifest_index.values() {
            for digest in digests {
                manifest_digests.insert(digest.clone());
            }
        }

        let promotion_map = load_promotion_postures(pool, &manifest_digests).await?;

        for snapshot in &mut snapshots {
            if let Some(digests) = workspace_manifest_index.get(&snapshot.workspace.id) {
                let mut promotions = Vec::new();
                for digest in digests {
                    if let Some(entries) = promotion_map.get(digest) {
                        promotions.extend(entries.clone());
                    }
                }
                promotions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                promotions.dedup_by(|left, right| left.promotion_id == right.promotion_id);
                snapshot.promotion_postures = promotions;
            }
        }
    }

    Ok(LifecycleConsolePage {
        workspaces: snapshots,
        next_cursor,
    })
}

async fn load_revisions(
    pool: &PgPool,
    revision_ids: &[i64],
) -> Result<HashMap<i64, RuntimeVmRemediationWorkspaceRevision>, AppError> {
    if revision_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<RuntimeVmRemediationWorkspaceRevision> = query_as(
        r#"
        SELECT id, workspace_id, revision_number, previous_revision_id, created_by, plan,
               schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
               promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
               simulated_at, promoted_at, created_at, updated_at, version
        FROM runtime_vm_remediation_workspace_revisions
        WHERE id = ANY($1)
        "#,
    )
    .bind(revision_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

async fn load_gate_snapshots(
    pool: &PgPool,
    revision_ids: &[i64],
) -> Result<HashMap<i64, Vec<RuntimeVmRemediationWorkspaceValidationSnapshot>>, AppError> {
    if revision_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<RuntimeVmRemediationWorkspaceValidationSnapshot> = query_as(
        r#"
        SELECT id, workspace_revision_id, snapshot_type, status, gate_context, notes,
               recorded_at, metadata, created_at, updated_at, version
        FROM runtime_vm_remediation_workspace_validation_snapshots
        WHERE workspace_revision_id = ANY($1)
        ORDER BY recorded_at DESC
        "#,
    )
    .bind(revision_ids)
    .fetch_all(pool)
    .await?;

    let mut grouped: HashMap<i64, Vec<RuntimeVmRemediationWorkspaceValidationSnapshot>> =
        HashMap::new();
    for row in rows {
        grouped
            .entry(row.workspace_revision_id)
            .or_default()
            .push(row);
    }
    Ok(grouped)
}

async fn load_runs(
    pool: &PgPool,
    workspace_ids: &[i64],
    limit: usize,
) -> Result<HashMap<i64, Vec<RuntimeVmRemediationRun>>, AppError> {
    if workspace_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<RuntimeVmRemediationRun> = query_as(
        r#"
        SELECT
            id,
            runtime_vm_instance_id,
            playbook,
            playbook_id,
            status,
            automation_payload,
            approval_required,
            started_at,
            completed_at,
            last_error,
            assigned_owner_id,
            sla_deadline,
            approval_state,
            approval_decided_at,
            approval_notes,
            metadata,
            workspace_id,
            workspace_revision_id,
            promotion_gate_context,
            version,
            updated_at,
            cancelled_at,
            cancellation_reason,
            failure_reason
        FROM (
            SELECT
                runs.*,
                ROW_NUMBER() OVER (PARTITION BY workspace_id ORDER BY started_at DESC) AS row_number
            FROM runtime_vm_remediation_runs runs
            WHERE workspace_id = ANY($1)
        ) ranked
        WHERE ranked.row_number <= $2
        ORDER BY workspace_id, started_at DESC
        "#,
    )
    .bind(workspace_ids)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    let mut grouped: HashMap<i64, Vec<RuntimeVmRemediationRun>> = HashMap::new();
    for row in rows {
        if let Some(workspace_id) = row.workspace_id {
            grouped.entry(workspace_id).or_default().push(row);
        }
    }
    Ok(grouped)
}

async fn load_runtime_instances(
    pool: &PgPool,
    instance_ids: &HashSet<i64>,
) -> Result<HashMap<i64, RuntimeVmInstanceRow>, AppError> {
    if instance_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<RuntimeVmInstanceRow> = query_as(
        r#"
        SELECT id, server_id
        FROM runtime_vm_instances
        WHERE id = ANY($1)
        "#,
    )
    .bind(instance_ids.iter().copied().collect::<Vec<_>>())
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

async fn load_trust_states(
    pool: &PgPool,
    instance_ids: &HashSet<i64>,
) -> Result<HashMap<i64, RuntimeVmTrustRegistryState>, AppError> {
    if instance_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<TrustRegistryRow> = query_as(
        r#"
        SELECT
            runtime_vm_instance_id,
            attestation_status,
            lifecycle_state,
            remediation_state,
            remediation_attempts,
            freshness_deadline,
            provenance_ref,
            provenance,
            version,
            updated_at
        FROM runtime_vm_trust_registry
        WHERE runtime_vm_instance_id = ANY($1)
        "#,
    )
    .bind(instance_ids.iter().copied().collect::<Vec<_>>())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.runtime_vm_instance_id,
                RuntimeVmTrustRegistryState {
                    runtime_vm_instance_id: row.runtime_vm_instance_id,
                    attestation_status: row.attestation_status,
                    lifecycle_state: row.lifecycle_state,
                    remediation_state: row.remediation_state,
                    remediation_attempts: row.remediation_attempts,
                    freshness_deadline: row.freshness_deadline,
                    provenance_ref: row.provenance_ref,
                    provenance: row.provenance,
                    version: row.version,
                    updated_at: row.updated_at,
                },
            )
        })
        .collect())
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PromotionPostureRow {
    pub id: i64,
    pub promotion_track_id: i32,
    pub manifest_digest: String,
    pub stage: String,
    pub status: String,
    pub notes: Vec<String>,
    pub posture_verdict: Option<Value>,
    pub updated_at: DateTime<Utc>,
    pub track_name: String,
    pub tier: String,
}

struct PromotionVerdictSummary {
    allowed: bool,
    veto_reasons: Vec<String>,
    notes: Vec<String>,
    signals: Option<Value>,
    remediation_hooks: Vec<String>,
}

fn collect_workspace_manifest_digests(
    workspace: &RuntimeVmRemediationWorkspace,
    revision: Option<&LifecycleWorkspaceRevision>,
    runs: &[LifecycleRunSnapshot],
) -> HashSet<String> {
    let mut digests = HashSet::new();
    collect_manifest_digests_from_value(&workspace.metadata, &mut digests);
    if let Some(revision) = revision {
        collect_manifest_digests_from_value(&revision.revision.plan, &mut digests);
        collect_manifest_digests_from_value(&revision.revision.metadata, &mut digests);
    }
    for run in runs {
        collect_manifest_digests_from_value(&run.run.metadata, &mut digests);
        if let Some(payload) = run.run.automation_payload.as_ref() {
            collect_manifest_digests_from_value(payload, &mut digests);
        }
        collect_manifest_digests_from_value(&run.run.promotion_gate_context, &mut digests);
    }
    digests
}

fn collect_manifest_digests_from_value(value: &Value, digests: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map {
                if key == "manifest_digest" {
                    if let Some(text) = entry.as_str() {
                        if !text.is_empty() {
                            digests.insert(text.to_string());
                        }
                    }
                }
                collect_manifest_digests_from_value(entry, digests);
            }
        }
        Value::Array(items) => {
            for entry in items {
                collect_manifest_digests_from_value(entry, digests);
            }
        }
        _ => {}
    }
}

async fn load_promotion_postures(
    pool: &PgPool,
    manifest_digests: &HashSet<String>,
) -> Result<HashMap<String, Vec<LifecyclePromotionPosture>>, AppError> {
    if manifest_digests.is_empty() {
        return Ok(HashMap::new());
    }

    let digests: Vec<String> = manifest_digests.iter().cloned().collect();
    let rows: Vec<PromotionPostureRow> = query_as(
        r#"
        SELECT ap.id, ap.promotion_track_id, ap.manifest_digest, ap.stage, ap.status,
               ap.notes, ap.posture_verdict, ap.updated_at, t.name AS track_name, t.tier
        FROM artifact_promotions ap
        JOIN promotion_tracks t ON t.id = ap.promotion_track_id
        WHERE ap.manifest_digest = ANY($1)
        ORDER BY ap.updated_at DESC
        "#,
    )
    .bind(&digests)
    .fetch_all(pool)
    .await?;

    let mut grouped: HashMap<String, Vec<LifecyclePromotionPosture>> = HashMap::new();
    for row in rows {
        let summary = summarize_promotion_verdict(row.posture_verdict.as_ref());
        let mut notes = row.notes.clone();
        notes.extend(summary.notes.iter().cloned());
        notes.sort();
        notes.dedup();

        let mut hooks = summary.remediation_hooks.clone();
        hooks.sort();
        hooks.dedup();

        grouped
            .entry(row.manifest_digest.clone())
            .or_default()
            .push(LifecyclePromotionPosture {
                promotion_id: row.id,
                manifest_digest: row.manifest_digest,
                stage: row.stage,
                status: row.status,
                track_id: row.promotion_track_id,
                track_name: row.track_name,
                track_tier: row.tier,
                allowed: summary.allowed,
                veto_reasons: summary.veto_reasons,
                notes,
                updated_at: row.updated_at,
                remediation_hooks: hooks,
                signals: summary.signals,
            });
    }

    Ok(grouped)
}

fn summarize_promotion_verdict(value: Option<&Value>) -> PromotionVerdictSummary {
    let mut summary = PromotionVerdictSummary {
        allowed: true,
        veto_reasons: Vec::new(),
        notes: Vec::new(),
        signals: None,
        remediation_hooks: Vec::new(),
    };

    let Some(verdict) = value.and_then(|v| v.as_object()) else {
        return summary;
    };

    if let Some(flag) = verdict.get("allowed").and_then(Value::as_bool) {
        summary.allowed = flag;
    }

    if let Some(reasons) = verdict.get("reasons").and_then(Value::as_array) {
        summary.veto_reasons = reasons
            .iter()
            .filter_map(|entry| entry.as_str().map(|value| value.to_string()))
            .collect();
    }

    if let Some(notes) = verdict.get("notes").and_then(Value::as_array) {
        summary.notes = notes
            .iter()
            .filter_map(|entry| entry.as_str().map(|value| value.to_string()))
            .collect();
    }

    if let Some(metadata) = verdict.get("metadata") {
        if let Some(signals) = metadata.get("signals") {
            summary.signals = Some(signals.clone());
            collect_remediation_hooks(signals, &mut summary.remediation_hooks);
        }
        collect_remediation_hooks(metadata, &mut summary.remediation_hooks);
    }

    summary.veto_reasons.sort();
    summary.veto_reasons.dedup();
    summary.notes.sort();
    summary.notes.dedup();
    summary.remediation_hooks.sort();
    summary.remediation_hooks.dedup();

    summary
}

fn collect_remediation_hooks(value: &Value, hooks: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map {
                if key == "hooks" || key == "remediation_hooks" {
                    if let Some(array) = entry.as_array() {
                        for item in array {
                            if let Some(text) = item.as_str() {
                                if !text.is_empty() {
                                    hooks.push(text.to_string());
                                }
                            }
                        }
                    } else if let Some(text) = entry.as_str() {
                        if !text.is_empty() {
                            hooks.push(text.to_string());
                        }
                    }
                } else {
                    collect_remediation_hooks(entry, hooks);
                }
            }
        }
        Value::Array(items) => {
            for entry in items {
                collect_remediation_hooks(entry, hooks);
            }
        }
        _ => {}
    }
}

fn compute_delta(
    previous: &HashMap<i64, LifecycleWorkspaceSnapshot>,
    page: &LifecycleConsolePage,
) -> Option<LifecycleDelta> {
    let mut workspaces = Vec::new();
    let mut seen = HashSet::new();

    for snapshot in &page.workspaces {
        let workspace_id = snapshot.workspace.id;
        seen.insert(workspace_id);
        let previous_snapshot = previous.get(&workspace_id);
        let previous_runs = previous_snapshot
            .map(|s| s.recent_runs.as_slice())
            .unwrap_or(&[]);
        let (run_deltas, removed_run_ids) = diff_runs(previous_runs, &snapshot.recent_runs);

        let previous_promotions = previous_snapshot
            .map(|s| s.promotion_postures.as_slice())
            .unwrap_or(&[]);
        let (promotion_posture_deltas, removed_promotion_ids) =
            diff_promotion_postures(previous_promotions, &snapshot.promotion_postures);

        if !run_deltas.is_empty()
            || !removed_run_ids.is_empty()
            || !promotion_posture_deltas.is_empty()
            || !removed_promotion_ids.is_empty()
            || previous_snapshot.is_none()
        {
            workspaces.push(LifecycleWorkspaceDelta {
                workspace_id,
                run_deltas,
                removed_run_ids,
                promotion_posture_deltas,
                removed_promotion_ids,
            });
        }
    }

    for (workspace_id, prev_snapshot) in previous {
        if !seen.contains(workspace_id) {
            let removed_run_ids = prev_snapshot
                .recent_runs
                .iter()
                .map(|run| run.run.id)
                .collect::<Vec<_>>();
            let removed_promotion_ids = prev_snapshot
                .promotion_postures
                .iter()
                .map(|posture| posture.promotion_id)
                .collect::<Vec<_>>();
            workspaces.push(LifecycleWorkspaceDelta {
                workspace_id: *workspace_id,
                run_deltas: Vec::new(),
                removed_run_ids,
                promotion_posture_deltas: Vec::new(),
                removed_promotion_ids,
            });
        }
    }

    if workspaces.is_empty() {
        None
    } else {
        Some(LifecycleDelta { workspaces })
    }
}

fn diff_promotion_postures(
    previous: &[LifecyclePromotionPosture],
    current: &[LifecyclePromotionPosture],
) -> (Vec<LifecyclePromotionPostureDelta>, Vec<i64>) {
    let mut previous_map: HashMap<i64, &LifecyclePromotionPosture> = HashMap::new();
    for posture in previous {
        previous_map.insert(posture.promotion_id, posture);
    }

    let mut deltas = Vec::new();
    for posture in current {
        let promotion_id = posture.promotion_id;
        let previous_posture = previous_map.remove(&promotion_id);
        let has_changes = previous_posture.map_or(true, |prev| {
            prev.status != posture.status
                || prev.allowed != posture.allowed
                || prev.veto_reasons != posture.veto_reasons
                || prev.notes != posture.notes
                || prev.remediation_hooks != posture.remediation_hooks
                || prev.signals != posture.signals
        });

        if has_changes {
            deltas.push(LifecyclePromotionPostureDelta {
                promotion_id,
                manifest_digest: posture.manifest_digest.clone(),
                stage: posture.stage.clone(),
                status: posture.status.clone(),
                track_id: posture.track_id,
                track_name: posture.track_name.clone(),
                track_tier: posture.track_tier.clone(),
                allowed: posture.allowed,
                veto_reasons: posture.veto_reasons.clone(),
                notes: posture.notes.clone(),
                updated_at: posture.updated_at,
                remediation_hooks: posture.remediation_hooks.clone(),
                signals: posture.signals.clone(),
            });
        }
    }

    let removed_ids = previous_map.into_keys().collect::<Vec<_>>();
    (deltas, removed_ids)
}

fn diff_runs(
    previous: &[LifecycleRunSnapshot],
    current: &[LifecycleRunSnapshot],
) -> (Vec<LifecycleRunDelta>, Vec<i64>) {
    let mut previous_map: HashMap<i64, &LifecycleRunSnapshot> = HashMap::new();
    for snapshot in previous {
        previous_map.insert(snapshot.run.id, snapshot);
    }

    let mut deltas = Vec::new();
    for run in current {
        let run_id = run.run.id;
        let previous_snapshot = previous_map.remove(&run_id);
        let status = run.run.status.clone();
        let trust_changes = diff_trust(
            previous_snapshot.and_then(|s| s.trust.as_ref()),
            run.trust.as_ref(),
        );
        let intelligence_changes = diff_intelligence(
            previous_snapshot
                .map(|s| s.intelligence.as_slice())
                .unwrap_or(&[]),
            &run.intelligence,
        );
        let marketplace_changes = diff_marketplace(
            previous_snapshot.and_then(|s| s.marketplace.as_ref()),
            run.marketplace.as_ref(),
        );
        let previous_status = previous_snapshot.map(|s| s.run.status.clone());
        let has_changes = previous_snapshot.is_none()
            || previous_status.as_ref() != Some(&status)
            || !trust_changes.is_empty()
            || !intelligence_changes.is_empty()
            || !marketplace_changes.is_empty();

        if has_changes {
            deltas.push(LifecycleRunDelta {
                run_id,
                status,
                trust_changes,
                intelligence_changes,
                marketplace_changes,
            });
        }
    }

    let removed_run_ids = previous_map.into_keys().collect::<Vec<_>>();
    (deltas, removed_run_ids)
}

fn diff_trust(
    previous: Option<&RuntimeVmTrustRegistryState>,
    current: Option<&RuntimeVmTrustRegistryState>,
) -> Vec<LifecycleFieldChange> {
    let mut changes = Vec::new();
    match (previous, current) {
        (None, None) => {}
        (None, Some(curr)) => {
            push_change(
                &mut changes,
                "trust.attestation_status",
                None,
                Some(curr.attestation_status.clone()),
            );
            push_change(
                &mut changes,
                "trust.lifecycle_state",
                None,
                Some(curr.lifecycle_state.clone()),
            );
            push_change(
                &mut changes,
                "trust.remediation_state",
                None,
                curr.remediation_state.clone(),
            );
            push_change(
                &mut changes,
                "trust.remediation_attempts",
                None,
                Some(curr.remediation_attempts.to_string()),
            );
            push_change(
                &mut changes,
                "trust.freshness_deadline",
                None,
                curr.freshness_deadline.map(|d| d.to_rfc3339()),
            );
            push_change(
                &mut changes,
                "trust.provenance_ref",
                None,
                curr.provenance_ref.clone(),
            );
            push_change(
                &mut changes,
                "trust.provenance",
                None,
                curr.provenance.as_ref().map(|value| value.to_string()),
            );
            push_change(
                &mut changes,
                "trust.version",
                None,
                Some(curr.version.to_string()),
            );
            push_change(
                &mut changes,
                "trust.updated_at",
                None,
                Some(curr.updated_at.to_rfc3339()),
            );
        }
        (Some(prev), Some(curr)) => {
            push_change(
                &mut changes,
                "trust.attestation_status",
                Some(prev.attestation_status.clone()),
                Some(curr.attestation_status.clone()),
            );
            push_change(
                &mut changes,
                "trust.lifecycle_state",
                Some(prev.lifecycle_state.clone()),
                Some(curr.lifecycle_state.clone()),
            );
            push_change(
                &mut changes,
                "trust.remediation_state",
                prev.remediation_state.clone(),
                curr.remediation_state.clone(),
            );
            push_change(
                &mut changes,
                "trust.remediation_attempts",
                Some(prev.remediation_attempts.to_string()),
                Some(curr.remediation_attempts.to_string()),
            );
            push_change(
                &mut changes,
                "trust.freshness_deadline",
                prev.freshness_deadline.map(|d| d.to_rfc3339()),
                curr.freshness_deadline.map(|d| d.to_rfc3339()),
            );
            push_change(
                &mut changes,
                "trust.provenance_ref",
                prev.provenance_ref.clone(),
                curr.provenance_ref.clone(),
            );
            push_change(
                &mut changes,
                "trust.provenance",
                prev.provenance.as_ref().map(|value| value.to_string()),
                curr.provenance.as_ref().map(|value| value.to_string()),
            );
            push_change(
                &mut changes,
                "trust.version",
                Some(prev.version.to_string()),
                Some(curr.version.to_string()),
            );
            push_change(
                &mut changes,
                "trust.updated_at",
                Some(prev.updated_at.to_rfc3339()),
                Some(curr.updated_at.to_rfc3339()),
            );
        }
        (Some(prev), None) => {
            push_change(
                &mut changes,
                "trust.attestation_status",
                Some(prev.attestation_status.clone()),
                None,
            );
            push_change(
                &mut changes,
                "trust.lifecycle_state",
                Some(prev.lifecycle_state.clone()),
                None,
            );
            push_change(
                &mut changes,
                "trust.remediation_state",
                prev.remediation_state.clone(),
                None,
            );
            push_change(
                &mut changes,
                "trust.remediation_attempts",
                Some(prev.remediation_attempts.to_string()),
                None,
            );
            push_change(
                &mut changes,
                "trust.freshness_deadline",
                prev.freshness_deadline.map(|d| d.to_rfc3339()),
                None,
            );
            push_change(
                &mut changes,
                "trust.provenance_ref",
                prev.provenance_ref.clone(),
                None,
            );
            push_change(
                &mut changes,
                "trust.provenance",
                prev.provenance.as_ref().map(|value| value.to_string()),
                None,
            );
            push_change(
                &mut changes,
                "trust.version",
                Some(prev.version.to_string()),
                None,
            );
            push_change(
                &mut changes,
                "trust.updated_at",
                Some(prev.updated_at.to_rfc3339()),
                None,
            );
        }
    }

    changes
}

fn diff_intelligence(
    previous: &[IntelligenceScoreOverview],
    current: &[IntelligenceScoreOverview],
) -> Vec<LifecycleFieldChange> {
    let mut changes = Vec::new();
    let mut previous_map: HashMap<String, &IntelligenceScoreOverview> = HashMap::new();
    for score in previous {
        previous_map.insert(score.capability.clone(), score);
    }

    for score in current {
        let previous_score = previous_map.remove(&score.capability);
        let capability_prefix = format!("intelligence.{}", score.capability);
        match previous_score {
            None => {
                push_change(
                    &mut changes,
                    &format!("{}.status", capability_prefix),
                    None,
                    Some(score.status.clone()),
                );
                push_change(
                    &mut changes,
                    &format!("{}.tier", capability_prefix),
                    None,
                    score.tier.clone(),
                );
                push_change(
                    &mut changes,
                    &format!("{}.backend", capability_prefix),
                    None,
                    score.backend.clone(),
                );
                push_change(
                    &mut changes,
                    &format!("{}.score", capability_prefix),
                    None,
                    Some(format_float(score.score)),
                );
                push_change(
                    &mut changes,
                    &format!("{}.confidence", capability_prefix),
                    None,
                    Some(format_float(score.confidence)),
                );
                push_change(
                    &mut changes,
                    &format!("{}.last_observed_at", capability_prefix),
                    None,
                    Some(score.last_observed_at.to_rfc3339()),
                );
            }
            Some(prev) => {
                push_change(
                    &mut changes,
                    &format!("{}.status", capability_prefix),
                    Some(prev.status.clone()),
                    Some(score.status.clone()),
                );
                push_change(
                    &mut changes,
                    &format!("{}.tier", capability_prefix),
                    prev.tier.clone(),
                    score.tier.clone(),
                );
                push_change(
                    &mut changes,
                    &format!("{}.backend", capability_prefix),
                    prev.backend.clone(),
                    score.backend.clone(),
                );
                push_change(
                    &mut changes,
                    &format!("{}.score", capability_prefix),
                    Some(format_float(prev.score)),
                    Some(format_float(score.score)),
                );
                push_change(
                    &mut changes,
                    &format!("{}.confidence", capability_prefix),
                    Some(format_float(prev.confidence)),
                    Some(format_float(score.confidence)),
                );
                push_change(
                    &mut changes,
                    &format!("{}.last_observed_at", capability_prefix),
                    Some(prev.last_observed_at.to_rfc3339()),
                    Some(score.last_observed_at.to_rfc3339()),
                );
            }
        }
    }

    for (capability, prev) in previous_map {
        let capability_prefix = format!("intelligence.{}", capability);
        push_change(
            &mut changes,
            &format!("{}.status", capability_prefix),
            Some(prev.status.clone()),
            None,
        );
        push_change(
            &mut changes,
            &format!("{}.tier", capability_prefix),
            prev.tier.clone(),
            None,
        );
        push_change(
            &mut changes,
            &format!("{}.backend", capability_prefix),
            prev.backend.clone(),
            None,
        );
        push_change(
            &mut changes,
            &format!("{}.score", capability_prefix),
            Some(format_float(prev.score)),
            None,
        );
        push_change(
            &mut changes,
            &format!("{}.confidence", capability_prefix),
            Some(format_float(prev.confidence)),
            None,
        );
        push_change(
            &mut changes,
            &format!("{}.last_observed_at", capability_prefix),
            Some(prev.last_observed_at.to_rfc3339()),
            None,
        );
    }

    changes
}

fn diff_marketplace(
    previous: Option<&MarketplaceReadiness>,
    current: Option<&MarketplaceReadiness>,
) -> Vec<LifecycleFieldChange> {
    let mut changes = Vec::new();
    match (previous, current) {
        (None, None) => {}
        (None, Some(curr)) => {
            push_change(
                &mut changes,
                "marketplace.status",
                None,
                Some(curr.status.clone()),
            );
            push_change(
                &mut changes,
                "marketplace.last_completed_at",
                None,
                curr.last_completed_at.map(|dt| dt.to_rfc3339()),
            );
        }
        (Some(prev), Some(curr)) => {
            push_change(
                &mut changes,
                "marketplace.status",
                Some(prev.status.clone()),
                Some(curr.status.clone()),
            );
            push_change(
                &mut changes,
                "marketplace.last_completed_at",
                prev.last_completed_at.map(|dt| dt.to_rfc3339()),
                curr.last_completed_at.map(|dt| dt.to_rfc3339()),
            );
        }
        (Some(prev), None) => {
            push_change(
                &mut changes,
                "marketplace.status",
                Some(prev.status.clone()),
                None,
            );
            push_change(
                &mut changes,
                "marketplace.last_completed_at",
                prev.last_completed_at.map(|dt| dt.to_rfc3339()),
                None,
            );
        }
    }
    changes
}

fn push_change(
    changes: &mut Vec<LifecycleFieldChange>,
    field: &str,
    previous: Option<String>,
    current: Option<String>,
) {
    if previous != current {
        changes.push(LifecycleFieldChange {
            field: field.to_string(),
            previous,
            current,
        });
    }
}

fn format_float(value: f32) -> String {
    format!("{value:.4}")
}

async fn load_intelligence_scores(
    pool: &PgPool,
    server_ids: &HashSet<i32>,
) -> Result<HashMap<i32, Vec<IntelligenceScoreOverview>>, AppError> {
    if server_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<IntelligenceRow> = query_as(
        r#"
        SELECT server_id, capability, backend, tier, score::float4 AS score,
               status, confidence::float4 AS confidence, last_observed_at
        FROM capability_intelligence_scores
        WHERE server_id = ANY($1)
        ORDER BY last_observed_at DESC
        "#,
    )
    .bind(server_ids.iter().copied().collect::<Vec<_>>())
    .fetch_all(pool)
    .await?;

    let mut grouped: HashMap<i32, Vec<IntelligenceScoreOverview>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.server_id)
            .or_default()
            .push(IntelligenceScoreOverview {
                capability: row.capability,
                backend: row.backend,
                tier: row.tier,
                score: row.score,
                status: row.status,
                confidence: row.confidence,
                last_observed_at: row.last_observed_at,
            });
    }

    Ok(grouped)
}

async fn load_marketplace(
    pool: &PgPool,
    server_ids: &HashSet<i32>,
) -> Result<HashMap<i32, MarketplaceReadiness>, AppError> {
    if server_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<MarketplaceRow> = query_as(
        r#"
        SELECT server_id, status, completed_at
        FROM (
            SELECT
                server_id,
                status,
                completed_at,
                ROW_NUMBER() OVER (PARTITION BY server_id ORDER BY completed_at DESC NULLS LAST) AS row_number
            FROM build_artifact_runs
            WHERE server_id = ANY($1)
        ) ranked
        WHERE ranked.row_number = 1
        "#,
    )
    .bind(server_ids.iter().copied().collect::<Vec<_>>())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.server_id,
                MarketplaceReadiness {
                    status: row.status,
                    last_completed_at: row.completed_at,
                },
            )
        })
        .collect())
}
