use axum::{
    extract::{Extension, Path, Query},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::convert::Infallible;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::extractor::AuthUser;
use crate::keys::{
    ProviderKeyPolicySummary, ProviderKeyService, ProviderKeyServiceConfig, ProviderTierRequirement,
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::warn;

const MARKETPLACE_BROADCAST_BUFFER: usize = 128;

static MARKETPLACE_EVENT_CHANNEL: Lazy<broadcast::Sender<ProviderMarketplaceStreamEvent>> =
    Lazy::new(|| {
        let (tx, _rx) = broadcast::channel(MARKETPLACE_BROADCAST_BUFFER);
        tx
    });

// key: marketplace-catalog -> artifact-ledger,policy-tiering

#[derive(Debug, Deserialize, Default)]
pub struct MarketplaceQuery {
    pub server_type: Option<String>,
    pub status: Option<String>,
    pub tier: Option<String>,
    pub q: Option<String>,
    pub limit: Option<u32>,
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/marketplace", get(list_marketplace))
        .route(
            "/api/marketplace/providers/:provider_id/submissions",
            get(list_provider_submissions).post(create_provider_submission),
        )
        .route(
            "/api/marketplace/providers/:provider_id/events/stream",
            get(stream_provider_marketplace_events),
        )
        .route(
            "/api/marketplace/providers/:provider_id/submissions/:submission_id/evaluations",
            post(create_evaluation_run),
        )
        .route(
            "/api/marketplace/providers/:provider_id/evaluations/:evaluation_id/transition",
            post(transition_evaluation_run),
        )
        .route(
            "/api/marketplace/providers/:provider_id/evaluations/:evaluation_id/promotions",
            post(create_promotion_gate),
        )
        .route(
            "/api/marketplace/providers/:provider_id/promotions/:promotion_id/transition",
            post(transition_promotion_gate),
        )
}

pub fn subscribe_marketplace_events() -> broadcast::Receiver<ProviderMarketplaceStreamEvent> {
    MARKETPLACE_EVENT_CHANNEL.subscribe()
}

fn publish_marketplace_event(event: ProviderMarketplaceStreamEvent) {
    let _ = MARKETPLACE_EVENT_CHANNEL.send(event);
}

async fn stream_provider_marketplace_events(
    Path(provider_id): Path<Uuid>,
    _user: AuthUser,
) -> AppResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>> {
    let receiver = subscribe_marketplace_events();
    let stream = BroadcastStream::new(receiver).filter_map(move |entry| {
        let provider_filter = provider_id;
        match entry {
            Ok(event) if event.provider_id == provider_filter => {
                match serde_json::to_string(&event) {
                    Ok(serialized) => Some(Ok(Event::default().data(serialized))),
                    Err(error) => {
                        warn!("failed to serialize marketplace event for stream: {error}");
                        None
                    }
                }
            }
            Ok(_) => None,
            Err(error) => {
                warn!("marketplace event stream receiver lagged: {error}");
                None
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketplacePlatform {
    pub platform: String,
    pub remote_image: String,
    pub remote_tag: String,
    pub digest: Option<String>,
    pub auth_refresh_attempted: bool,
    pub auth_refresh_succeeded: bool,
    pub auth_rotation_attempted: bool,
    pub auth_rotation_succeeded: bool,
    pub credential_health_status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketplaceVmInstance {
    pub instance_id: String,
    pub isolation_tier: Option<String>,
    pub attestation_status: String,
    pub policy_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminated_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub capability_notes: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ArtifactHealth {
    pub overall: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketplacePromotion {
    pub track_id: i32,
    pub track_name: String,
    pub stage: String,
    pub status: String,
    pub updated_at: DateTime<Utc>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MarketplaceArtifact {
    pub server_id: i32,
    pub server_name: String,
    pub server_type: String,
    pub manifest_tag: Option<String>,
    pub manifest_digest: Option<String>,
    pub registry_image: Option<String>,
    pub local_image: String,
    pub status: String,
    pub last_built_at: DateTime<Utc>,
    pub source_repo: Option<String>,
    pub source_branch: Option<String>,
    pub source_revision: Option<String>,
    pub multi_arch: bool,
    pub credential_health_status: String,
    pub tier: String,
    pub health: ArtifactHealth,
    pub platforms: Vec<MarketplacePlatform>,
    pub vm_instances: Vec<MarketplaceVmInstance>,
    pub promotion: Option<MarketplacePromotion>,
    pub promotion_history: Vec<MarketplacePromotion>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMarketplaceStreamEvent {
    pub id: Uuid,
    pub provider_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_id: Option<Uuid>,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_ref: Option<String>,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

// key: marketplace-provider-submissions -> byok-gated-portal
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProviderMarketplaceSubmission {
    pub id: Uuid,
    pub provider_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_by: Option<i32>,
    pub tier: String,
    pub manifest_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    pub posture_state: Value,
    pub posture_vetoed: bool,
    pub posture_notes: Vec<String>,
    pub status: String,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProviderMarketplaceEvaluation {
    pub id: Uuid,
    pub submission_id: Uuid,
    pub evaluation_type: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluator_ref: Option<String>,
    pub result: Value,
    pub posture_state: Value,
    pub posture_vetoed: bool,
    pub posture_notes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProviderMarketplacePromotion {
    pub id: Uuid,
    pub evaluation_id: Uuid,
    pub gate: String,
    pub status: String,
    pub opened_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    pub notes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMarketplaceEvaluationSummary {
    pub evaluation: ProviderMarketplaceEvaluation,
    pub promotions: Vec<ProviderMarketplacePromotion>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMarketplaceSubmissionSummary {
    pub submission: ProviderMarketplaceSubmission,
    pub evaluations: Vec<ProviderMarketplaceEvaluationSummary>,
}

#[derive(Debug, Deserialize)]
struct ProviderMarketplaceSubmissionRequest {
    pub tier: String,
    pub manifest_uri: String,
    pub artifact_digest: Option<String>,
    pub release_notes: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ProviderMarketplaceEvaluationRequest {
    pub evaluation_type: String,
    pub status: Option<String>,
    pub evaluator_ref: Option<String>,
    pub result: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ProviderMarketplaceEvaluationTransition {
    pub status: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ProviderMarketplacePromotionRequest {
    pub gate: String,
    pub status: Option<String>,
    pub notes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ProviderMarketplacePromotionTransition {
    pub status: String,
    pub notes: Option<Vec<String>>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct ProviderPostureSnapshot {
    state: Value,
    vetoed: bool,
    notes: Vec<String>,
}

pub async fn list_marketplace(
    Extension(pool): Extension<PgPool>,
    Query(params): Query<MarketplaceQuery>,
) -> AppResult<Json<Vec<MarketplaceArtifact>>> {
    let limit = params.limit.unwrap_or(50).min(200) as i64;
    let search_pattern = params
        .q
        .as_ref()
        .filter(|term| !term.trim().is_empty())
        .map(|term| format!("%{}%", term.trim()));

    let rows = sqlx::query(
        r#"
        SELECT
            runs.id,
            runs.server_id,
            runs.source_repo,
            runs.source_branch,
            runs.source_revision,
            runs.registry,
            runs.local_image,
            runs.registry_image,
            runs.manifest_tag,
            runs.manifest_digest,
            runs.status,
            runs.multi_arch,
            runs.completed_at,
            runs.credential_health_status,
            runs.auth_refresh_attempted,
            runs.auth_refresh_succeeded,
            runs.auth_rotation_attempted,
            runs.auth_rotation_succeeded,
            servers.name AS server_name,
            servers.server_type,
            promotion_current.current_promotion,
            promotion_history.promotion_history,
            COALESCE(vm_instances.instances, '[]'::json) AS vm_instances,
            COALESCE(
                json_agg(
                    json_build_object(
                        'platform', platforms.platform,
                        'remote_image', platforms.remote_image,
                        'remote_tag', platforms.remote_tag,
                        'digest', platforms.digest,
                        'auth_refresh_attempted', platforms.auth_refresh_attempted,
                        'auth_refresh_succeeded', platforms.auth_refresh_succeeded,
                        'auth_rotation_attempted', platforms.auth_rotation_attempted,
                        'auth_rotation_succeeded', platforms.auth_rotation_succeeded,
                        'credential_health_status', platforms.credential_health_status
                    )
                    ORDER BY platforms.platform
                ) FILTER (WHERE platforms.id IS NOT NULL),
                '[]'::json
            ) AS platforms
        FROM build_artifact_runs runs
        JOIN mcp_servers servers ON servers.id = runs.server_id
        LEFT JOIN build_artifact_platforms platforms ON platforms.run_id = runs.id
        LEFT JOIN LATERAL (
            SELECT json_build_object(
                    'track_id', ap.promotion_track_id,
                    'track_name', t.name,
                    'stage', ap.stage,
                    'status', ap.status,
                    'updated_at', ap.updated_at,
                    'notes', ap.notes
                ) AS current_promotion
            FROM artifact_promotions ap
            JOIN promotion_tracks t ON t.id = ap.promotion_track_id
            WHERE runs.manifest_digest IS NOT NULL
              AND ap.manifest_digest = runs.manifest_digest
              AND t.tier = servers.server_type
            ORDER BY CASE ap.status
                        WHEN 'active' THEN 0
                        WHEN 'approved' THEN 1
                        WHEN 'in_progress' THEN 2
                        WHEN 'scheduled' THEN 3
                        ELSE 4
                     END,
                     ap.updated_at DESC
            LIMIT 1
        ) promotion_current ON TRUE
        LEFT JOIN LATERAL (
            SELECT COALESCE(
                    json_agg(
                        json_build_object(
                            'track_id', ap.promotion_track_id,
                            'track_name', t.name,
                            'stage', ap.stage,
                            'status', ap.status,
                            'updated_at', ap.updated_at,
                            'notes', ap.notes
                        )
                        ORDER BY ap.updated_at DESC
                    ),
                    '[]'::json
                ) AS promotion_history
            FROM artifact_promotions ap
            JOIN promotion_tracks t ON t.id = ap.promotion_track_id
            WHERE runs.manifest_digest IS NOT NULL
              AND ap.manifest_digest = runs.manifest_digest
              AND t.tier = servers.server_type
        ) promotion_history ON TRUE
        LEFT JOIN LATERAL (
            SELECT COALESCE(
                    json_agg(
                        json_build_object(
                            'instance_id', vmi.instance_id,
                            'isolation_tier', vmi.isolation_tier,
                            'attestation_status', vmi.attestation_status,
                            'policy_version', vmi.policy_version,
                            'created_at', vmi.created_at,
                            'updated_at', vmi.updated_at,
                            'terminated_at', vmi.terminated_at,
                            'last_error', vmi.last_error,
                            'capability_notes', vmi.capability_notes
                        )
                        ORDER BY vmi.created_at DESC
                    ),
                    '[]'::json
                ) AS instances
            FROM runtime_vm_instances vmi
            WHERE vmi.server_id = runs.server_id
        ) vm_instances ON TRUE
        WHERE ($2::text IS NULL OR servers.server_type = $2)
          AND ($3::text IS NULL OR runs.status = $3)
          AND (
                $4::text IS NULL
                OR servers.name ILIKE $4
                OR runs.manifest_tag ILIKE $4
                OR runs.manifest_digest ILIKE $4
                OR runs.registry_image ILIKE $4
                OR runs.local_image ILIKE $4
                OR runs.source_repo ILIKE $4
            )
        GROUP BY runs.id, servers.id
        ORDER BY runs.completed_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .bind(params.server_type.as_deref())
    .bind(params.status.as_deref())
    .bind(search_pattern.as_deref())
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    let mut artifacts = Vec::with_capacity(rows.len());

    for row in rows {
        let raw_platforms: serde_json::Value = row.get("platforms");
        let mut platforms: Vec<MarketplacePlatform> = serde_json::from_value(raw_platforms)
            .map_err(|error| {
                AppError::Message(format!("failed to deserialize platform slices: {error}"))
            })?;
        platforms.sort_by(|a, b| a.platform.cmp(&b.platform));

        let promotion_current: Option<serde_json::Value> = row.get("current_promotion");
        let promotion = match promotion_current {
            Some(value) if !value.is_null() => Some(
                serde_json::from_value::<MarketplacePromotion>(value).map_err(|error| {
                    AppError::Message(format!("failed to deserialize promotion snapshot: {error}"))
                })?,
            ),
            _ => None,
        };

        let history_value: serde_json::Value = row.get("promotion_history");
        let mut promotion_history: Vec<MarketplacePromotion> =
            serde_json::from_value(history_value).map_err(|error| {
                AppError::Message(format!("failed to deserialize promotion history: {error}"))
            })?;
        promotion_history.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        let vm_instances_value: serde_json::Value = row.get("vm_instances");
        let vm_instances: Vec<MarketplaceVmInstance> = serde_json::from_value(vm_instances_value)
            .map_err(|error| {
            AppError::Message(format!("failed to deserialize vm instances: {error}"))
        })?;

        let status: String = row.get("status");
        let credential_health_status: String = row.get("credential_health_status");
        let health = derive_health(&status, &credential_health_status, &platforms);

        let tier = classify_tier(row.get("server_type"), row.get("multi_arch"), &health);

        let artifact = MarketplaceArtifact {
            server_id: row.get("server_id"),
            server_name: row.get("server_name"),
            server_type: row.get("server_type"),
            manifest_tag: row.get("manifest_tag"),
            manifest_digest: row.get("manifest_digest"),
            registry_image: row.get("registry_image"),
            local_image: row.get("local_image"),
            status: status.clone(),
            last_built_at: row.get("completed_at"),
            source_repo: row.get("source_repo"),
            source_branch: row.get("source_branch"),
            source_revision: row.get("source_revision"),
            multi_arch: row.get("multi_arch"),
            credential_health_status,
            tier: tier.clone(),
            health,
            platforms,
            vm_instances,
            promotion,
            promotion_history,
        };

        let tier_filter_allows = params
            .tier
            .as_ref()
            .map(|expected| expected.eq_ignore_ascii_case(&tier))
            .unwrap_or(true);

        if tier_filter_allows {
            artifacts.push(artifact);
        }
    }

    Ok(Json(artifacts))
}

async fn list_provider_submissions(
    Extension(pool): Extension<PgPool>,
    Path(provider_id): Path<Uuid>,
    _user: AuthUser,
) -> AppResult<Json<Vec<ProviderMarketplaceSubmissionSummary>>> {
    let submissions = sqlx::query_as::<_, ProviderMarketplaceSubmission>(
        r#"
        SELECT id, provider_id, submitted_by, tier, manifest_uri, artifact_digest,
               release_notes, posture_state, posture_vetoed, posture_notes, status,
               metadata, created_at, updated_at
        FROM provider_marketplace_submissions
        WHERE provider_id = $1
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .bind(provider_id)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    if submissions.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let submission_ids: Vec<Uuid> = submissions.iter().map(|record| record.id).collect();
    let evaluation_rows = sqlx::query_as::<_, ProviderMarketplaceEvaluation>(
        r#"
        SELECT id, submission_id, evaluation_type, status, started_at, completed_at,
               evaluator_ref, result, posture_state, posture_vetoed, posture_notes,
               created_at, updated_at
        FROM provider_marketplace_evaluations
        WHERE submission_id = ANY($1)
        ORDER BY started_at DESC
        "#,
    )
    .bind(&submission_ids)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    let mut evaluation_map: HashMap<Uuid, Vec<ProviderMarketplaceEvaluation>> = HashMap::new();
    let mut evaluation_ids = Vec::new();
    for evaluation in evaluation_rows {
        evaluation_ids.push(evaluation.id);
        evaluation_map
            .entry(evaluation.submission_id)
            .or_default()
            .push(evaluation);
    }

    let mut promotion_map: HashMap<Uuid, Vec<ProviderMarketplacePromotion>> = HashMap::new();
    if !evaluation_ids.is_empty() {
        let promotions = sqlx::query_as::<_, ProviderMarketplacePromotion>(
            r#"
            SELECT id, evaluation_id, gate, status, opened_at, closed_at, notes,
                   created_at, updated_at
            FROM provider_marketplace_promotions
            WHERE evaluation_id = ANY($1)
            ORDER BY opened_at DESC
            "#,
        )
        .bind(&evaluation_ids)
        .fetch_all(&pool)
        .await
        .map_err(AppError::from)?;

        for promotion in promotions {
            promotion_map
                .entry(promotion.evaluation_id)
                .or_default()
                .push(promotion);
        }
    }

    let mut payload = Vec::with_capacity(submissions.len());
    for submission in submissions {
        let mut evaluation_summaries = Vec::new();
        if let Some(mut evaluations) = evaluation_map.remove(&submission.id) {
            evaluations.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            for evaluation in evaluations {
                let mut promotions = promotion_map.remove(&evaluation.id).unwrap_or_default();
                promotions.sort_by(|a, b| b.opened_at.cmp(&a.opened_at));
                evaluation_summaries.push(ProviderMarketplaceEvaluationSummary {
                    evaluation,
                    promotions,
                });
            }
        }
        payload.push(ProviderMarketplaceSubmissionSummary {
            submission,
            evaluations: evaluation_summaries,
        });
    }

    payload.sort_by(|a, b| b.submission.created_at.cmp(&a.submission.created_at));

    Ok(Json(payload))
}

async fn create_provider_submission(
    Extension(pool): Extension<PgPool>,
    Path(provider_id): Path<Uuid>,
    user: AuthUser,
    Json(request): Json<ProviderMarketplaceSubmissionRequest>,
) -> AppResult<Json<ProviderMarketplaceSubmission>> {
    let ProviderMarketplaceSubmissionRequest {
        tier,
        manifest_uri,
        artifact_digest,
        release_notes,
        metadata,
    } = request;

    let tier = tier.trim().to_string();
    let manifest_uri = manifest_uri.trim().to_string();
    if manifest_uri.is_empty() {
        return Err(AppError::BadRequest("manifest_uri required".into()));
    }

    let snapshot = ensure_provider_marketplace_eligible(&pool, provider_id, &tier).await?;
    if snapshot.vetoed {
        let reason = if snapshot.notes.is_empty() {
            "provider posture vetoed".to_string()
        } else {
            format!("provider posture vetoed: {}", snapshot.notes.join(","))
        };
        return Err(AppError::BadRequest(reason));
    }

    let ProviderPostureSnapshot {
        state,
        vetoed,
        notes,
    } = snapshot;
    let artifact_digest = artifact_digest.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let release_notes = release_notes.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let metadata = metadata.unwrap_or_else(|| json!({}));
    let submission = sqlx::query_as::<_, ProviderMarketplaceSubmission>(
        r#"
        INSERT INTO provider_marketplace_submissions (
            id, provider_id, submitted_by, tier, manifest_uri, artifact_digest,
            release_notes, posture_state, posture_vetoed, posture_notes, status,
            metadata
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'pending', $11)
        RETURNING id, provider_id, submitted_by, tier, manifest_uri, artifact_digest,
                  release_notes, posture_state, posture_vetoed, posture_notes, status,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(provider_id)
    .bind(Some(user.user_id))
    .bind(&tier)
    .bind(&manifest_uri)
    .bind(artifact_digest.as_deref())
    .bind(release_notes.as_deref())
    .bind(state)
    .bind(vetoed)
    .bind(notes.clone())
    .bind(metadata)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    record_marketplace_event(
        &pool,
        submission.provider_id,
        Some(submission.id),
        None,
        None,
        Some(format!("user:{}:{}", user.role, user.user_id)),
        "submission_created",
        json!({
            "tier": submission.tier,
            "manifest_uri": submission.manifest_uri,
            "status": submission.status,
        }),
    )
    .await?;

    Ok(Json(submission))
}

async fn create_evaluation_run(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, submission_id)): Path<(Uuid, Uuid)>,
    user: AuthUser,
    Json(request): Json<ProviderMarketplaceEvaluationRequest>,
) -> AppResult<Json<ProviderMarketplaceEvaluation>> {
    let submission = fetch_submission(&pool, provider_id, submission_id).await?;
    let snapshot =
        ensure_provider_marketplace_eligible(&pool, provider_id, &submission.tier).await?;
    if snapshot.vetoed {
        let reason = if snapshot.notes.is_empty() {
            "provider posture vetoed".to_string()
        } else {
            format!("provider posture vetoed: {}", snapshot.notes.join(","))
        };
        return Err(AppError::BadRequest(reason));
    }

    let ProviderPostureSnapshot {
        state,
        vetoed,
        notes,
    } = snapshot;
    let ProviderMarketplaceEvaluationRequest {
        evaluation_type,
        status,
        evaluator_ref,
        result,
    } = request;

    let status = status
        .unwrap_or_else(|| "running".to_string())
        .trim()
        .to_ascii_lowercase();
    if status.is_empty() {
        return Err(AppError::BadRequest("status must be non-empty".into()));
    }

    let result_payload = result.unwrap_or_else(|| json!({}));
    let evaluator_ref = evaluator_ref.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let evaluation = sqlx::query_as::<_, ProviderMarketplaceEvaluation>(
        r#"
        INSERT INTO provider_marketplace_evaluations (
            id, submission_id, evaluation_type, status, evaluator_ref, result,
            posture_state, posture_vetoed, posture_notes
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id, submission_id, evaluation_type, status, started_at, completed_at,
                  evaluator_ref, result, posture_state, posture_vetoed, posture_notes,
                  created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(submission.id)
    .bind(evaluation_type.trim())
    .bind(status)
    .bind(evaluator_ref.as_deref())
    .bind(result_payload)
    .bind(state)
    .bind(vetoed)
    .bind(notes.clone())
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    record_marketplace_event(
        &pool,
        submission.provider_id,
        Some(submission.id),
        Some(evaluation.id),
        None,
        Some(format!("user:{}:{}", user.role, user.user_id)),
        "evaluation_started",
        json!({
            "evaluation_type": evaluation.evaluation_type,
            "status": evaluation.status,
        }),
    )
    .await?;

    Ok(Json(evaluation))
}

async fn transition_evaluation_run(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, evaluation_id)): Path<(Uuid, Uuid)>,
    user: AuthUser,
    Json(request): Json<ProviderMarketplaceEvaluationTransition>,
) -> AppResult<Json<ProviderMarketplaceEvaluation>> {
    let (mut evaluation, submission) = fetch_evaluation(&pool, provider_id, evaluation_id).await?;
    let ProviderMarketplaceEvaluationTransition {
        status,
        completed_at,
        result,
    } = request;

    let status = status.trim();
    if status.is_empty() {
        return Err(AppError::BadRequest("status must be non-empty".into()));
    }

    let result_payload = result.unwrap_or_else(|| evaluation.result.clone());

    let updated = sqlx::query_as::<_, ProviderMarketplaceEvaluation>(
        r#"
        UPDATE provider_marketplace_evaluations
        SET status = $1,
            completed_at = COALESCE($2, completed_at),
            result = $3,
            updated_at = NOW()
        WHERE id = $4
        RETURNING id, submission_id, evaluation_type, status, started_at, completed_at,
                  evaluator_ref, result, posture_state, posture_vetoed, posture_notes,
                  created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(completed_at)
    .bind(result_payload.clone())
    .bind(evaluation.id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    evaluation = updated.clone();

    record_marketplace_event(
        &pool,
        submission.provider_id,
        Some(submission.id),
        Some(evaluation.id),
        None,
        Some(format!("user:{}:{}", user.role, user.user_id)),
        "evaluation_transitioned",
        json!({
            "status": evaluation.status,
            "completed_at": evaluation.completed_at,
        }),
    )
    .await?;

    Ok(Json(updated))
}

async fn create_promotion_gate(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, evaluation_id)): Path<(Uuid, Uuid)>,
    user: AuthUser,
    Json(request): Json<ProviderMarketplacePromotionRequest>,
) -> AppResult<Json<ProviderMarketplacePromotion>> {
    let (evaluation, submission) = fetch_evaluation(&pool, provider_id, evaluation_id).await?;
    let ProviderMarketplacePromotionRequest {
        gate,
        status,
        notes,
    } = request;

    let status = status
        .unwrap_or_else(|| "pending".to_string())
        .trim()
        .to_string();
    if status.is_empty() {
        return Err(AppError::BadRequest("status must be non-empty".into()));
    }

    let gate = gate.trim();
    if gate.is_empty() {
        return Err(AppError::BadRequest("gate must be non-empty".into()));
    }

    let notes = notes.unwrap_or_default();
    let promotion = sqlx::query_as::<_, ProviderMarketplacePromotion>(
        r#"
        INSERT INTO provider_marketplace_promotions (
            id, evaluation_id, gate, status, notes
        )
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, evaluation_id, gate, status, opened_at, closed_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(evaluation.id)
    .bind(gate)
    .bind(status)
    .bind(notes.clone())
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    record_marketplace_event(
        &pool,
        submission.provider_id,
        Some(submission.id),
        Some(evaluation.id),
        Some(promotion.id),
        Some(format!("user:{}:{}", user.role, user.user_id)),
        "promotion_created",
        json!({
            "gate": promotion.gate,
            "status": promotion.status,
        }),
    )
    .await?;

    Ok(Json(promotion))
}

async fn transition_promotion_gate(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, promotion_id)): Path<(Uuid, Uuid)>,
    user: AuthUser,
    Json(request): Json<ProviderMarketplacePromotionTransition>,
) -> AppResult<Json<ProviderMarketplacePromotion>> {
    let (mut promotion, evaluation, submission) =
        fetch_promotion(&pool, provider_id, promotion_id).await?;

    let ProviderMarketplacePromotionTransition {
        status,
        notes,
        closed_at,
    } = request;

    let status = status.trim();
    if status.is_empty() {
        return Err(AppError::BadRequest("status must be non-empty".into()));
    }

    let notes = notes.unwrap_or_else(|| promotion.notes.clone());
    let updated = sqlx::query_as::<_, ProviderMarketplacePromotion>(
        r#"
        UPDATE provider_marketplace_promotions
        SET status = $1,
            notes = $2,
            closed_at = COALESCE($3, closed_at),
            updated_at = NOW()
        WHERE id = $4
        RETURNING id, evaluation_id, gate, status, opened_at, closed_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(notes.clone())
    .bind(closed_at)
    .bind(promotion.id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    promotion = updated.clone();

    record_marketplace_event(
        &pool,
        submission.provider_id,
        Some(submission.id),
        Some(evaluation.id),
        Some(promotion.id),
        Some(format!("user:{}:{}", user.role, user.user_id)),
        "promotion_transitioned",
        json!({
            "status": promotion.status,
            "closed_at": promotion.closed_at,
        }),
    )
    .await?;

    Ok(Json(updated))
}

async fn fetch_submission(
    pool: &PgPool,
    provider_id: Uuid,
    submission_id: Uuid,
) -> AppResult<ProviderMarketplaceSubmission> {
    let record = sqlx::query_as::<_, ProviderMarketplaceSubmission>(
        r#"
        SELECT id, provider_id, submitted_by, tier, manifest_uri, artifact_digest,
               release_notes, posture_state, posture_vetoed, posture_notes, status,
               metadata, created_at, updated_at
        FROM provider_marketplace_submissions
        WHERE id = $1 AND provider_id = $2
        "#,
    )
    .bind(submission_id)
    .bind(provider_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;

    record.ok_or(AppError::NotFound)
}

async fn fetch_evaluation(
    pool: &PgPool,
    provider_id: Uuid,
    evaluation_id: Uuid,
) -> AppResult<(ProviderMarketplaceEvaluation, ProviderMarketplaceSubmission)> {
    let evaluation = sqlx::query_as::<_, ProviderMarketplaceEvaluation>(
        r#"
        SELECT id, submission_id, evaluation_type, status, started_at, completed_at,
               evaluator_ref, result, posture_state, posture_vetoed, posture_notes,
               created_at, updated_at
        FROM provider_marketplace_evaluations
        WHERE id = $1
        "#,
    )
    .bind(evaluation_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;

    let Some(evaluation) = evaluation else {
        return Err(AppError::NotFound);
    };

    let submission = fetch_submission(pool, provider_id, evaluation.submission_id).await?;

    Ok((evaluation, submission))
}

async fn fetch_promotion(
    pool: &PgPool,
    provider_id: Uuid,
    promotion_id: Uuid,
) -> AppResult<(
    ProviderMarketplacePromotion,
    ProviderMarketplaceEvaluation,
    ProviderMarketplaceSubmission,
)> {
    let promotion = sqlx::query_as::<_, ProviderMarketplacePromotion>(
        r#"
        SELECT id, evaluation_id, gate, status, opened_at, closed_at, notes,
               created_at, updated_at
        FROM provider_marketplace_promotions
        WHERE id = $1
        "#,
    )
    .bind(promotion_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;

    let Some(promotion) = promotion else {
        return Err(AppError::NotFound);
    };

    let (evaluation, submission) =
        fetch_evaluation(pool, provider_id, promotion.evaluation_id).await?;

    Ok((promotion, evaluation, submission))
}

async fn record_marketplace_event(
    pool: &PgPool,
    provider_id: Uuid,
    submission_id: Option<Uuid>,
    evaluation_id: Option<Uuid>,
    promotion_id: Option<Uuid>,
    actor_ref: Option<String>,
    event_type: &str,
    payload: Value,
) -> AppResult<()> {
    let event_id = Uuid::new_v4();
    let occurred_at: DateTime<Utc> = sqlx::query_scalar(
        r#"
        INSERT INTO provider_marketplace_events (
            id, submission_id, evaluation_id, promotion_id, actor_ref, event_type, payload
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING occurred_at
        "#,
    )
    .bind(event_id)
    .bind(submission_id)
    .bind(evaluation_id)
    .bind(promotion_id)
    .bind(actor_ref.clone())
    .bind(event_type)
    .bind(payload.clone())
    .fetch_one(pool)
    .await
    .map_err(AppError::from)?;

    publish_marketplace_event(ProviderMarketplaceStreamEvent {
        id: event_id,
        provider_id,
        submission_id,
        evaluation_id,
        promotion_id,
        event_type: event_type.to_string(),
        actor_ref,
        payload,
        occurred_at,
    });

    Ok(())
}

fn ensure_posture_summary_state(
    provider_id: Uuid,
    tier: &str,
    summary: &ProviderKeyPolicySummary,
    requirement: Option<&ProviderTierRequirement>,
) -> Value {
    let requirement_payload = requirement.map(|requirement| {
        json!({
            "tier": requirement.tier,
            "provider_id": requirement.provider_id,
            "byok_required": requirement.byok_required,
        })
    });

    let record_payload = summary.record.as_ref().map(|record| {
        json!({
            "provider_key_id": record.id,
            "state": record.state,
            "rotation_due_at": record.rotation_due_at,
            "attestation_verified_at": record.attestation_verified_at,
            "attestation_signature_registered": record.attestation_signature_registered,
            "attestation_digest_present": record.attestation_digest.is_some(),
        })
    });

    json!({
        "provider_id": provider_id,
        "tier": tier,
        "requirement": requirement_payload,
        "notes": summary.notes,
        "vetoed": summary.vetoed,
        "record": record_payload,
    })
}

async fn ensure_provider_marketplace_eligible(
    pool: &PgPool,
    provider_id: Uuid,
    tier: &str,
) -> AppResult<ProviderPostureSnapshot> {
    let service = ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());
    let summary = service.summarize_for_policy(provider_id).await?;
    let requirement = service.tier_requirement(tier).await?;

    if let Some(ref requirement) = requirement {
        if requirement.provider_id != provider_id {
            return Err(AppError::Forbidden);
        }
    }

    if summary.vetoed {
        let reason = if summary.notes.is_empty() {
            "provider posture vetoed".to_string()
        } else {
            format!("provider posture vetoed: {}", summary.notes.join(","))
        };
        return Err(AppError::BadRequest(reason));
    }

    if let Some(ref requirement) = requirement {
        if requirement.byok_required && summary.record.is_none() {
            return Err(AppError::BadRequest(
                "provider must maintain an active BYOK key".into(),
            ));
        }
    }

    let state = ensure_posture_summary_state(provider_id, tier, &summary, requirement.as_ref());

    Ok(ProviderPostureSnapshot {
        state,
        vetoed: summary.vetoed,
        notes: summary.notes.clone(),
    })
}

pub(crate) fn derive_health(
    status: &str,
    run_health: &str,
    platforms: &[MarketplacePlatform],
) -> ArtifactHealth {
    let mut issues = Vec::new();
    if !matches_success(status) {
        issues.push(format!("build_status:{status}"));
    }
    if !matches_healthy(run_health) {
        issues.push(format!("credential:{run_health}"));
    }
    for platform in platforms {
        if !matches_healthy(&platform.credential_health_status) {
            issues.push(format!(
                "platform:{}:{}",
                platform.platform, platform.credential_health_status
            ));
        }
    }

    let overall = if issues.is_empty() {
        "healthy"
    } else if issues
        .iter()
        .any(|issue| issue.starts_with("build_status:"))
    {
        "blocked"
    } else {
        "degraded"
    };

    ArtifactHealth {
        overall: overall.into(),
        issues,
    }
}

pub(crate) fn classify_tier(
    server_type: String,
    multi_arch: bool,
    health: &ArtifactHealth,
) -> String {
    let normalized_health = health.overall.as_str();
    if normalized_health == "healthy" && multi_arch {
        format!("gold:{}", server_type)
    } else if normalized_health == "healthy" {
        format!("silver:{}", server_type)
    } else if multi_arch {
        format!("watchlist-multi:{}", server_type)
    } else {
        format!("watchlist:{}", server_type)
    }
}

fn matches_success(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "succeeded" | "success" | "completed"
    )
}

fn matches_healthy(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "healthy" | "ok" | "success" | "succeeded" | "passing"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::RegisterProviderKey;
    use anyhow::Result;
    use axum::{extract::Path, Extension, Json};
    use base64::{engine::general_purpose, Engine as _};
    use chrono::Utc;
    use sqlx::postgres::PgPoolOptions;
    use tokio::time::{timeout, Duration};
    use uuid::Uuid;

    #[test]
    fn derives_health_for_successful_run() {
        let platforms = vec![MarketplacePlatform {
            platform: "linux/amd64".into(),
            remote_image: "example".into(),
            remote_tag: "latest".into(),
            digest: None,
            auth_refresh_attempted: false,
            auth_refresh_succeeded: true,
            auth_rotation_attempted: false,
            auth_rotation_succeeded: true,
            credential_health_status: "healthy".into(),
        }];

        let health = derive_health("succeeded", "healthy", &platforms);
        assert_eq!(health.overall, "healthy");
        assert!(health.issues.is_empty());
    }

    #[test]
    fn derives_health_with_platform_issue() {
        let platforms = vec![MarketplacePlatform {
            platform: "linux/arm64".into(),
            remote_image: "example".into(),
            remote_tag: "latest".into(),
            digest: None,
            auth_refresh_attempted: true,
            auth_refresh_succeeded: false,
            auth_rotation_attempted: false,
            auth_rotation_succeeded: true,
            credential_health_status: "error".into(),
        }];

        let health = derive_health("succeeded", "healthy", &platforms);
        assert_eq!(health.overall, "degraded");
        assert_eq!(health.issues.len(), 1);
    }

    #[test]
    fn classify_tier_promotes_multi_arch() {
        let health = ArtifactHealth {
            overall: "healthy".into(),
            issues: Vec::new(),
        };
        let tier = classify_tier("Router".into(), true, &health);
        assert!(tier.starts_with("gold:"));
    }

    #[tokio::test]
    async fn submission_flow_persists_events() -> Result<()> {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("skipping submission_flow_persists_events: DATABASE_URL not configured");
                return Ok(());
            }
        };

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
        sqlx::migrate!("../backend/migrations").run(&pool).await?;

        sqlx::query(
            "TRUNCATE provider_marketplace_events, provider_marketplace_promotions, provider_marketplace_evaluations, provider_marketplace_submissions RESTART IDENTITY CASCADE",
        )
        .execute(&pool)
        .await?;

        let tier = "gold-inference";
        sqlx::query("DELETE FROM provider_tiers WHERE tier = $1")
            .bind(tier)
            .execute(&pool)
            .await?;

        let provider_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO provider_tiers (tier, provider_id, byok_required) VALUES ($1,$2,TRUE)
             ON CONFLICT (tier) DO UPDATE SET provider_id = EXCLUDED.provider_id,
             byok_required = EXCLUDED.byok_required, updated_at = NOW()",
        )
        .bind(tier)
        .bind(provider_id)
        .execute(&pool)
        .await?;

        let service = ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());
        let attestation = general_purpose::STANDARD.encode(b"marketplace-posture");
        service
            .register_key(
                provider_id,
                RegisterProviderKey {
                    alias: Some("primary".to_string()),
                    attestation_digest: Some(attestation.clone()),
                    attestation_signature: Some(attestation),
                    rotation_due_at: None,
                },
            )
            .await?;

        let mut event_rx = super::subscribe_marketplace_events();

        let submission = create_provider_submission(
            Extension(pool.clone()),
            Path(provider_id),
            AuthUser {
                user_id: 41,
                role: "operator".into(),
            },
            Json(ProviderMarketplaceSubmissionRequest {
                tier: tier.to_string(),
                manifest_uri: "oci://registry/image:stable".into(),
                artifact_digest: Some("sha256:abc123".into()),
                release_notes: Some("initial submission".into()),
                metadata: None,
            }),
        )
        .await?
        .0;

        let first_event = timeout(Duration::from_secs(2), event_rx.recv()).await??;
        assert_eq!(first_event.event_type, "submission_created");
        assert_eq!(first_event.provider_id, provider_id);
        assert_eq!(first_event.submission_id, Some(submission.id));

        let evaluation = create_evaluation_run(
            Extension(pool.clone()),
            Path((provider_id, submission.id)),
            AuthUser {
                user_id: 41,
                role: "operator".into(),
            },
            Json(ProviderMarketplaceEvaluationRequest {
                evaluation_type: "compliance".into(),
                status: Some("running".into()),
                evaluator_ref: Some("automation".into()),
                result: Some(json!({"started": true})),
            }),
        )
        .await?
        .0;

        let evaluation = transition_evaluation_run(
            Extension(pool.clone()),
            Path((provider_id, evaluation.id)),
            AuthUser {
                user_id: 41,
                role: "operator".into(),
            },
            Json(ProviderMarketplaceEvaluationTransition {
                status: "succeeded".into(),
                completed_at: Some(Utc::now()),
                result: Some(json!({"score": "pass"})),
            }),
        )
        .await?
        .0;

        let promotion = create_promotion_gate(
            Extension(pool.clone()),
            Path((provider_id, evaluation.id)),
            AuthUser {
                user_id: 41,
                role: "operator".into(),
            },
            Json(ProviderMarketplacePromotionRequest {
                gate: "sandbox".into(),
                status: Some("pending".into()),
                notes: Some(vec!["initial-review".into()]),
            }),
        )
        .await?
        .0;

        let promotion = transition_promotion_gate(
            Extension(pool.clone()),
            Path((provider_id, promotion.id)),
            AuthUser {
                user_id: 41,
                role: "operator".into(),
            },
            Json(ProviderMarketplacePromotionTransition {
                status: "approved".into(),
                notes: Some(vec!["passed".into()]),
                closed_at: Some(Utc::now()),
            }),
        )
        .await?
        .0;

        assert_eq!(promotion.status, "approved");

        let listings = list_provider_submissions(
            Extension(pool.clone()),
            Path(provider_id),
            AuthUser {
                user_id: 51,
                role: "operator".into(),
            },
        )
        .await?
        .0;

        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].evaluations.len(), 1);
        assert_eq!(listings[0].evaluations[0].promotions.len(), 1);

        let event_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_marketplace_events WHERE submission_id = $1",
        )
        .bind(submission.id)
        .fetch_one(&pool)
        .await?;

        assert!(event_count >= 5);

        Ok(())
    }

    #[tokio::test]
    async fn submission_rejected_without_active_key() -> Result<()> {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping submission_rejected_without_active_key: DATABASE_URL not configured"
                );
                return Ok(());
            }
        };

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
        sqlx::migrate!("../backend/migrations").run(&pool).await?;

        sqlx::query(
            "TRUNCATE provider_marketplace_events, provider_marketplace_promotions, provider_marketplace_evaluations, provider_marketplace_submissions RESTART IDENTITY CASCADE",
        )
        .execute(&pool)
        .await?;

        let tier = "gold-inference";
        sqlx::query("DELETE FROM provider_tiers WHERE tier = $1")
            .bind(tier)
            .execute(&pool)
            .await?;

        let provider_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO provider_tiers (tier, provider_id, byok_required) VALUES ($1,$2,TRUE)
             ON CONFLICT (tier) DO UPDATE SET provider_id = EXCLUDED.provider_id,
             byok_required = EXCLUDED.byok_required, updated_at = NOW()",
        )
        .bind(tier)
        .bind(provider_id)
        .execute(&pool)
        .await?;

        let result = create_provider_submission(
            Extension(pool.clone()),
            Path(provider_id),
            AuthUser {
                user_id: 11,
                role: "operator".into(),
            },
            Json(ProviderMarketplaceSubmissionRequest {
                tier: tier.to_string(),
                manifest_uri: "oci://registry/image:stable".into(),
                artifact_digest: None,
                release_notes: None,
                metadata: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));

        Ok(())
    }
}
