use std::convert::Infallible;

use axum::{
    extract::{Extension, Path, Query},
    response::sse::{Event, Sse},
    Json,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{postgres::PgListener, Executor, FromRow, PgPool, Postgres, QueryBuilder, Row};
use tokio::sync::{broadcast, mpsc::Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, error, warn};

use crate::{
    db::runtime_vm_trust_history::{
        history_for_instance as history_for_vm, insert_trust_event, NewRuntimeVmTrustEvent,
        RuntimeVmTrustEvent,
    },
    db::runtime_vm_trust_registry::{
        upsert_state as upsert_registry_state, UpsertRuntimeVmTrustRegistryState,
    },
    error::{AppError, AppResult},
    evaluations::scheduler::{self, TrustTransitionSignal},
    extractor::AuthUser,
    job_queue::{self, Job},
};

const TRUST_CHANNEL: &str = "runtime_vm_trust_transition";

// key: trust-control -> event-channel
static TRUST_EVENT_CHANNEL: Lazy<broadcast::Sender<TrustRegistryEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(128);
    tx
});

#[derive(Debug, Clone, Serialize)]
pub struct TrustRegistryEvent {
    #[serde(skip_serializing)]
    pub owner_id: i32,
    pub server_id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    pub vm_instance_id: i64,
    pub instance_id: String,
    pub attestation_status: String,
    pub lifecycle_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_attestation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_lifecycle_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation_state: Option<String>,
    pub remediation_attempts: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_deadline: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_reason: Option<String>,
    pub triggered_at: DateTime<Utc>,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustRegistryView {
    pub server_id: i32,
    pub server_name: String,
    pub vm_instance_id: i64,
    pub instance_id: String,
    pub attestation_status: String,
    pub lifecycle_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation_state: Option<String>,
    pub remediation_attempts: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_deadline: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
    pub version: i64,
    pub updated_at: DateTime<Utc>,
    pub stale: bool,
}

#[derive(Debug, Serialize)]
pub struct TrustHistoryResponse {
    pub server_id: i32,
    pub server_name: String,
    pub instance_id: String,
    pub events: Vec<RuntimeVmTrustEvent>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TrustRegistryQuery {
    #[serde(default)]
    pub server_id: Option<i32>,
    #[serde(default)]
    pub lifecycle_state: Option<String>,
    #[serde(default)]
    pub attestation_status: Option<String>,
    #[serde(default)]
    pub stale: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TrustRegistryTransitionRequest {
    pub attestation_status: String,
    pub lifecycle_state: String,
    pub remediation_state: Option<String>,
    pub remediation_attempts: Option<i32>,
    pub freshness_deadline: Option<DateTime<Utc>>,
    pub provenance_ref: Option<String>,
    pub provenance: Option<Value>,
    pub transition_reason: Option<String>,
    pub metadata: Option<Value>,
    pub expected_version: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TrustWatchParams {
    #[serde(default)]
    pub server_id: Option<i32>,
    #[serde(default)]
    pub lifecycle_state: Option<String>,
    #[serde(default)]
    pub attestation_status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TrustHistoryQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TrustNotification {
    runtime_vm_instance_id: i64,
    attestation_id: Option<i64>,
    previous_status: Option<String>,
    current_status: String,
    previous_lifecycle_state: Option<String>,
    current_lifecycle_state: String,
    transition_reason: Option<String>,
    remediation_state: Option<String>,
    remediation_attempts: Option<i32>,
    freshness_deadline: Option<chrono::DateTime<Utc>>,
    provenance_ref: Option<String>,
    provenance: Option<Value>,
    triggered_at: chrono::DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct TrustRegistryRow {
    server_id: i32,
    server_name: String,
    vm_instance_id: i64,
    instance_id: String,
    attestation_status: String,
    lifecycle_state: String,
    remediation_state: Option<String>,
    remediation_attempts: i32,
    freshness_deadline: Option<DateTime<Utc>>,
    provenance_ref: Option<String>,
    provenance: Option<Value>,
    version: i64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct VmContextRow {
    owner_id: i32,
    server_id: i32,
    server_name: String,
    instance_id: String,
    attestation_status: Option<String>,
    lifecycle_state: Option<String>,
    remediation_state: Option<String>,
    remediation_attempts: Option<i32>,
    freshness_deadline: Option<DateTime<Utc>>,
    provenance_ref: Option<String>,
    provenance: Option<Value>,
    version: Option<i64>,
}

const REGISTRY_BASE_QUERY: &str = r#"
    SELECT
        registry.runtime_vm_instance_id AS vm_instance_id,
        registry.attestation_status,
        registry.lifecycle_state,
        registry.remediation_state,
        registry.remediation_attempts,
        registry.freshness_deadline,
        registry.provenance_ref,
        registry.provenance,
        registry.version,
        registry.updated_at,
        servers.id AS server_id,
        servers.name AS server_name,
        instances.instance_id
    FROM runtime_vm_trust_registry registry
    JOIN runtime_vm_instances instances ON instances.id = registry.runtime_vm_instance_id
    JOIN mcp_servers servers ON servers.id = instances.server_id
"#;

const VM_CONTEXT_QUERY: &str = r#"
    SELECT
        servers.owner_id,
        servers.id AS server_id,
        servers.name AS server_name,
        instances.instance_id,
        registry.attestation_status,
        registry.lifecycle_state,
        registry.remediation_state,
        registry.remediation_attempts,
        registry.freshness_deadline,
        registry.provenance_ref,
        registry.provenance,
        registry.version
    FROM runtime_vm_instances instances
    JOIN mcp_servers servers ON servers.id = instances.server_id
    LEFT JOIN runtime_vm_trust_registry registry
        ON registry.runtime_vm_instance_id = instances.id
    WHERE instances.id = $1
"#;

impl From<TrustRegistryRow> for TrustRegistryView {
    fn from(row: TrustRegistryRow) -> Self {
        let TrustRegistryRow {
            server_id,
            server_name,
            vm_instance_id,
            instance_id,
            attestation_status,
            lifecycle_state,
            remediation_state,
            remediation_attempts,
            freshness_deadline,
            provenance_ref,
            provenance,
            version,
            updated_at,
        } = row;
        let stale = compute_stale(freshness_deadline);
        Self {
            server_id,
            server_name,
            vm_instance_id,
            instance_id,
            attestation_status,
            lifecycle_state,
            remediation_state,
            remediation_attempts,
            freshness_deadline,
            provenance_ref,
            provenance,
            version,
            updated_at,
            stale,
        }
    }
}

fn publish_trust_event(event: TrustRegistryEvent) {
    let _ = TRUST_EVENT_CHANNEL.send(event);
}

pub fn subscribe_registry_events() -> broadcast::Receiver<TrustRegistryEvent> {
    TRUST_EVENT_CHANNEL.subscribe()
}

fn compute_stale(deadline: Option<DateTime<Utc>>) -> bool {
    matches!(deadline, Some(ts) if ts < Utc::now())
}

fn normalize_attestation_status(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "trusted" | "untrusted" | "unknown")
        .then_some(normalized)
}

fn normalize_lifecycle_state(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "suspect" | "quarantined" | "remediating" | "restored"
    )
    .then_some(normalized)
}

fn matches_filter(filter: &Option<String>, candidate: &str) -> bool {
    filter.as_ref().map(|expected| expected == candidate).unwrap_or(true)
}

async fn load_vm_context<'c, E>(
    executor: E,
    vm_instance_id: i64,
) -> Result<Option<VmContextRow>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query_as::<_, VmContextRow>(VM_CONTEXT_QUERY)
        .bind(vm_instance_id)
        .fetch_optional(executor)
        .await
}

#[cfg(test)]
mod tests {
    // key: trust-control -> unit-tests
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn compute_stale_recognizes_deadlines() {
        let past = Utc::now() - Duration::seconds(5);
        let future = Utc::now() + Duration::seconds(30);
        assert!(compute_stale(Some(past)));
        assert!(!compute_stale(Some(future)));
        assert!(!compute_stale(None));
    }

    #[test]
    fn normalize_attestation_status_accepts_known_values() {
        assert_eq!(normalize_attestation_status(" Trusted "), Some("trusted".into()));
        assert_eq!(normalize_attestation_status("UNTRUSTED"), Some("untrusted".into()));
        assert_eq!(normalize_attestation_status("unknown"), Some("unknown".into()));
        assert_eq!(normalize_attestation_status("invalid"), None);
    }

    #[test]
    fn normalize_lifecycle_state_accepts_expected_states() {
        assert_eq!(normalize_lifecycle_state(" Suspect"), Some("suspect".into()));
        assert_eq!(normalize_lifecycle_state("QUARANTINED"), Some("quarantined".into()));
        assert_eq!(normalize_lifecycle_state("remediating"), Some("remediating".into()));
        assert_eq!(normalize_lifecycle_state("restored"), Some("restored".into()));
        assert_eq!(normalize_lifecycle_state("unknown"), None);
    }

    #[test]
    fn matches_filter_evaluates_candidates() {
        assert!(matches_filter(&None, "anything"));
        assert!(matches_filter(&Some("trusted".into()), "trusted"));
        assert!(!matches_filter(&Some("trusted".into()), "untrusted"));
    }
}

async fn fetch_registry_view_for_vm(
    pool: &PgPool,
    user_id: i32,
    vm_instance_id: i64,
) -> AppResult<TrustRegistryView> {
    let sql = format!(
        "{} WHERE servers.owner_id = $1 AND registry.runtime_vm_instance_id = $2",
        REGISTRY_BASE_QUERY
    );
    let row = sqlx::query_as::<_, TrustRegistryRow>(&sql)
        .bind(user_id)
        .bind(vm_instance_id)
        .fetch_optional(pool)
        .await?;

    row.map(TrustRegistryView::from)
        .ok_or(AppError::NotFound)
}

pub fn spawn_trust_listener(pool: PgPool, job_tx: Sender<Job>) {
    tokio::spawn(async move {
        if let Err(err) = listen(pool, job_tx).await {
            error!(?err, "trust transition listener terminated");
        }
    });
}

async fn listen(pool: PgPool, job_tx: Sender<Job>) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(TRUST_CHANNEL).await?;

    loop {
        let notification = listener.recv().await?;
        let payload = notification.payload();
        match serde_json::from_str::<TrustNotification>(payload) {
            Ok(message) => {
                debug!(?message, "received trust transition notification");
                let instance_row = sqlx::query(
                    r#"
                    SELECT
                        instances.server_id,
                        instances.instance_id,
                        servers.owner_id,
                        servers.name AS server_name
                    FROM runtime_vm_instances instances
                    JOIN mcp_servers servers ON servers.id = instances.server_id
                    WHERE instances.id = $1
                    "#,
                )
                .bind(message.runtime_vm_instance_id)
                .fetch_optional(&pool)
                .await?;

                let Some(instance_row) = instance_row else {
                    warn!(
                        vm_instance_id = message.runtime_vm_instance_id,
                        "ignoring trust notification for missing runtime VM instance"
                    );
                    continue;
                };

                let server_id: i32 = instance_row.get("server_id");
                let owner_id: i32 = instance_row.get("owner_id");
                let server_name: String = instance_row.get("server_name");
                let instance_id: String = instance_row.get("instance_id");
                let stale = compute_stale(message.freshness_deadline);
                publish_trust_event(TrustRegistryEvent {
                    owner_id,
                    server_id,
                    server_name: Some(server_name),
                    vm_instance_id: message.runtime_vm_instance_id,
                    instance_id,
                    attestation_status: message.current_status.clone(),
                    lifecycle_state: message.current_lifecycle_state.clone(),
                    previous_attestation_status: message.previous_status.clone(),
                    previous_lifecycle_state: message.previous_lifecycle_state.clone(),
                    remediation_state: message.remediation_state.clone(),
                    remediation_attempts: message.remediation_attempts.unwrap_or_default(),
                    freshness_deadline: message.freshness_deadline,
                    provenance_ref: message.provenance_ref.clone(),
                    provenance: message.provenance.clone(),
                    transition_reason: message.transition_reason.clone(),
                    triggered_at: message.triggered_at,
                    stale,
                });
                let signal = TrustTransitionSignal {
                    server_id,
                    vm_instance_id: message.runtime_vm_instance_id,
                    current_status: message.current_status.clone(),
                    previous_status: message.previous_status.clone(),
                    lifecycle_state: message.current_lifecycle_state.clone(),
                    previous_lifecycle_state: message.previous_lifecycle_state.clone(),
                    transition_reason: message.transition_reason.clone(),
                    remediation_state: message.remediation_state.clone(),
                    triggered_at: message.triggered_at,
                    freshness_expires_at: message.freshness_deadline,
                    remediation_attempts: message.remediation_attempts.unwrap_or_default(),
                    provenance_ref: message.provenance_ref.clone(),
                    provenance: message.provenance.clone(),
                    posture_changed: message
                        .previous_status
                        .as_deref()
                        .map(|status| status != message.current_status)
                        .unwrap_or(true),
                };

                if let Err(err) = scheduler::handle_trust_transition(&pool, &job_tx, &signal).await
                {
                    warn!(
                        ?err,
                        server_id = signal.server_id,
                        vm_instance_id = signal.vm_instance_id,
                        "failed to apply trust transition"
                    );
                }

                job_queue::enqueue_intelligence_refresh(&pool, signal.server_id).await;
            }
            Err(err) => warn!(?err, payload, "failed to parse trust notification payload"),
        }
    }
}

// key: trust-control -> rest-endpoints
pub async fn list_registry_states(
    AuthUser { user_id, .. }: AuthUser,
    Query(query): Query<TrustRegistryQuery>,
    Extension(pool): Extension<PgPool>,
) -> AppResult<Json<Vec<TrustRegistryView>>> {
    let TrustRegistryQuery {
        server_id,
        lifecycle_state,
        attestation_status,
        stale,
    } = query;

    let lifecycle_filter = match lifecycle_state {
        Some(value) => Some(
            normalize_lifecycle_state(&value).ok_or_else(|| {
                AppError::BadRequest(format!("invalid lifecycle_state '{value}'"))
            })?,
        ),
        None => None,
    };
    let status_filter = match attestation_status {
        Some(value) => Some(
            normalize_attestation_status(&value).ok_or_else(|| {
                AppError::BadRequest(format!("invalid attestation_status '{value}'"))
            })?,
        ),
        None => None,
    };

    let mut builder = QueryBuilder::<Postgres>::new(format!(
        "{} WHERE servers.owner_id = ",
        REGISTRY_BASE_QUERY
    ));
    builder.push_bind(user_id);
    if let Some(server_id) = server_id {
        builder.push(" AND servers.id = ");
        builder.push_bind(server_id);
    }
    if let Some(state) = lifecycle_filter.as_ref() {
        builder.push(" AND registry.lifecycle_state = ");
        builder.push_bind(state);
    }
    if let Some(status) = status_filter.as_ref() {
        builder.push(" AND registry.attestation_status = ");
        builder.push_bind(status);
    }
    if let Some(stale_filter) = stale {
        if stale_filter {
            builder.push(
                " AND registry.freshness_deadline IS NOT NULL AND registry.freshness_deadline < NOW()",
            );
        } else {
            builder.push(
                " AND (registry.freshness_deadline IS NULL OR registry.freshness_deadline >= NOW())",
            );
        }
    }
    builder.push(" ORDER BY registry.updated_at DESC");

    let rows: Vec<TrustRegistryRow> = builder
        .build_query_as::<TrustRegistryRow>()
        .fetch_all(&pool)
        .await?;
    let entries = rows.into_iter().map(TrustRegistryView::from).collect();
    Ok(Json(entries))
}

pub async fn get_registry_state(
    AuthUser { user_id, .. }: AuthUser,
    Path(vm_instance_id): Path<i64>,
    Extension(pool): Extension<PgPool>,
) -> AppResult<Json<TrustRegistryView>> {
    let view = fetch_registry_view_for_vm(&pool, user_id, vm_instance_id).await?;
    Ok(Json(view))
}

pub async fn get_registry_history(
    AuthUser { user_id, .. }: AuthUser,
    Path(vm_instance_id): Path<i64>,
    Query(query): Query<TrustHistoryQuery>,
    Extension(pool): Extension<PgPool>,
) -> AppResult<Json<TrustHistoryResponse>> {
    let context = load_vm_context(&pool, vm_instance_id).await?;
    let Some(context) = context else {
        return Err(AppError::NotFound);
    };
    if context.owner_id != user_id {
        return Err(AppError::Forbidden);
    }

    let limit = query.limit.unwrap_or(25).clamp(1, 200);
    let events = history_for_vm(&pool, vm_instance_id, limit).await?;
    Ok(Json(TrustHistoryResponse {
        server_id: context.server_id,
        server_name: context.server_name,
        instance_id: context.instance_id,
        events,
    }))
}

pub async fn transition_registry_state(
    AuthUser { user_id, .. }: AuthUser,
    Path(vm_instance_id): Path<i64>,
    Json(payload): Json<TrustRegistryTransitionRequest>,
    Extension(pool): Extension<PgPool>,
) -> AppResult<Json<TrustRegistryView>> {
    let TrustRegistryTransitionRequest {
        attestation_status,
        lifecycle_state,
        remediation_state,
        remediation_attempts,
        freshness_deadline,
        provenance_ref,
        provenance,
        transition_reason,
        metadata,
        expected_version,
    } = payload;

    let attestation_status = normalize_attestation_status(&attestation_status).ok_or_else(|| {
        AppError::BadRequest(format!("invalid attestation_status '{attestation_status}'"))
    })?;
    let lifecycle_state = normalize_lifecycle_state(&lifecycle_state).ok_or_else(|| {
        AppError::BadRequest(format!("invalid lifecycle_state '{lifecycle_state}'"))
    })?;

    let mut tx = pool.begin().await?;
    let context = load_vm_context(&mut *tx, vm_instance_id).await?;
    let Some(context) = context else {
        return Err(AppError::NotFound);
    };
    if context.owner_id != user_id {
        return Err(AppError::Forbidden);
    }

    let previous_status = context.attestation_status.clone();
    let previous_lifecycle = context.lifecycle_state.clone();
    let current_attempts = context.remediation_attempts.unwrap_or(0);
    let attempts = remediation_attempts.unwrap_or(current_attempts);
    if attempts < 0 {
        return Err(AppError::BadRequest("remediation_attempts must be non-negative".into()));
    }

    let expected_version = match (expected_version, context.version) {
        (Some(expected), Some(current)) if expected == current => Some(expected),
        (Some(expected), Some(current)) => {
            return Err(AppError::Conflict(format!(
                "trust registry version mismatch: expected {expected}, found {current}"
            )))
        }
        (None, Some(current)) => {
            return Err(AppError::Conflict(format!(
                "trust registry version required; current version is {current}"
            )))
        }
        (value, None) => value,
    };

    let reason = transition_reason.unwrap_or_else(|| "manual".to_string());
    let registry = match upsert_registry_state(
        &mut *tx,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: vm_instance_id,
            attestation_status: attestation_status.as_str(),
            lifecycle_state: lifecycle_state.as_str(),
            remediation_state: remediation_state.as_deref(),
            remediation_attempts: attempts,
            freshness_deadline,
            provenance_ref: provenance_ref.as_deref(),
            provenance: provenance.as_ref(),
            expected_version,
        },
    )
    .await
    {
        Ok(state) => state,
        Err(sqlx::Error::RowNotFound) => {
            return Err(AppError::Conflict(
                "trust registry version mismatch during update".into(),
            ))
        }
        Err(err) => return Err(err.into()),
    };

    insert_trust_event(
        &mut *tx,
        NewRuntimeVmTrustEvent {
            runtime_vm_instance_id: vm_instance_id,
            attestation_id: None,
            previous_status: previous_status.as_deref(),
            current_status: registry.attestation_status.as_str(),
            previous_lifecycle_state: previous_lifecycle.as_deref(),
            current_lifecycle_state: registry.lifecycle_state.as_str(),
            transition_reason: Some(reason.as_str()),
            remediation_state: registry.remediation_state.as_deref(),
            remediation_attempts: attempts,
            freshness_deadline,
            provenance_ref: registry.provenance_ref.as_deref(),
            provenance: provenance.as_ref(),
            metadata: metadata.as_ref(),
        },
    )
    .await?;

    tx.commit().await?;

    let view = fetch_registry_view_for_vm(&pool, user_id, vm_instance_id).await?;
    Ok(Json(view))
}

pub async fn stream_trust_events(
    AuthUser { user_id, .. }: AuthUser,
    Query(params): Query<TrustWatchParams>,
) -> AppResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>> {
    let lifecycle_filter = match params.lifecycle_state {
        Some(value) => Some(
            normalize_lifecycle_state(&value).ok_or_else(|| {
                AppError::BadRequest(format!("invalid lifecycle_state '{value}'"))
            })?,
        ),
        None => None,
    };
    let status_filter = match params.attestation_status {
        Some(value) => Some(
            normalize_attestation_status(&value).ok_or_else(|| {
                AppError::BadRequest(format!("invalid attestation_status '{value}'"))
            })?,
        ),
        None => None,
    };

    let server_filter = params.server_id;
    let receiver = subscribe_registry_events();
    let stream = BroadcastStream::new(receiver).filter_map(move |item| {
        let lifecycle_filter = lifecycle_filter.clone();
        let status_filter = status_filter.clone();
        async move {
            match item {
                Ok(event) if event.owner_id == user_id => {
                    if let Some(server_id) = server_filter {
                        if event.server_id != server_id {
                            return None;
                        }
                    }
                    if !matches_filter(&lifecycle_filter, event.lifecycle_state.as_str()) {
                        return None;
                    }
                    if !matches_filter(&status_filter, event.attestation_status.as_str()) {
                        return None;
                    }
                    match serde_json::to_string(&event) {
                        Ok(payload) => Some(Ok(Event::default().data(payload))),
                        Err(err) => {
                            tracing::error!(?err, "failed to serialize trust event");
                            None
                        }
                    }
                }
                Ok(_) => None,
                Err(err) => {
                    tracing::debug!(?err, "dropped trust event subscriber update");
                    None
                }
            }
        }
    });

    Ok(Sse::new(stream))
}
