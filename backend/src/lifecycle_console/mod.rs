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
use serde_json::{to_value, Value};
use sqlx::{query_as, PgPool, QueryBuilder};
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_stream::wrappers::ReceiverStream;

use sha2::{Digest, Sha256};

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promotion_runs: Vec<RuntimeVmRemediationRun>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_window: Option<LifecycleRunExecutionWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_attempt: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_ledger: Vec<LifecycleRunRetryRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_override: Option<LifecycleRunOverride>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<LifecycleRunArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_fingerprints: Vec<LifecycleRunArtifactFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_verdict: Option<LifecycleRunPromotionVerdictRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunExecutionWindow {
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunRetryRecord {
    pub attempt: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunOverride {
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_email: Option<String>,
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
pub struct LifecycleRunArtifactFingerprint {
    pub manifest_digest: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunPromotionVerdictRef {
    pub verdict_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_tier: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promotion_run_deltas: Vec<LifecyclePromotionRunDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_promotion_run_ids: Vec<i64>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analytics_changes: Vec<LifecycleFieldChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_changes: Vec<LifecycleFieldChange>,
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
pub struct LifecyclePromotionRunDelta {
    pub run_id: i64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation_payload_changes: Vec<LifecycleFieldChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_context_changes: Vec<LifecycleFieldChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata_changes: Vec<LifecycleFieldChange>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_duration_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRunArtifact {
    pub manifest_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct RuntimeVmInstanceRow {
    pub id: i64,
    pub server_id: i32,
}

#[derive(Debug, Clone)]
struct OverrideActorRecord {
    pub email: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct UserRow {
    pub id: i32,
    pub email: String,
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
    pub manifest_digest: Option<String>,
    pub manifest_tag: Option<String>,
    pub registry_image: Option<String>,
    pub duration_seconds: Option<i64>,
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
    let promotion_runs = load_promotion_runs(pool, &workspace_ids, run_limit).await?;

    let mut instance_ids = HashSet::new();
    let mut override_actor_ids = HashSet::new();
    for run_list in runs.values() {
        for run in run_list {
            instance_ids.insert(run.runtime_vm_instance_id);
            if let Some(actor_id) = run.analytics_override_actor_id {
                override_actor_ids.insert(actor_id);
            }
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
    let override_actors = load_override_actors(pool, &override_actor_ids).await?;

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

            let duration_seconds = compute_run_duration(&run);
            let duration_ms = compute_run_duration_ms(&run);
            let execution_window = build_execution_window(&run);
            let retry_attempt = compute_run_retry_attempt(&run);
            let retry_limit = compute_run_retry_limit(&run);
            let retry_count = compute_retry_count(&run, retry_attempt);
            let retry_ledger = build_retry_ledger(&run, retry_attempt);
            let override_reason = compute_run_override_reason(&run);
            let manual_override =
                build_manual_override(&run, override_reason.clone(), &override_actors);
            let artifacts = extract_run_artifacts(&run);
            let artifact_fingerprints = derive_artifact_fingerprints(&artifacts);

            run_snapshots.push(LifecycleRunSnapshot {
                trust,
                intelligence,
                marketplace: marketplace_state,
                duration_seconds,
                duration_ms,
                execution_window,
                retry_attempt,
                retry_limit,
                retry_count,
                retry_ledger,
                override_reason,
                manual_override,
                artifacts,
                artifact_fingerprints,
                promotion_verdict: None,
                run,
            });
        }

        let mut workspace_promotion_runs = promotion_runs
            .get(&workspace.id)
            .cloned()
            .unwrap_or_default();
        workspace_promotion_runs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        workspace_promotion_runs.truncate(run_limit);

        let manifest_digests = collect_workspace_manifest_digests(
            &workspace,
            revision.as_ref(),
            &run_snapshots,
            &workspace_promotion_runs,
        );
        if !manifest_digests.is_empty() {
            workspace_manifest_index.insert(workspace.id, manifest_digests);
        }

        snapshots.push(LifecycleWorkspaceSnapshot {
            workspace,
            active_revision: revision,
            recent_runs: run_snapshots,
            promotion_runs: workspace_promotion_runs,
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
        let artifact_map = load_build_artifacts_by_digest(pool, &manifest_digests).await?;
        let mut promotion_index_by_id: HashMap<i64, LifecyclePromotionPosture> = HashMap::new();
        for entries in promotion_map.values() {
            for posture in entries {
                promotion_index_by_id.insert(posture.promotion_id, posture.clone());
            }
        }

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

            for run in &mut snapshot.recent_runs {
                enrich_run_artifacts(&mut run.artifacts, &artifact_map);
                run.artifact_fingerprints = derive_artifact_fingerprints(&run.artifacts);
                if run.promotion_verdict.is_none() {
                    let mut verdict = run.run.analytics_promotion_verdict_id.and_then(|id| {
                        promotion_index_by_id
                            .get(&id)
                            .map(|posture| make_promotion_verdict_ref(id, Some(posture)))
                    });

                    if verdict.is_none() {
                        for artifact in &run.artifacts {
                            if let Some(entries) = promotion_map.get(&artifact.manifest_digest) {
                                if let Some(posture) = entries.first() {
                                    verdict = Some(make_promotion_verdict_ref(
                                        posture.promotion_id,
                                        Some(posture),
                                    ));
                                    break;
                                }
                            }
                        }
                    }

                    run.promotion_verdict = verdict;
                }
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

async fn load_promotion_runs(
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
                ROW_NUMBER() OVER (PARTITION BY workspace_id ORDER BY updated_at DESC) AS row_number
            FROM runtime_vm_remediation_runs runs
            WHERE workspace_id = ANY($1)
              AND metadata ? 'promotion'
        ) ranked
        WHERE ranked.row_number <= $2
        ORDER BY workspace_id, updated_at DESC
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

async fn load_override_actors(
    pool: &PgPool,
    actor_ids: &HashSet<i32>,
) -> Result<HashMap<i32, OverrideActorRecord>, AppError> {
    if actor_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let ids: Vec<i32> = actor_ids.iter().copied().collect();
    let rows: Vec<UserRow> = query_as(
        r#"
        SELECT id, email
        FROM users
        WHERE id = ANY($1)
        "#,
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.id, OverrideActorRecord { email: row.email }))
        .collect())
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

#[derive(Debug, Clone, sqlx::FromRow)]
struct BuildArtifactRow {
    pub manifest_digest: String,
    pub manifest_tag: Option<String>,
    pub registry_image: Option<String>,
    pub source_repo: Option<String>,
    pub source_revision: Option<String>,
    pub status: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i64>,
}

#[derive(Debug, Clone)]
struct BuildArtifactSummary {
    manifest_tag: Option<String>,
    registry_image: Option<String>,
    source_repo: Option<String>,
    source_revision: Option<String>,
    status: String,
    completed_at: Option<DateTime<Utc>>,
    duration_seconds: Option<i64>,
}

fn collect_workspace_manifest_digests(
    workspace: &RuntimeVmRemediationWorkspace,
    revision: Option<&LifecycleWorkspaceRevision>,
    runs: &[LifecycleRunSnapshot],
    promotion_runs: &[RuntimeVmRemediationRun],
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
    for run in promotion_runs {
        collect_manifest_digests_from_value(&run.metadata, &mut digests);
        if let Some(payload) = run.automation_payload.as_ref() {
            collect_manifest_digests_from_value(payload, &mut digests);
        }
        collect_manifest_digests_from_value(&run.promotion_gate_context, &mut digests);
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

async fn load_build_artifacts_by_digest(
    pool: &PgPool,
    manifest_digests: &HashSet<String>,
) -> Result<HashMap<String, BuildArtifactSummary>, AppError> {
    if manifest_digests.is_empty() {
        return Ok(HashMap::new());
    }

    let rows: Vec<BuildArtifactRow> = query_as(
        r#"
        SELECT manifest_digest, manifest_tag, registry_image, source_repo, source_revision, status, completed_at, duration_seconds
        FROM (
            SELECT
                manifest_digest,
                manifest_tag,
                registry_image,
                source_repo,
                source_revision,
                status,
                completed_at,
                CASE
                    WHEN completed_at IS NOT NULL AND started_at IS NOT NULL THEN
                        EXTRACT(EPOCH FROM (completed_at - started_at))::BIGINT
                    ELSE NULL
                END AS duration_seconds,
                ROW_NUMBER() OVER (
                    PARTITION BY manifest_digest
                    ORDER BY completed_at DESC NULLS LAST, started_at DESC
                ) AS row_number
            FROM build_artifact_runs
            WHERE manifest_digest = ANY($1)
        ) ranked
        WHERE ranked.row_number = 1
        "#,
    )
    .bind(manifest_digests.iter().cloned().collect::<Vec<_>>())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.manifest_digest.clone(),
                BuildArtifactSummary {
                    manifest_tag: row.manifest_tag,
                    registry_image: row.registry_image,
                    source_repo: row.source_repo,
                    source_revision: row.source_revision,
                    status: row.status,
                    completed_at: row.completed_at,
                    duration_seconds: row.duration_seconds,
                },
            )
        })
        .collect())
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

        let previous_promotion_runs = previous_snapshot
            .map(|s| s.promotion_runs.as_slice())
            .unwrap_or(&[]);
        let (promotion_run_deltas, removed_promotion_run_ids) =
            diff_promotion_runs(previous_promotion_runs, &snapshot.promotion_runs);

        let previous_promotions = previous_snapshot
            .map(|s| s.promotion_postures.as_slice())
            .unwrap_or(&[]);
        let (promotion_posture_deltas, removed_promotion_ids) =
            diff_promotion_postures(previous_promotions, &snapshot.promotion_postures);

        if !run_deltas.is_empty()
            || !removed_run_ids.is_empty()
            || !promotion_run_deltas.is_empty()
            || !removed_promotion_run_ids.is_empty()
            || !promotion_posture_deltas.is_empty()
            || !removed_promotion_ids.is_empty()
            || previous_snapshot.is_none()
        {
            workspaces.push(LifecycleWorkspaceDelta {
                workspace_id,
                run_deltas,
                removed_run_ids,
                promotion_run_deltas,
                removed_promotion_run_ids,
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
            let removed_promotion_run_ids = prev_snapshot
                .promotion_runs
                .iter()
                .map(|run| run.id)
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
                promotion_run_deltas: Vec::new(),
                removed_promotion_run_ids,
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
        let analytics_changes = diff_run_analytics(previous_snapshot, run);
        let artifact_changes = diff_run_artifacts(previous_snapshot, run);
        let previous_status = previous_snapshot.map(|s| s.run.status.clone());
        let has_changes = previous_snapshot.is_none()
            || previous_status.as_ref() != Some(&status)
            || !trust_changes.is_empty()
            || !intelligence_changes.is_empty()
            || !marketplace_changes.is_empty()
            || !analytics_changes.is_empty()
            || !artifact_changes.is_empty();

        if has_changes {
            deltas.push(LifecycleRunDelta {
                run_id,
                status,
                trust_changes,
                intelligence_changes,
                marketplace_changes,
                analytics_changes,
                artifact_changes,
            });
        }
    }

    let removed_run_ids = previous_map.into_keys().collect::<Vec<_>>();
    (deltas, removed_run_ids)
}

fn diff_promotion_runs(
    previous: &[RuntimeVmRemediationRun],
    current: &[RuntimeVmRemediationRun],
) -> (Vec<LifecyclePromotionRunDelta>, Vec<i64>) {
    let mut previous_map: HashMap<i64, &RuntimeVmRemediationRun> = HashMap::new();
    for run in previous {
        previous_map.insert(run.id, run);
    }

    let mut deltas = Vec::new();
    for run in current {
        let run_id = run.id;
        let previous_run = previous_map.remove(&run_id);
        let status = run.status.clone();

        let mut automation_payload_changes = Vec::new();
        let mut gate_context_changes = Vec::new();
        let mut metadata_changes = Vec::new();

        push_json_change(
            &mut automation_payload_changes,
            "promotion_run.automation_payload",
            previous_run
                .as_ref()
                .and_then(|prev| prev.automation_payload.as_ref()),
            run.automation_payload.as_ref(),
        );
        push_json_change(
            &mut gate_context_changes,
            "promotion_run.promotion_gate_context",
            previous_run
                .as_ref()
                .map(|prev| &prev.promotion_gate_context),
            Some(&run.promotion_gate_context),
        );
        push_json_change(
            &mut metadata_changes,
            "promotion_run.metadata",
            previous_run.as_ref().map(|prev| &prev.metadata),
            Some(&run.metadata),
        );

        let previous_status = previous_run.as_ref().map(|prev| prev.status.as_str());
        let has_changes = previous_run.is_none()
            || previous_status != Some(run.status.as_str())
            || !automation_payload_changes.is_empty()
            || !gate_context_changes.is_empty()
            || !metadata_changes.is_empty();

        if has_changes {
            deltas.push(LifecyclePromotionRunDelta {
                run_id,
                status,
                automation_payload_changes,
                gate_context_changes,
                metadata_changes,
            });
        }
    }

    let removed = previous_map.into_keys().collect::<Vec<_>>();
    (deltas, removed)
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
            push_change(
                &mut changes,
                "marketplace.manifest_digest",
                None,
                curr.manifest_digest.clone(),
            );
            push_change(
                &mut changes,
                "marketplace.manifest_tag",
                None,
                curr.manifest_tag.clone(),
            );
            push_change(
                &mut changes,
                "marketplace.registry_image",
                None,
                curr.registry_image.clone(),
            );
            push_change(
                &mut changes,
                "marketplace.build_duration_seconds",
                None,
                curr.build_duration_seconds.map(|value| value.to_string()),
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
            push_change(
                &mut changes,
                "marketplace.manifest_digest",
                prev.manifest_digest.clone(),
                curr.manifest_digest.clone(),
            );
            push_change(
                &mut changes,
                "marketplace.manifest_tag",
                prev.manifest_tag.clone(),
                curr.manifest_tag.clone(),
            );
            push_change(
                &mut changes,
                "marketplace.registry_image",
                prev.registry_image.clone(),
                curr.registry_image.clone(),
            );
            push_change(
                &mut changes,
                "marketplace.build_duration_seconds",
                prev.build_duration_seconds.map(|value| value.to_string()),
                curr.build_duration_seconds.map(|value| value.to_string()),
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
            push_change(
                &mut changes,
                "marketplace.manifest_digest",
                prev.manifest_digest.clone(),
                None,
            );
            push_change(
                &mut changes,
                "marketplace.manifest_tag",
                prev.manifest_tag.clone(),
                None,
            );
            push_change(
                &mut changes,
                "marketplace.registry_image",
                prev.registry_image.clone(),
                None,
            );
            push_change(
                &mut changes,
                "marketplace.build_duration_seconds",
                prev.build_duration_seconds.map(|value| value.to_string()),
                None,
            );
        }
    }
    changes
}

fn diff_run_analytics(
    previous: Option<&LifecycleRunSnapshot>,
    current: &LifecycleRunSnapshot,
) -> Vec<LifecycleFieldChange> {
    let mut changes = Vec::new();
    let previous_duration_ms = previous.and_then(|snap| snap.duration_ms);
    let current_duration_ms = current.duration_ms;
    push_change(
        &mut changes,
        "run.duration_ms",
        previous_duration_ms.map(|value| value.to_string()),
        current_duration_ms.map(|value| value.to_string()),
    );

    let previous_execution_start = previous
        .and_then(|snap| snap.execution_window.as_ref())
        .map(|window| window.started_at.to_rfc3339());
    let current_execution_start = current
        .execution_window
        .as_ref()
        .map(|window| window.started_at.to_rfc3339());
    push_change(
        &mut changes,
        "run.execution_started_at",
        previous_execution_start,
        current_execution_start,
    );

    let previous_execution_end = previous
        .and_then(|snap| snap.execution_window.as_ref())
        .and_then(|window| window.completed_at.map(|ts| ts.to_rfc3339()));
    let current_execution_end = current
        .execution_window
        .as_ref()
        .and_then(|window| window.completed_at.map(|ts| ts.to_rfc3339()));
    push_change(
        &mut changes,
        "run.execution_completed_at",
        previous_execution_end,
        current_execution_end,
    );

    let previous_duration = previous.and_then(|snap| snap.duration_seconds);
    let current_duration = current.duration_seconds;
    push_change(
        &mut changes,
        "run.duration_seconds",
        previous_duration.map(|value| value.to_string()),
        current_duration.map(|value| value.to_string()),
    );

    let previous_attempt = previous.and_then(|snap| snap.retry_attempt);
    let current_attempt = current.retry_attempt;
    push_change(
        &mut changes,
        "run.retry_attempt",
        previous_attempt.map(|value| value.to_string()),
        current_attempt.map(|value| value.to_string()),
    );

    let previous_limit = previous.and_then(|snap| snap.retry_limit);
    let current_limit = current.retry_limit;
    push_change(
        &mut changes,
        "run.retry_limit",
        previous_limit.map(|value| value.to_string()),
        current_limit.map(|value| value.to_string()),
    );

    let previous_retry_count = previous.and_then(|snap| snap.retry_count);
    let current_retry_count = current.retry_count;
    push_change(
        &mut changes,
        "run.retry_count",
        previous_retry_count.map(|value| value.to_string()),
        current_retry_count.map(|value| value.to_string()),
    );

    let previous_retry_ledger = previous
        .filter(|snap| !snap.retry_ledger.is_empty())
        .and_then(|snap| serde_json::to_string(&snap.retry_ledger).ok());
    let current_retry_ledger = if current.retry_ledger.is_empty() {
        None
    } else {
        serde_json::to_string(&current.retry_ledger).ok()
    };
    push_change(
        &mut changes,
        "run.retry_ledger",
        previous_retry_ledger,
        current_retry_ledger,
    );

    let previous_override = previous.and_then(|snap| snap.override_reason.clone());
    let current_override = current.override_reason.clone();
    push_change(
        &mut changes,
        "run.override_reason",
        previous_override,
        current_override,
    );

    let previous_override_actor = previous
        .and_then(|snap| snap.manual_override.as_ref())
        .and_then(|value| serde_json::to_string(value).ok());
    let current_override_actor = current
        .manual_override
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok());
    push_change(
        &mut changes,
        "run.override_actor",
        previous_override_actor,
        current_override_actor,
    );

    let previous_verdict = previous
        .and_then(|snap| snap.promotion_verdict.as_ref())
        .and_then(|value| serde_json::to_string(value).ok());
    let current_verdict = current
        .promotion_verdict
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok());
    push_change(
        &mut changes,
        "run.promotion_verdict",
        previous_verdict,
        current_verdict,
    );

    changes
}

fn diff_run_artifacts(
    previous: Option<&LifecycleRunSnapshot>,
    current: &LifecycleRunSnapshot,
) -> Vec<LifecycleFieldChange> {
    let mut changes = Vec::new();
    let previous_value = previous.and_then(|snapshot| {
        if snapshot.artifacts.is_empty() {
            None
        } else {
            to_value(&snapshot.artifacts).ok()
        }
    });
    let current_value = if current.artifacts.is_empty() {
        None
    } else {
        to_value(&current.artifacts).ok()
    };

    push_json_change(
        &mut changes,
        "run.artifacts",
        previous_value.as_ref(),
        current_value.as_ref(),
    );

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

fn push_json_change(
    changes: &mut Vec<LifecycleFieldChange>,
    field: &str,
    previous: Option<&Value>,
    current: Option<&Value>,
) {
    let previous_text = previous.map(|value| value.to_string());
    let current_text = current.map(|value| value.to_string());
    push_change(changes, field, previous_text, current_text);
}

fn format_float(value: f32) -> String {
    format!("{value:.4}")
}

fn compute_run_duration(run: &RuntimeVmRemediationRun) -> Option<i64> {
    let end_time = run.completed_at.or(run.cancelled_at).or_else(|| {
        if run.status == "running" || run.status == "pending" {
            Some(Utc::now())
        } else {
            None
        }
    });

    end_time.map(|end| {
        let duration = end.signed_duration_since(run.started_at);
        duration.num_seconds().max(0)
    })
}

fn compute_run_duration_ms(run: &RuntimeVmRemediationRun) -> Option<i64> {
    if let Some(value) = run.analytics_duration_ms {
        return Some(value.max(0));
    }

    let start = run.analytics_execution_started_at.unwrap_or(run.started_at);
    let end = run
        .analytics_execution_completed_at
        .or(run.completed_at)
        .or(run.cancelled_at)
        .or_else(|| {
            if run.status == "running" || run.status == "pending" {
                Some(Utc::now())
            } else {
                None
            }
        });

    end.map(|finish| {
        finish
            .signed_duration_since(start)
            .num_milliseconds()
            .max(0)
    })
}

fn build_execution_window(run: &RuntimeVmRemediationRun) -> Option<LifecycleRunExecutionWindow> {
    let started_at = run.analytics_execution_started_at.unwrap_or(run.started_at);
    let completed_at = run
        .analytics_execution_completed_at
        .or(run.completed_at)
        .or(run.cancelled_at);
    Some(LifecycleRunExecutionWindow {
        started_at,
        completed_at,
    })
}

fn compute_run_retry_attempt(run: &RuntimeVmRemediationRun) -> Option<i64> {
    if let Some(value) = run
        .automation_payload
        .as_ref()
        .and_then(|payload| search_for_integer(payload, "attempt"))
    {
        return Some(value);
    }
    search_for_integer(&run.metadata, "attempt")
}

fn compute_run_retry_limit(run: &RuntimeVmRemediationRun) -> Option<i64> {
    if let Some(value) = run
        .automation_payload
        .as_ref()
        .and_then(|payload| search_for_integer(payload, "retry_limit"))
    {
        return Some(value);
    }
    search_for_integer(&run.metadata, "retry_limit")
}

fn compute_retry_count(run: &RuntimeVmRemediationRun, retry_attempt: Option<i64>) -> Option<i64> {
    if let Some(value) = run.analytics_retry_count {
        return Some(i64::from(value.max(0)));
    }

    if let Some(Value::Array(entries)) = run.analytics_retry_ledger.as_ref() {
        if !entries.is_empty() {
            return Some(entries.len() as i64);
        }
    }

    retry_attempt
}

fn compute_run_override_reason(run: &RuntimeVmRemediationRun) -> Option<String> {
    if let Some(notes) = run.approval_notes.clone() {
        if !notes.is_empty() {
            return Some(notes);
        }
    }
    for key in ["override_reason", "manual_override", "override"] {
        if let Some(value) = search_for_string(&run.metadata, key) {
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    search_for_string(&run.promotion_gate_context, "override_reason")
}

fn build_retry_ledger(
    run: &RuntimeVmRemediationRun,
    retry_attempt: Option<i64>,
) -> Vec<LifecycleRunRetryRecord> {
    if let Some(Value::Array(entries)) = run.analytics_retry_ledger.as_ref() {
        let mut records = Vec::new();
        for entry in entries {
            if let Some(map) = entry.as_object() {
                if let Some(attempt) = map.get("attempt").and_then(Value::as_i64) {
                    let status = map
                        .get("status")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    let reason = map
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                    let observed_at = map
                        .get("observed_at")
                        .and_then(Value::as_str)
                        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                        .map(|dt| dt.with_timezone(&Utc));
                    records.push(LifecycleRunRetryRecord {
                        attempt,
                        status,
                        reason,
                        observed_at,
                    });
                }
            }
        }

        if !records.is_empty() {
            records.sort_by_key(|record| record.attempt);
            return records;
        }
    }

    let mut records = Vec::new();
    if let Some(attempt) = retry_attempt {
        let observed_at = run
            .analytics_execution_completed_at
            .or(run.completed_at)
            .or(run.cancelled_at)
            .unwrap_or(run.updated_at);
        records.push(LifecycleRunRetryRecord {
            attempt,
            status: Some(run.status.clone()),
            reason: run.failure_reason.clone(),
            observed_at: Some(observed_at),
        });
    }
    records
}

fn build_manual_override(
    run: &RuntimeVmRemediationRun,
    override_reason: Option<String>,
    actors: &HashMap<i32, OverrideActorRecord>,
) -> Option<LifecycleRunOverride> {
    let reason = override_reason?;
    if reason.trim().is_empty() {
        return None;
    }

    let actor_id = run.analytics_override_actor_id;
    let actor_email = actor_id.and_then(|id| actors.get(&id).map(|record| record.email.clone()));

    Some(LifecycleRunOverride {
        reason,
        actor_id,
        actor_email,
    })
}

fn derive_artifact_fingerprints(
    artifacts: &[LifecycleRunArtifact],
) -> Vec<LifecycleRunArtifactFingerprint> {
    let mut fingerprints = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for artifact in artifacts {
        let mut hasher = Sha256::new();
        hasher.update(artifact.manifest_digest.as_bytes());
        if let Some(lane) = &artifact.lane {
            hasher.update(lane.as_bytes());
        }
        if let Some(stage) = &artifact.stage {
            hasher.update(stage.as_bytes());
        }
        if let Some(tag) = &artifact.manifest_tag {
            hasher.update(tag.as_bytes());
        }
        if let Some(image) = &artifact.registry_image {
            hasher.update(image.as_bytes());
        }

        let digest = format!("{:x}", hasher.finalize());
        let key = format!("{}:{digest}", artifact.manifest_digest);
        if seen.insert(key) {
            fingerprints.push(LifecycleRunArtifactFingerprint {
                manifest_digest: artifact.manifest_digest.clone(),
                fingerprint: digest,
            });
        }
    }

    fingerprints
}

fn make_promotion_verdict_ref(
    verdict_id: i64,
    posture: Option<&LifecyclePromotionPosture>,
) -> LifecycleRunPromotionVerdictRef {
    if let Some(posture) = posture {
        LifecycleRunPromotionVerdictRef {
            verdict_id,
            promotion_id: Some(posture.promotion_id),
            allowed: Some(posture.allowed),
            stage: Some(posture.stage.clone()),
            track_name: Some(posture.track_name.clone()),
            track_tier: Some(posture.track_tier.clone()),
        }
    } else {
        LifecycleRunPromotionVerdictRef {
            verdict_id,
            promotion_id: None,
            allowed: None,
            stage: None,
            track_name: None,
            track_tier: None,
        }
    }
}

fn extract_run_artifacts(run: &RuntimeVmRemediationRun) -> Vec<LifecycleRunArtifact> {
    let promotion = run.metadata.get("promotion");
    let promotion_track = promotion.and_then(|value| value.get("track"));
    let promotion_stage = promotion
        .and_then(|value| value.get("stage"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let promotion_lane = promotion
        .and_then(|value| value.get("lane"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let promotion_source_repo = promotion
        .and_then(|value| value.get("source_repo"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let promotion_source_revision = promotion
        .and_then(|value| value.get("source_revision"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let track_name = promotion_track
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let track_tier = promotion_track
        .and_then(|value| value.get("tier"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());

    let gate_lane = run
        .promotion_gate_context
        .get("lane")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let gate_stage = run
        .promotion_gate_context
        .get("stage")
        .and_then(Value::as_str)
        .map(|value| value.to_string());

    let mut artifacts = Vec::new();
    let mut push_from_target = |target: &serde_json::Map<String, Value>| {
        if let Some(digest) = target.get("manifest_digest").and_then(Value::as_str) {
            if digest.is_empty() {
                return;
            }
            let lane = target
                .get("lane")
                .and_then(Value::as_str)
                .map(|value| value.to_string())
                .or_else(|| promotion_lane.clone())
                .or_else(|| gate_lane.clone());
            let stage = target
                .get("stage")
                .and_then(Value::as_str)
                .map(|value| value.to_string())
                .or_else(|| promotion_stage.clone())
                .or_else(|| gate_stage.clone());
            artifacts.push(LifecycleRunArtifact {
                manifest_digest: digest.to_string(),
                lane,
                stage,
                track_name: track_name.clone(),
                track_tier: track_tier.clone(),
                manifest_tag: None,
                registry_image: None,
                source_repo: promotion_source_repo.clone(),
                source_revision: promotion_source_revision.clone(),
                build_status: None,
                completed_at: None,
                duration_seconds: None,
            });
        }
    };

    if let Some(target) = run.metadata.get("target").and_then(Value::as_object) {
        push_from_target(target);
    }
    if let Some(targets) = run.metadata.get("targets").and_then(Value::as_array) {
        for entry in targets {
            if let Some(target) = entry.as_object() {
                push_from_target(target);
            }
        }
    }

    if artifacts.is_empty() {
        let mut digests = HashSet::new();
        collect_manifest_digests_from_value(&run.metadata, &mut digests);
        collect_manifest_digests_from_value(&run.promotion_gate_context, &mut digests);
        for digest in digests {
            artifacts.push(LifecycleRunArtifact {
                manifest_digest: digest,
                lane: promotion_lane.clone().or_else(|| gate_lane.clone()),
                stage: promotion_stage.clone().or_else(|| gate_stage.clone()),
                track_name: track_name.clone(),
                track_tier: track_tier.clone(),
                manifest_tag: None,
                registry_image: None,
                source_repo: promotion_source_repo.clone(),
                source_revision: promotion_source_revision.clone(),
                build_status: None,
                completed_at: None,
                duration_seconds: None,
            });
        }
    }

    artifacts
}

fn enrich_run_artifacts(
    artifacts: &mut Vec<LifecycleRunArtifact>,
    artifact_map: &HashMap<String, BuildArtifactSummary>,
) {
    for artifact in artifacts {
        if let Some(summary) = artifact_map.get(&artifact.manifest_digest) {
            if artifact.manifest_tag.is_none() {
                artifact.manifest_tag = summary.manifest_tag.clone();
            }
            if artifact.registry_image.is_none() {
                artifact.registry_image = summary.registry_image.clone();
            }
            if artifact.source_repo.is_none() {
                artifact.source_repo = summary.source_repo.clone();
            }
            if artifact.source_revision.is_none() {
                artifact.source_revision = summary.source_revision.clone();
            }
            artifact.build_status = Some(summary.status.clone());
            artifact.completed_at = summary.completed_at;
            artifact.duration_seconds = summary.duration_seconds;
        }
    }
}

fn search_for_integer(value: &Value, key: &str) -> Option<i64> {
    match value {
        Value::Object(map) => {
            if let Some(entry) = map.get(key) {
                if let Some(num) = entry.as_i64() {
                    return Some(num);
                }
            }
            for entry in map.values() {
                if let Some(num) = search_for_integer(entry, key) {
                    return Some(num);
                }
            }
            None
        }
        Value::Array(items) => {
            for entry in items {
                if let Some(num) = search_for_integer(entry, key) {
                    return Some(num);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use std::collections::HashMap;

    fn base_run() -> RuntimeVmRemediationRun {
        let now = Utc::now();
        RuntimeVmRemediationRun {
            id: 1,
            runtime_vm_instance_id: 99,
            playbook: "lifecycle".to_string(),
            playbook_id: None,
            status: "succeeded".to_string(),
            automation_payload: None,
            approval_required: false,
            started_at: now,
            completed_at: Some(now),
            last_error: None,
            assigned_owner_id: None,
            sla_deadline: None,
            approval_state: "auto-approved".to_string(),
            approval_decided_at: Some(now),
            approval_notes: None,
            metadata: json!({}),
            workspace_id: Some(7),
            workspace_revision_id: None,
            promotion_gate_context: json!({}),
            version: 1,
            updated_at: now,
            cancelled_at: None,
            cancellation_reason: None,
            failure_reason: None,
            analytics_duration_ms: None,
            analytics_execution_started_at: None,
            analytics_execution_completed_at: None,
            analytics_retry_count: None,
            analytics_retry_ledger: None,
            analytics_override_actor_id: None,
            analytics_artifact_hash: None,
            analytics_promotion_verdict_id: None,
        }
    }

    #[test]
    fn extract_run_artifacts_prefers_target_metadata() {
        let mut run = base_run();
        run.metadata = json!({
            "promotion": {
                "stage": "staging",
                "lane": "blue",
                "track": {
                    "name": "release-alpha",
                    "tier": "pilot"
                },
                "source_repo": "git@example.com/demo.git",
                "source_revision": "abcdef"
            },
            "targets": [
                {
                    "manifest_digest": "sha256:target",
                    "lane": "green",
                    "stage": "production",
                    "manifest_tag": "v1"
                }
            ]
        });
        run.promotion_gate_context = json!({
            "lane": "fallback-lane",
            "stage": "fallback-stage"
        });

        let artifacts = extract_run_artifacts(&run);
        assert_eq!(artifacts.len(), 1);
        let artifact = &artifacts[0];
        assert_eq!(artifact.manifest_digest, "sha256:target");
        assert_eq!(artifact.lane.as_deref(), Some("green"));
        assert_eq!(artifact.stage.as_deref(), Some("production"));
        assert_eq!(artifact.track_name.as_deref(), Some("release-alpha"));
        assert_eq!(artifact.track_tier.as_deref(), Some("pilot"));
        assert_eq!(
            artifact.source_repo.as_deref(),
            Some("git@example.com/demo.git")
        );
        assert_eq!(artifact.source_revision.as_deref(), Some("abcdef"));
    }

    #[test]
    fn extract_run_artifacts_falls_back_to_gate_context() {
        let mut run = base_run();
        run.metadata = json!({
            "promotion": {
                "track": {
                    "name": "release-beta"
                },
                "source_repo": "git@example.com/demo.git"
            },
            "automation": {
                "result": {
                    "image": {
                        "manifest_digest": "sha256:fallback"
                    }
                }
            }
        });
        run.promotion_gate_context = json!({
            "lane": "gate-lane",
            "stage": "gate-stage"
        });

        let artifacts = extract_run_artifacts(&run);
        assert_eq!(artifacts.len(), 1);
        let artifact = &artifacts[0];
        assert_eq!(artifact.manifest_digest, "sha256:fallback");
        assert_eq!(artifact.lane.as_deref(), Some("gate-lane"));
        assert_eq!(artifact.stage.as_deref(), Some("gate-stage"));
        assert_eq!(artifact.track_name.as_deref(), Some("release-beta"));
        assert_eq!(
            artifact.source_repo.as_deref(),
            Some("git@example.com/demo.git")
        );
    }

    #[test]
    fn enrich_run_artifacts_merges_summary_details() {
        let timestamp = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let mut artifacts = vec![LifecycleRunArtifact {
            manifest_digest: "sha256:artifact".to_string(),
            lane: None,
            stage: None,
            track_name: None,
            track_tier: None,
            manifest_tag: None,
            registry_image: None,
            source_repo: None,
            source_revision: None,
            build_status: None,
            completed_at: None,
            duration_seconds: None,
        }];
        let mut artifact_map = HashMap::new();
        artifact_map.insert(
            "sha256:artifact".to_string(),
            BuildArtifactSummary {
                manifest_tag: Some("v2".to_string()),
                registry_image: Some("registry/app:v2".to_string()),
                source_repo: Some("git@example.com/demo.git".to_string()),
                source_revision: Some("1234567".to_string()),
                status: "succeeded".to_string(),
                completed_at: Some(timestamp),
                duration_seconds: Some(95),
            },
        );

        enrich_run_artifacts(&mut artifacts, &artifact_map);

        let artifact = &artifacts[0];
        assert_eq!(artifact.manifest_tag.as_deref(), Some("v2"));
        assert_eq!(artifact.registry_image.as_deref(), Some("registry/app:v2"));
        assert_eq!(
            artifact.source_repo.as_deref(),
            Some("git@example.com/demo.git")
        );
        assert_eq!(artifact.source_revision.as_deref(), Some("1234567"));
        assert_eq!(artifact.build_status.as_deref(), Some("succeeded"));
        assert_eq!(artifact.completed_at, Some(timestamp));
        assert_eq!(artifact.duration_seconds, Some(95));
    }
}

fn search_for_string(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(entry) = map.get(key) {
                if let Some(text) = entry.as_str() {
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            for entry in map.values() {
                if let Some(text) = search_for_string(entry, key) {
                    return Some(text);
                }
            }
            None
        }
        Value::Array(items) => {
            for entry in items {
                if let Some(text) = search_for_string(entry, key) {
                    return Some(text);
                }
            }
            None
        }
        _ => None,
    }
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
        SELECT server_id, status, completed_at, manifest_digest, manifest_tag, registry_image, duration_seconds
        FROM (
            SELECT
                server_id,
                status,
                completed_at,
                manifest_digest,
                manifest_tag,
                registry_image,
                CASE
                    WHEN completed_at IS NOT NULL AND started_at IS NOT NULL THEN
                        EXTRACT(EPOCH FROM (completed_at - started_at))::BIGINT
                    ELSE NULL
                END AS duration_seconds,
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
                    manifest_digest: row.manifest_digest,
                    manifest_tag: row.manifest_tag,
                    registry_image: row.registry_image,
                    build_duration_seconds: row.duration_seconds,
                },
            )
        })
        .collect())
}
