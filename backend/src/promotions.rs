use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{query_as, FromRow, PgPool, Postgres, QueryBuilder, Transaction};
use tracing::error;

use crate::error::{AppError, AppResult};
use crate::extractor::AuthUser;
use crate::governance::{GovernanceEngine, StartWorkflowRunRequest};

// key: release-train -> promotion-tracks,governance-binding

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PromotionTrack {
    pub id: i32,
    pub owner_id: i32,
    pub name: String,
    pub tier: String,
    pub stages: Vec<String>,
    pub description: Option<String>,
    pub workflow_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PromotionRecord {
    pub id: i64,
    pub promotion_track_id: i32,
    pub manifest_digest: String,
    pub artifact_run_id: Option<i32>,
    pub stage: String,
    pub status: String,
    pub workflow_run_id: Option<i64>,
    pub scheduled_by: Option<i32>,
    pub approved_by: Option<i32>,
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture_verdict: Option<Value>,
    pub scheduled_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub track_name: String,
    pub tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulePromotionRequest {
    pub track_id: i32,
    pub manifest_digest: String,
    pub artifact_run_id: Option<i32>,
    pub stage: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovePromotionRequest {
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionHistoryQuery {
    pub manifest_digest: Option<String>,
    pub track_id: Option<i32>,
}

#[derive(Debug, Clone)]
struct ReleaseTrain {
    stages: Vec<String>,
}

// key: promotion-gate -> trust-intel-fusion
#[derive(Debug, Clone)]
struct PromotionPostureSignals {
    artifact_status: Option<String>,
    credential_health_status: Option<String>,
    trust_lifecycle_state: Option<String>,
    trust_attestation_status: Option<String>,
    trust_remediation_state: Option<String>,
    trust_remediation_attempts: Option<i32>,
    remediation_status: Option<String>,
    remediation_failure_reason: Option<String>,
    intelligence: Vec<IntelligenceSignal>,
}

#[derive(Debug, Clone)]
struct IntelligenceSignal {
    capability: String,
    status: String,
    score: f32,
    confidence: f32,
}

#[derive(Debug, Clone)]
struct PromotionVerdict {
    allowed: bool,
    veto_reasons: Vec<String>,
    metadata: Value,
    posture_notes: Vec<String>,
}

impl ReleaseTrain {
    fn new(mut stages: Vec<String>) -> Self {
        if stages.is_empty() {
            stages = vec![
                "candidate".to_string(),
                "staging".to_string(),
                "production".to_string(),
            ];
        }
        stages
            .iter_mut()
            .for_each(|stage| *stage = stage.to_lowercase());
        Self { stages }
    }

    fn contains(&self, stage: &str) -> bool {
        let stage = stage.to_lowercase();
        self.stages.iter().any(|item| item == &stage)
    }

    fn previous_stage(&self, stage: &str) -> Option<String> {
        let stage = stage.to_lowercase();
        self.stages
            .iter()
            .position(|candidate| candidate == &stage)
            .and_then(|idx| idx.checked_sub(1))
            .and_then(|prev| self.stages.get(prev).cloned())
    }
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/promotions/tracks", get(list_tracks))
        .route("/api/promotions/schedule", post(schedule_promotion))
        .route("/api/promotions/:id/approve", post(approve_promotion))
        .route("/api/promotions/history", get(history))
}

async fn list_tracks(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> AppResult<Json<Vec<PromotionTrack>>> {
    let tracks = sqlx::query_as::<_, PromotionTrack>(
        r#"
        SELECT id, owner_id, name, tier, stages, description, workflow_id, created_at, updated_at
        FROM promotion_tracks
        WHERE owner_id = $1
        ORDER BY name
        "#,
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;

    Ok(Json(tracks))
}

async fn schedule_promotion(
    Extension(pool): Extension<PgPool>,
    Extension(engine): Extension<Arc<GovernanceEngine>>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<SchedulePromotionRequest>,
) -> AppResult<Json<PromotionRecord>> {
    let mut tx = pool.begin().await?;

    let track = sqlx::query_as::<_, PromotionTrack>(
        r#"
        SELECT id, owner_id, name, tier, stages, description, workflow_id, created_at, updated_at
        FROM promotion_tracks
        WHERE id = $1 AND owner_id = $2
        "#,
    )
    .bind(payload.track_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(track) = track else {
        return Err(AppError::NotFound);
    };

    let SchedulePromotionRequest {
        track_id: _,
        manifest_digest,
        artifact_run_id,
        stage: stage_input,
        mut notes,
    } = payload;

    let train = ReleaseTrain::new(track.stages.clone());
    let stage = stage_input.to_lowercase();
    if !train.contains(&stage) {
        return Err(AppError::BadRequest(format!(
            "stage `{stage}` is not part of track `{}`",
            track.name
        )));
    }

    if let Some(previous) = train.previous_stage(&stage) {
        let previous_active = sqlx::query_scalar::<_, Option<i64>>(
            r#"
            SELECT ap.id
            FROM artifact_promotions ap
            WHERE ap.promotion_track_id = $1
              AND ap.stage = $2
              AND ap.manifest_digest = $3
              AND ap.status = 'active'
            LIMIT 1
            "#,
        )
        .bind(track.id)
        .bind(&previous)
        .bind(&manifest_digest)
        .fetch_optional(&mut *tx)
        .await?;

        if previous_active.is_none() {
            return Err(AppError::BadRequest(format!(
                "previous stage `{previous}` must be active before promoting to `{stage}`"
            )));
        }
    }

    let existing = sqlx::query_scalar::<_, Option<i64>>(
        r#"
        SELECT ap.id
        FROM artifact_promotions ap
        WHERE ap.promotion_track_id = $1
          AND ap.stage = $2
          AND ap.manifest_digest = $3
        LIMIT 1
        "#,
    )
    .bind(track.id)
    .bind(&stage)
    .bind(&manifest_digest)
    .fetch_optional(&mut *tx)
    .await?;

    if existing.is_some() {
        return Err(AppError::BadRequest(format!(
            "promotion already exists for stage `{stage}` and digest `{}`",
            manifest_digest
        )));
    }

    let signals = collect_promotion_signals(&mut tx, artifact_run_id, &manifest_digest).await?;
    let verdict = evaluate_promotion_posture(&track, &signals);
    let verdict_payload = build_verdict_payload(&track, &stage, &verdict);

    if !verdict.allowed {
        let mut payload = verdict_payload.clone();
        if let Some(object) = payload.as_object_mut() {
            object.insert("error".to_string(), json!("promotion_veto"));
        }
        return Err(AppError::JsonBadRequest(payload));
    }

    notes.extend(verdict.posture_notes);

    let record_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO artifact_promotions (
            promotion_track_id,
            manifest_digest,
            artifact_run_id,
            stage,
            status,
            scheduled_by,
            notes,
            posture_verdict
        ) VALUES ($1, $2, $3, $4, 'scheduled', $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(track.id)
    .bind(&manifest_digest)
    .bind(artifact_run_id)
    .bind(&stage)
    .bind(user_id)
    .bind(&notes)
    .bind(&verdict_payload)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let mut record = load_promotion(&pool, record_id).await?;

    if let Some(workflow_id) = track.workflow_id {
        let mut workflow_notes = notes.clone();
        workflow_notes.push(format!("promotion:scheduled:{}:{}", track.name, stage));
        let workflow_request = StartWorkflowRunRequest {
            target_manifest_digest: Some(manifest_digest.clone()),
            target_artifact_run_id: artifact_run_id,
            notes: Some(workflow_notes),
            promotion_track_id: Some(track.id),
            promotion_stage: Some(stage.clone()),
        };

        match engine
            .start_workflow_run(&pool, workflow_id, user_id, workflow_request)
            .await
        {
            Ok(run) => {
                sqlx::query(
                    r#"
                    UPDATE artifact_promotions
                    SET workflow_run_id = $1,
                        status = 'in_progress',
                        updated_at = NOW(),
                        notes = array_append(notes, $2)
                    WHERE id = $3
                    "#,
                )
                .bind(run.id)
                .bind(format!("governance:run-started:{}", run.id))
                .bind(record.id)
                .execute(&pool)
                .await?;
                record = load_promotion(&pool, record.id).await?;
            }
            Err(err) => {
                error!(?err, "failed to start governance workflow for promotion");
                sqlx::query(
                    r#"
                    UPDATE artifact_promotions
                    SET notes = array_append(notes, $1),
                        updated_at = NOW()
                    WHERE id = $2
                    "#,
                )
                .bind(format!("governance:error:{}", err))
                .bind(record.id)
                .execute(&pool)
                .await?;
                return Err(AppError::Message(
                    "failed to start governance workflow".into(),
                ));
            }
        }
    }

    Ok(Json(record))
}

async fn approve_promotion(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<ApprovePromotionRequest>,
) -> AppResult<Json<PromotionRecord>> {
    let mut note = format!("promotion:approved:user:{user_id}");
    if let Some(extra) = payload.note.as_deref() {
        note.push(':');
        note.push_str(extra);
    }

    let rows = sqlx::query(
        r#"
        UPDATE artifact_promotions
        SET status = 'approved',
            approved_by = $1,
            approved_at = NOW(),
            updated_at = NOW(),
            notes = array_append(notes, $2)
        WHERE id = $3
        "#,
    )
    .bind(user_id)
    .bind(note)
    .bind(id)
    .execute(&pool)
    .await?;

    if rows.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    let record = load_promotion(&pool, id).await?;
    Ok(Json(record))
}

async fn history(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Query(params): Query<PromotionHistoryQuery>,
) -> AppResult<Json<Vec<PromotionRecord>>> {
    let mut builder = QueryBuilder::new(
        "SELECT ap.id, ap.promotion_track_id, ap.manifest_digest, ap.artifact_run_id, ap.stage, ap.status, \
         ap.workflow_run_id, ap.scheduled_by, ap.approved_by, ap.notes, ap.posture_verdict, ap.scheduled_at, ap.approved_at, \
         ap.activated_at, ap.updated_at, ap.created_at, t.name as track_name, t.tier \
         FROM artifact_promotions ap \
         JOIN promotion_tracks t ON t.id = ap.promotion_track_id \
         WHERE t.owner_id = "
    );
    builder.push_bind(user_id);

    if let Some(track_id) = params.track_id {
        builder.push(" AND ap.promotion_track_id = ");
        builder.push_bind(track_id);
    }

    if let Some(manifest_digest) = params.manifest_digest.as_ref() {
        builder.push(" AND ap.manifest_digest = ");
        builder.push_bind(manifest_digest);
    }

    builder.push(" ORDER BY ap.updated_at DESC, ap.id DESC");

    let query = builder.build_query_as::<PromotionRecord>();
    let records = query.fetch_all(&pool).await?;
    Ok(Json(records))
}

async fn load_promotion(pool: &PgPool, id: i64) -> AppResult<PromotionRecord> {
    let record = sqlx::query_as::<_, PromotionRecord>(
        r#"
        SELECT ap.id, ap.promotion_track_id, ap.manifest_digest, ap.artifact_run_id, ap.stage, ap.status,
               ap.workflow_run_id, ap.scheduled_by, ap.approved_by, ap.notes, ap.posture_verdict, ap.scheduled_at, ap.approved_at,
               ap.activated_at, ap.updated_at, ap.created_at, t.name as track_name, t.tier
        FROM artifact_promotions ap
        JOIN promotion_tracks t ON t.id = ap.promotion_track_id
        WHERE ap.id = $1
        "#,
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(record)
}

fn build_verdict_payload(track: &PromotionTrack, stage: &str, verdict: &PromotionVerdict) -> Value {
    let mut root = Map::new();
    root.insert("allowed".to_string(), json!(verdict.allowed));
    root.insert(
        "track".to_string(),
        json!({
            "id": track.id,
            "name": track.name,
            "tier": track.tier,
        }),
    );
    root.insert("stage".to_string(), json!(stage));
    root.insert("reasons".to_string(), json!(verdict.veto_reasons));
    if !verdict.posture_notes.is_empty() {
        root.insert("notes".to_string(), json!(verdict.posture_notes));
    }
    root.insert("metadata".to_string(), verdict.metadata.clone());
    Value::Object(root)
}

#[derive(Debug, FromRow)]
struct ArtifactRunRow {
    pub id: i32,
    pub server_id: i32,
    pub status: String,
    pub credential_health_status: String,
}

#[derive(Debug, FromRow)]
struct TrustSignalRow {
    pub lifecycle_state: String,
    pub attestation_status: String,
    pub remediation_state: Option<String>,
    pub remediation_attempts: i32,
}

#[derive(Debug, FromRow)]
struct RemediationSignalRow {
    pub status: String,
    pub failure_reason: Option<String>,
}

#[derive(Debug, FromRow)]
struct IntelligenceRow {
    pub capability: String,
    pub status: String,
    pub score: f32,
    pub confidence: f32,
}

async fn collect_promotion_signals(
    tx: &mut Transaction<'_, Postgres>,
    artifact_run_id: Option<i32>,
    manifest_digest: &str,
) -> AppResult<PromotionPostureSignals> {
    let mut signals = PromotionPostureSignals {
        artifact_status: None,
        credential_health_status: None,
        trust_lifecycle_state: None,
        trust_attestation_status: None,
        trust_remediation_state: None,
        trust_remediation_attempts: None,
        remediation_status: None,
        remediation_failure_reason: None,
        intelligence: Vec::new(),
    };

    let artifact_row = if let Some(id) = artifact_run_id {
        query_as::<_, ArtifactRunRow>(
            r#"
            SELECT id, server_id, status, credential_health_status
            FROM build_artifact_runs
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        query_as::<_, ArtifactRunRow>(
            r#"
            SELECT id, server_id, status, credential_health_status
            FROM build_artifact_runs
            WHERE manifest_digest = $1
            ORDER BY completed_at DESC NULLS LAST
            LIMIT 1
            "#,
        )
        .bind(manifest_digest)
        .fetch_optional(&mut *tx)
        .await?
    };

    if let Some(row) = artifact_row {
        signals.artifact_status = Some(row.status.clone());
        signals.credential_health_status = Some(row.credential_health_status.clone());

        let trust_row = query_as::<_, TrustSignalRow>(
            r#"
            SELECT
                registry.lifecycle_state,
                registry.attestation_status,
                registry.remediation_state,
                registry.remediation_attempts
            FROM runtime_vm_instances instances
            JOIN runtime_vm_trust_registry registry
                ON registry.runtime_vm_instance_id = instances.id
            WHERE instances.server_id = $1
            ORDER BY registry.updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(row.server_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(trust) = trust_row {
            signals.trust_lifecycle_state = Some(trust.lifecycle_state);
            signals.trust_attestation_status = Some(trust.attestation_status);
            signals.trust_remediation_state = trust.remediation_state;
            signals.trust_remediation_attempts = Some(trust.remediation_attempts);
        }

        let remediation_row = query_as::<_, RemediationSignalRow>(
            r#"
            SELECT runs.status, runs.failure_reason
            FROM runtime_vm_remediation_runs runs
            JOIN runtime_vm_instances instances
                ON instances.id = runs.runtime_vm_instance_id
            WHERE instances.server_id = $1
            ORDER BY runs.updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(row.server_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(remediation) = remediation_row {
            signals.remediation_status = Some(remediation.status);
            signals.remediation_failure_reason = remediation.failure_reason;
        }

        let intelligence_rows = query_as::<_, IntelligenceRow>(
            r#"
            SELECT capability, status, score::float4 AS score, confidence::float4 AS confidence
            FROM capability_intelligence_scores
            WHERE server_id = $1
            ORDER BY last_observed_at DESC
            LIMIT 10
            "#,
        )
        .bind(row.server_id)
        .fetch_all(&mut *tx)
        .await?;

        signals.intelligence = intelligence_rows
            .into_iter()
            .map(|row| IntelligenceSignal {
                capability: row.capability,
                status: row.status,
                score: row.score,
                confidence: row.confidence,
            })
            .collect();
    }
    Ok(signals)
}

fn evaluate_promotion_posture(
    track: &PromotionTrack,
    signals: &PromotionPostureSignals,
) -> PromotionVerdict {
    let mut allowed = true;
    let mut veto_reasons = Vec::new();
    let mut posture_notes = Vec::new();

    let mut artifact_map = Map::new();
    let mut trust_map = Map::new();
    let mut remediation_map = Map::new();
    let mut signals_map = Map::new();

    if let Some(status) = signals.artifact_status.as_ref() {
        artifact_map.insert("status".to_string(), json!(status));
        posture_notes.push(format!("posture:artifact.status:{status}"));
    } else {
        artifact_map.insert("status".to_string(), Value::Null);
        posture_notes.push("posture:artifact.status:missing".to_string());
    }

    if let Some(credential) = signals.credential_health_status.as_ref() {
        artifact_map.insert("credential_health_status".to_string(), json!(credential));
        posture_notes.push(format!("posture:artifact.credential_health:{credential}"));
        if credential != "healthy" {
            allowed = false;
            veto_reasons.push(format!("artifact.credential_health={credential}"));
        }
    }

    if let Some(lifecycle) = signals.trust_lifecycle_state.as_ref() {
        trust_map.insert("lifecycle_state".to_string(), json!(lifecycle));
        posture_notes.push(format!("posture:trust.lifecycle_state:{lifecycle}"));
        if lifecycle != "trusted" {
            allowed = false;
            veto_reasons.push(format!("trust.lifecycle_state={lifecycle}"));
        }
    }

    if let Some(attestation) = signals.trust_attestation_status.as_ref() {
        trust_map.insert("attestation_status".to_string(), json!(attestation));
        posture_notes.push(format!("posture:trust.attestation_status:{attestation}"));
        if attestation != "trusted" && attestation != "certified" {
            allowed = false;
            veto_reasons.push(format!("trust.attestation_status={attestation}"));
        }
    }

    if let Some(state) = signals.trust_remediation_state.as_ref() {
        trust_map.insert("remediation_state".to_string(), json!(state));
        posture_notes.push(format!("posture:trust.remediation_state:{state}"));
        if state != "remediation:none" && state != "remediation:clear" {
            allowed = false;
            veto_reasons.push(format!("trust.remediation_state={state}"));
        }
    }

    if let Some(attempts) = signals.trust_remediation_attempts {
        trust_map.insert("remediation_attempts".to_string(), json!(attempts));
        posture_notes.push(format!("posture:trust.remediation_attempts:{attempts}"));
        if attempts > 3 {
            allowed = false;
            veto_reasons.push(format!("trust.remediation_attempts={attempts}"));
        }
    }

    if let Some(remediation_status) = signals.remediation_status.as_ref() {
        remediation_map.insert("status".to_string(), json!(remediation_status));
        posture_notes.push(format!("posture:remediation.status:{remediation_status}"));
        if remediation_status == "failed" || remediation_status == "cancelled" {
            allowed = false;
            veto_reasons.push(format!("remediation.status={remediation_status}"));
        }
    }

    if let Some(failure) = signals.remediation_failure_reason.as_ref() {
        remediation_map.insert("failure_reason".to_string(), json!(failure));
        posture_notes.push(format!("posture:remediation.failure_reason:{failure}"));
    }

    if !artifact_map.is_empty() {
        signals_map.insert("artifact".to_string(), Value::Object(artifact_map));
    }
    if !trust_map.is_empty() {
        signals_map.insert("trust".to_string(), Value::Object(trust_map));
    }
    if !remediation_map.is_empty() {
        signals_map.insert("remediation".to_string(), Value::Object(remediation_map));
    }

    if !signals.intelligence.is_empty() {
        let intel_metadata: Vec<Value> = signals
            .intelligence
            .iter()
            .map(|signal| {
                json!({
                    "capability": signal.capability,
                    "status": signal.status,
                    "score": signal.score,
                    "confidence": signal.confidence,
                })
            })
            .collect();
        signals_map.insert("intelligence".to_string(), Value::Array(intel_metadata));

        for intel in &signals.intelligence {
            posture_notes.push(format!(
                "posture:intelligence.{}:{}:{:.1}",
                intel.capability, intel.status, intel.score
            ));
            if intel.status.eq_ignore_ascii_case("critical") || intel.score < 60.0 {
                allowed = false;
                veto_reasons.push(format!(
                    "intelligence.{}={}:{}",
                    intel.capability,
                    intel.status,
                    format!("{:.1}", intel.score)
                ));
            }
        }
    }

    let mut root = Map::new();
    root.insert(
        "track".to_string(),
        json!({
            "id": track.id,
            "name": track.name,
            "tier": track.tier,
        }),
    );
    root.insert("signals".to_string(), Value::Object(signals_map));

    PromotionVerdict {
        allowed,
        veto_reasons,
        metadata: Value::Object(root),
        posture_notes,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_verdict_payload, evaluate_promotion_posture, IntelligenceSignal,
        PromotionPostureSignals, PromotionTrack, ReleaseTrain,
    };

    #[test]
    fn release_train_defaults_when_missing() {
        let train = ReleaseTrain::new(vec![]);
        assert!(train.contains("candidate"));
        assert_eq!(train.previous_stage("staging"), Some("candidate".into()));
        assert_eq!(train.previous_stage("candidate"), None);
    }

    #[test]
    fn release_train_respects_case_insensitive_lookup() {
        let train = ReleaseTrain::new(vec!["Alpha".into(), "BETA".into(), "GA".into()]);
        assert!(train.contains("beta"));
        assert_eq!(train.previous_stage("GA"), Some("beta".into()));
    }

    #[test]
    fn promotion_verdict_allows_trusted_posture() {
        let track = PromotionTrack {
            id: 1,
            owner_id: 7,
            name: "Mainline".to_string(),
            tier: "stable".to_string(),
            stages: vec!["candidate".into(), "prod".into()],
            description: None,
            workflow_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let signals = PromotionPostureSignals {
            artifact_status: Some("completed".to_string()),
            credential_health_status: Some("healthy".to_string()),
            trust_lifecycle_state: Some("trusted".to_string()),
            trust_attestation_status: Some("trusted".to_string()),
            trust_remediation_state: Some("remediation:none".to_string()),
            trust_remediation_attempts: Some(0),
            remediation_status: Some("succeeded".to_string()),
            remediation_failure_reason: None,
            intelligence: vec![IntelligenceSignal {
                capability: "runtime".to_string(),
                status: "healthy".to_string(),
                score: 92.0,
                confidence: 0.9,
            }],
        };

        let verdict = evaluate_promotion_posture(&track, &signals);
        assert!(verdict.allowed);
        assert!(verdict.veto_reasons.is_empty());
        assert!(verdict
            .metadata
            .get("signals")
            .and_then(|signals| signals.get("trust"))
            .is_some());
    }

    #[test]
    fn promotion_verdict_blocks_on_critical_intelligence() {
        let track = PromotionTrack {
            id: 99,
            owner_id: 3,
            name: "FastTrack".to_string(),
            tier: "beta".to_string(),
            stages: vec!["preprod".into(), "prod".into()],
            description: None,
            workflow_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let signals = PromotionPostureSignals {
            artifact_status: Some("completed".to_string()),
            credential_health_status: Some("healthy".to_string()),
            trust_lifecycle_state: Some("trusted".to_string()),
            trust_attestation_status: Some("trusted".to_string()),
            trust_remediation_state: Some("remediation:none".to_string()),
            trust_remediation_attempts: Some(0),
            remediation_status: Some("succeeded".to_string()),
            remediation_failure_reason: None,
            intelligence: vec![IntelligenceSignal {
                capability: "supply".to_string(),
                status: "critical".to_string(),
                score: 48.5,
                confidence: 0.7,
            }],
        };

        let verdict = evaluate_promotion_posture(&track, &signals);
        assert!(!verdict.allowed);
        assert!(verdict
            .veto_reasons
            .iter()
            .any(|reason| reason.contains("intelligence.supply")));
    }

    #[test]
    fn verdict_payload_captures_track_and_stage() {
        let track = PromotionTrack {
            id: 7,
            owner_id: 9,
            name: "Release".to_string(),
            tier: "gold".to_string(),
            stages: vec!["candidate".into(), "production".into()],
            description: None,
            workflow_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let signals = PromotionPostureSignals {
            artifact_status: Some("completed".into()),
            credential_health_status: Some("degraded".into()),
            trust_lifecycle_state: Some("quarantined".into()),
            trust_attestation_status: Some("critical".into()),
            trust_remediation_state: Some("remediation:pending".into()),
            trust_remediation_attempts: Some(3),
            remediation_status: Some("failed".into()),
            remediation_failure_reason: Some("policy".into()),
            intelligence: vec![],
        };

        let verdict = evaluate_promotion_posture(&track, &signals);
        assert!(!verdict.allowed);

        let payload = build_verdict_payload(&track, "production", &verdict);
        let root = payload.as_object().expect("payload should be an object");
        assert_eq!(
            root.get("stage").and_then(|value| value.as_str()),
            Some("production")
        );
        let track_obj = root
            .get("track")
            .and_then(|value| value.as_object())
            .expect("track metadata expected");
        assert_eq!(
            track_obj.get("name").and_then(|value| value.as_str()),
            Some("Release")
        );
        assert!(root
            .get("reasons")
            .and_then(|value| value.as_array())
            .map(|entries| !entries.is_empty())
            .unwrap_or(false));
    }
}
