use std::sync::Arc;

use axum::{
    extract::{Extension, Path, Query},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, QueryBuilder};
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
        notes,
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

    let record_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO artifact_promotions (
            promotion_track_id,
            manifest_digest,
            artifact_run_id,
            stage,
            status,
            scheduled_by,
            notes
        ) VALUES ($1, $2, $3, $4, 'scheduled', $5, $6)
        RETURNING id
        "#,
    )
    .bind(track.id)
    .bind(&manifest_digest)
    .bind(artifact_run_id)
    .bind(&stage)
    .bind(user_id)
    .bind(&notes)
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
         ap.workflow_run_id, ap.scheduled_by, ap.approved_by, ap.notes, ap.scheduled_at, ap.approved_at, \
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
               ap.workflow_run_id, ap.scheduled_by, ap.approved_by, ap.notes, ap.scheduled_at, ap.approved_at,
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

#[cfg(test)]
mod tests {
    use super::ReleaseTrain;

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
}
