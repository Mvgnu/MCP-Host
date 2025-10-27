// key: governance-workflows
use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;

use super::models::{
    CreateGovernanceWorkflow, GovernanceAuditLogEntry, GovernanceRunDetail, GovernanceRunStatus,
    GovernanceStepRunDetail, GovernanceWorkflow, GovernanceWorkflowKind, RunStatusUpdateRequest,
    StartWorkflowRunRequest,
};

#[derive(Debug, Clone, Default)]
pub struct GovernanceEngine;

#[derive(Debug, Clone)]
pub struct GovernanceGateEvaluation {
    pub satisfied: bool,
    pub run_id: Option<i64>,
    pub notes: Vec<String>,
    pub promotion_track_id: Option<i32>,
    pub promotion_track_name: Option<String>,
    pub promotion_stage: Option<String>,
    pub promotion_status: Option<String>,
}

#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("workflow not found")]
    NotFound,
    #[error("workflow access denied")]
    Forbidden,
}

#[derive(Debug, Clone)]
pub struct RunTransitionOutcome {
    pub run_id: i64,
    pub workflow_id: i32,
    pub status: GovernanceRunStatus,
    pub policy_decision_id: Option<i32>,
}

impl GovernanceEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn list_workflows(
        &self,
        pool: &PgPool,
        owner_id: i32,
    ) -> Result<Vec<GovernanceWorkflow>, GovernanceError> {
        let workflows = sqlx::query_as::<_, GovernanceWorkflow>(
            r#"
            SELECT id, owner_id, name, workflow_type, tier, created_at, updated_at
            FROM governance_workflows
            WHERE owner_id = $1
            ORDER BY id
            "#,
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await?;
        Ok(workflows)
    }

    pub async fn create_workflow(
        &self,
        pool: &PgPool,
        owner_id: i32,
        payload: CreateGovernanceWorkflow,
    ) -> Result<GovernanceWorkflow, GovernanceError> {
        let mut tx: Transaction<'_, Postgres> = pool.begin().await?;

        let workflow = sqlx::query_as::<_, GovernanceWorkflow>(
            r#"
            INSERT INTO governance_workflows (owner_id, name, workflow_type, tier)
            VALUES ($1, $2, $3, $4)
            RETURNING id, owner_id, name, workflow_type, tier, created_at, updated_at
            "#,
        )
        .bind(owner_id)
        .bind(&payload.name)
        .bind(payload.workflow_type as GovernanceWorkflowKind)
        .bind(&payload.tier)
        .fetch_one(&mut *tx)
        .await?;

        for (idx, step) in payload.steps.into_iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO governance_workflow_steps (workflow_id, position, action, config)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(workflow.id)
            .bind((idx + 1) as i32)
            .bind(step.action)
            .bind(step.config)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(workflow)
    }

    pub async fn start_workflow_run(
        &self,
        pool: &PgPool,
        workflow_id: i32,
        owner_id: i32,
        payload: StartWorkflowRunRequest,
    ) -> Result<GovernanceRunDetail, GovernanceError> {
        let workflow = sqlx::query_as::<_, GovernanceWorkflow>(
            r#"
            SELECT id, owner_id, name, workflow_type, tier, created_at, updated_at
            FROM governance_workflows
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(pool)
        .await?;

        let Some(workflow) = workflow else {
            return Err(GovernanceError::NotFound);
        };
        if workflow.owner_id != owner_id {
            return Err(GovernanceError::Forbidden);
        }

        let StartWorkflowRunRequest {
            target_manifest_digest,
            target_artifact_run_id,
            notes,
            promotion_track_id,
            promotion_stage,
        } = payload;

        let notes = notes.unwrap_or_default();
        let normalized_stage = promotion_stage.map(|stage| stage.to_lowercase());

        let mut tx: Transaction<'_, Postgres> = pool.begin().await?;

        let run_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO governance_workflow_runs (
                workflow_id,
                initiated_by,
                target_artifact_run_id,
                target_manifest_digest,
                target_tier,
                notes,
                promotion_track_id,
                promotion_stage
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id
            "#,
        )
        .bind(workflow.id)
        .bind(owner_id)
        .bind(target_artifact_run_id)
        .bind(&target_manifest_digest)
        .bind(&workflow.tier)
        .bind(&notes)
        .bind(promotion_track_id)
        .bind(normalized_stage.clone())
        .fetch_one(&mut *tx)
        .await?;

        let steps = sqlx::query(
            r#"
            SELECT id
            FROM governance_workflow_steps
            WHERE workflow_id = $1
            ORDER BY position
            "#,
        )
        .bind(workflow.id)
        .fetch_all(&mut *tx)
        .await?;

        for row in steps {
            let step_id: i32 = row.get("id");
            sqlx::query(
                r#"
                INSERT INTO governance_step_runs (workflow_run_id, step_id)
                VALUES ($1, $2)
                "#,
            )
            .bind(run_id)
            .bind(step_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        self.fetch_run_detail(pool, run_id, owner_id).await
    }

    pub async fn fetch_run_detail(
        &self,
        pool: &PgPool,
        run_id: i64,
        owner_id: i32,
    ) -> Result<GovernanceRunDetail, GovernanceError> {
        let rec = sqlx::query(
            r#"
            SELECT r.id,
                   r.workflow_id,
                   r.status,
                   r.notes,
                   r.target_manifest_digest,
                   r.target_tier,
                   r.promotion_track_id,
                   r.promotion_stage,
                   r.initiated_by,
                   r.created_at,
                   r.updated_at
            FROM governance_workflow_runs r
            JOIN governance_workflows w ON w.id = r.workflow_id
            WHERE r.id = $1 AND w.owner_id = $2
            "#,
        )
        .bind(run_id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        let Some(row) = rec else {
            return Err(GovernanceError::NotFound);
        };

        let steps = sqlx::query_as::<_, GovernanceStepRunDetail>(
            r#"
            SELECT sr.id,
                   sr.step_id,
                   s.action,
                   sr.status,
                   sr.started_at,
                   sr.completed_at,
                   sr.error
            FROM governance_step_runs sr
            LEFT JOIN governance_workflow_steps s ON s.id = sr.step_id
            WHERE sr.workflow_run_id = $1
            ORDER BY sr.id
            "#,
        )
        .bind(run_id)
        .fetch_all(pool)
        .await?;

        let audit_log = sqlx::query_as::<_, GovernanceAuditLogEntry>(
            r#"
            SELECT id, event_type, details, created_at, actor_id
            FROM governance_audit_logs
            WHERE workflow_run_id = $1
            ORDER BY created_at, id
            "#,
        )
        .bind(run_id)
        .fetch_all(pool)
        .await?;

        let detail = GovernanceRunDetail {
            id: row.get("id"),
            workflow_id: row.get("workflow_id"),
            status: row.get("status"),
            notes: row
                .get::<Option<Vec<String>>, _>("notes")
                .unwrap_or_default(),
            target_manifest_digest: row.get("target_manifest_digest"),
            target_tier: row.get("target_tier"),
            promotion_track_id: row.get("promotion_track_id"),
            promotion_stage: row.get("promotion_stage"),
            initiated_by: row.get("initiated_by"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            steps,
            audit_log,
        };
        Ok(detail)
    }

    pub async fn update_run_status(
        &self,
        pool: &PgPool,
        run_id: i64,
        owner_id: i32,
        payload: RunStatusUpdateRequest,
    ) -> Result<Option<RunTransitionOutcome>, GovernanceError> {
        let res = sqlx::query(
            r#"
            UPDATE governance_workflow_runs r
            SET status = $3,
                notes = CASE WHEN $4 IS NULL THEN r.notes ELSE array_append(r.notes, $4) END,
                updated_at = NOW()
            FROM governance_workflows w
            WHERE r.id = $1
              AND r.workflow_id = w.id
              AND w.owner_id = $2
            RETURNING r.id, r.workflow_id, r.status, r.policy_decision_id
            "#,
        )
        .bind(run_id)
        .bind(owner_id)
        .bind(payload.status as GovernanceRunStatus)
        .bind(&payload.note)
        .fetch_optional(pool)
        .await?;

        let Some(row) = res else {
            return Ok(None);
        };

        let audit_payload = if let Some(note) = &payload.note {
            json!({"status": payload.status, "note": note})
        } else {
            json!({"status": payload.status})
        };

        sqlx::query(
            r#"
            INSERT INTO governance_audit_logs (workflow_run_id, actor_id, event_type, details)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(run_id)
        .bind(owner_id)
        .bind("status_change")
        .bind(audit_payload)
        .execute(pool)
        .await?;

        self.sync_promotion_status(pool, row.get("id"), payload.status)
            .await?;

        match payload.status {
            GovernanceRunStatus::Completed => {
                sqlx::query(
                    r#"
                    UPDATE governance_step_runs
                    SET status = 'completed'::governance_step_status,
                        completed_at = COALESCE(completed_at, NOW())
                    WHERE workflow_run_id = $1 AND status <> 'completed'::governance_step_status
                    "#,
                )
                .bind(run_id)
                .execute(pool)
                .await?;
            }
            GovernanceRunStatus::Failed => {
                sqlx::query(
                    r#"
                    UPDATE governance_step_runs
                    SET status = 'failed'::governance_step_status,
                        completed_at = COALESCE(completed_at, NOW())
                    WHERE workflow_run_id = $1 AND status NOT IN (
                        'failed'::governance_step_status,
                        'completed'::governance_step_status
                    )
                    "#,
                )
                .bind(run_id)
                .execute(pool)
                .await?;
            }
            _ => {}
        }

        Ok(Some(RunTransitionOutcome {
            run_id: row.get("id"),
            workflow_id: row.get("workflow_id"),
            status: row.get("status"),
            policy_decision_id: row.get("policy_decision_id"),
        }))
    }

    async fn sync_promotion_status(
        &self,
        pool: &PgPool,
        run_id: i64,
        status: GovernanceRunStatus,
    ) -> Result<(), GovernanceError> {
        let rec = sqlx::query(
            r#"
            SELECT promotion_track_id, promotion_stage, target_manifest_digest
            FROM governance_workflow_runs
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .fetch_optional(pool)
        .await?;

        let Some(row) = rec else {
            return Ok(());
        };

        let track_id: Option<i32> = row.get("promotion_track_id");
        let stage: Option<String> = row.get("promotion_stage");
        let digest: Option<String> = row.get("target_manifest_digest");

        let (Some(track_id), Some(stage), Some(digest)) = (track_id, stage, digest) else {
            return Ok(());
        };

        let status_str = match status {
            GovernanceRunStatus::Pending => "scheduled",
            GovernanceRunStatus::InProgress => "in_progress",
            GovernanceRunStatus::Completed => "active",
            GovernanceRunStatus::Failed | GovernanceRunStatus::Cancelled => "rolled_back",
        };

        let note = format!("promotion:governance:{run_id}:{status_str}");

        sqlx::query(
            r#"
            UPDATE artifact_promotions
            SET status = $1,
                updated_at = NOW(),
                activated_at = CASE WHEN $1 = 'active' THEN NOW() ELSE activated_at END,
                notes = array_append(notes, $2),
                workflow_run_id = COALESCE(workflow_run_id, $3)
            WHERE promotion_track_id = $4
              AND stage = $5
              AND manifest_digest = $6
            "#,
        )
        .bind(status_str)
        .bind(note)
        .bind(run_id)
        .bind(track_id)
        .bind(stage.to_lowercase())
        .bind(digest)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn ensure_promotion_ready(
        &self,
        pool: &PgPool,
        manifest_digest: Option<&str>,
        tier: Option<&str>,
    ) -> Result<GovernanceGateEvaluation, GovernanceError> {
        let mut notes = Vec::new();
        let Some(digest) = manifest_digest else {
            notes.push("governance:missing-manifest".to_string());
            return Ok(GovernanceGateEvaluation {
                satisfied: false,
                run_id: None,
                notes,
                promotion_track_id: None,
                promotion_track_name: None,
                promotion_stage: None,
                promotion_status: None,
            });
        };
        let Some(tier_value) = tier else {
            notes.push("governance:missing-tier".to_string());
            return Ok(GovernanceGateEvaluation {
                satisfied: false,
                run_id: None,
                notes,
                promotion_track_id: None,
                promotion_track_name: None,
                promotion_stage: None,
                promotion_status: None,
            });
        };

        let row = sqlx::query(
            r#"
            SELECT ap.id,
                   ap.promotion_track_id,
                   ap.stage,
                   ap.status,
                   ap.workflow_run_id,
                   ap.notes,
                   t.name AS track_name
            FROM artifact_promotions ap
            JOIN promotion_tracks t ON t.id = ap.promotion_track_id
            WHERE ap.manifest_digest = $1
              AND t.tier = $2
            ORDER BY CASE ap.status
                        WHEN 'active' THEN 0
                        WHEN 'approved' THEN 1
                        WHEN 'in_progress' THEN 2
                        WHEN 'scheduled' THEN 3
                        ELSE 4
                     END,
                     ap.updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(digest)
        .bind(tier_value)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = row {
            let status: String = row.get("status");
            let stage_value: String = row.get("stage");
            let track_id: i32 = row.get("promotion_track_id");
            let track_name: String = row.get("track_name");
            let run_id: Option<i64> = row.get("workflow_run_id");
            let mut record_notes = row
                .get::<Option<Vec<String>>, _>("notes")
                .unwrap_or_default();

            let stage_matches = stage_value.eq_ignore_ascii_case(tier_value);
            let satisfied = stage_matches && status == "active";

            notes.push(format!(
                "promotion:status:{}:{}:{}",
                track_name, stage_value, status
            ));
            if !stage_matches {
                notes.push(format!(
                    "promotion:stage-mismatch:{}:{}",
                    stage_value, tier_value
                ));
            }
            notes.append(&mut record_notes);

            Ok(GovernanceGateEvaluation {
                satisfied,
                run_id,
                notes,
                promotion_track_id: Some(track_id),
                promotion_track_name: Some(track_name),
                promotion_stage: Some(stage_value),
                promotion_status: Some(status),
            })
        } else {
            notes.push(format!("promotion:missing-track:{tier_value}"));
            Ok(GovernanceGateEvaluation {
                satisfied: false,
                run_id: None,
                notes,
                promotion_track_id: None,
                promotion_track_name: None,
                promotion_stage: None,
                promotion_status: None,
            })
        }
    }

    pub async fn attach_policy_decision(
        &self,
        pool: &PgPool,
        run_id: i64,
        decision_id: i32,
    ) -> Result<(), GovernanceError> {
        sqlx::query(
            r#"
            UPDATE governance_workflow_runs
            SET policy_decision_id = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .bind(decision_id)
        .execute(pool)
        .await?;
        Ok(())
    }
}
