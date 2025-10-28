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
                    let envelope = LifecycleConsoleEventEnvelope {
                        event_type: LifecycleConsoleEventType::Snapshot,
                        emitted_at: Utc::now(),
                        cursor: event_cursor,
                        page: Some(page.clone()),
                        error: None,
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

        snapshots.push(LifecycleWorkspaceSnapshot {
            workspace,
            active_revision: revision,
            recent_runs: run_snapshots,
        });
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
