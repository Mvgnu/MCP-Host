use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres, QueryBuilder};

// key: remediation-db -> run-tracking
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationRun {
    pub id: i64,
    pub runtime_vm_instance_id: i64,
    pub playbook: String,
    pub playbook_id: Option<i64>,
    pub status: String,
    pub automation_payload: Option<Value>,
    pub approval_required: bool,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub assigned_owner_id: Option<i32>,
    pub sla_deadline: Option<DateTime<Utc>>,
    pub approval_state: String,
    pub approval_decided_at: Option<DateTime<Utc>>,
    pub approval_notes: Option<String>,
    pub metadata: Value,
    pub workspace_id: Option<i64>,
    pub workspace_revision_id: Option<i64>,
    pub promotion_gate_context: Value,
    pub version: i64,
    pub updated_at: DateTime<Utc>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancellation_reason: Option<String>,
    pub failure_reason: Option<String>,
}

pub struct ListRuntimeVmRemediationRuns<'a> {
    pub runtime_vm_instance_id: Option<i64>,
    pub status: Option<&'a str>,
    pub workspace_id: Option<i64>,
    pub workspace_revision_id: Option<i64>,
}

pub async fn list_runs(
    pool: &PgPool,
    filter: ListRuntimeVmRemediationRuns<'_>,
) -> Result<Vec<RuntimeVmRemediationRun>, sqlx::Error> {
    let mut builder = QueryBuilder::new(
        "SELECT id, runtime_vm_instance_id, playbook, playbook_id, status, automation_payload, \\n         approval_required, started_at, completed_at, last_error, assigned_owner_id, sla_deadline, \\n         approval_state, approval_decided_at, approval_notes, metadata, workspace_id, \\n         workspace_revision_id, promotion_gate_context, version, updated_at, cancelled_at, \\n         cancellation_reason, failure_reason FROM runtime_vm_remediation_runs",
    );
    if filter.runtime_vm_instance_id.is_some()
        || filter.status.is_some()
        || filter.workspace_id.is_some()
        || filter.workspace_revision_id.is_some()
    {
        builder.push(" WHERE ");
    }

    let mut has_clause = false;

    if let Some(instance_id) = filter.runtime_vm_instance_id {
        builder.push(" runtime_vm_instance_id = ");
        builder.push_bind(instance_id);
        has_clause = true;
    }

    if let Some(status) = filter.status {
        if has_clause {
            builder.push(" AND ");
        }
        builder.push(" status = ");
        builder.push_bind(status);
        has_clause = true;
    }

    if let Some(workspace_id) = filter.workspace_id {
        if has_clause {
            builder.push(" AND ");
        }
        builder.push(" workspace_id = ");
        builder.push_bind(workspace_id);
        has_clause = true;
    }

    if let Some(revision_id) = filter.workspace_revision_id {
        if has_clause {
            builder.push(" AND ");
        }
        builder.push(" workspace_revision_id = ");
        builder.push_bind(revision_id);
    }

    builder.push(" ORDER BY started_at DESC");

    builder
        .build_query_as::<RuntimeVmRemediationRun>()
        .fetch_all(pool)
        .await
}

pub async fn get_run_by_id(
    pool: &PgPool,
    run_id: i64,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmRemediationRun>(
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
        FROM runtime_vm_remediation_runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

pub struct EnsureRemediationRunRequest<'a> {
    pub runtime_vm_instance_id: i64,
    pub playbook_key: &'a str,
    pub playbook_id: Option<i64>,
    pub metadata: Option<&'a Value>,
    pub automation_payload: Option<&'a Value>,
    pub approval_required: bool,
    pub assigned_owner_id: Option<i32>,
    pub sla_duration_seconds: Option<i32>,
    pub workspace_id: Option<i64>,
    pub workspace_revision_id: Option<i64>,
    pub promotion_gate_context: Option<&'a Value>,
}

pub async fn ensure_remediation_run<'c, E>(
    executor: E,
    request: EnsureRemediationRunRequest<'_>,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_as::<_, RuntimeVmRemediationRun>(
        r#"
        WITH inserted AS (
            INSERT INTO runtime_vm_remediation_runs (
                runtime_vm_instance_id,
                playbook,
                playbook_id,
                status,
                automation_payload,
                approval_required,
                approval_state,
                approval_decided_at,
                assigned_owner_id,
                sla_deadline,
                metadata,
                workspace_id,
                workspace_revision_id,
                promotion_gate_context
            )
            SELECT
                $1,
                $2,
                $3,
                'pending',
                $4,
                $5,
                CASE WHEN $5 THEN 'pending' ELSE 'auto-approved' END,
                CASE WHEN $5 THEN NULL ELSE NOW() END,
                $6,
                CASE
                    WHEN $7 IS NULL THEN NULL
                    ELSE NOW() + ($7::INT * INTERVAL '1 second')
                END,
                COALESCE($8, '{}'::JSONB),
                $9,
                $10,
                COALESCE($11, '{}'::JSONB)
            WHERE NOT EXISTS (
                SELECT 1
                FROM runtime_vm_remediation_runs
                WHERE runtime_vm_instance_id = $1
                  AND status IN ('pending', 'running')
            )
            RETURNING
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
        )
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
        FROM inserted
        "#,
    )
    .bind(request.runtime_vm_instance_id)
    .bind(request.playbook_key)
    .bind(request.playbook_id)
    .bind(request.automation_payload)
    .bind(request.approval_required)
    .bind(request.assigned_owner_id)
    .bind(request.sla_duration_seconds)
    .bind(request.metadata)
    .bind(request.workspace_id)
    .bind(request.workspace_revision_id)
    .bind(request.promotion_gate_context)
    .fetch_optional(executor)
    .await?;

    Ok(record)
}

pub async fn get_active_run_for_instance<'c, E>(
    executor: E,
    runtime_vm_instance_id: i64,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query_as::<_, RuntimeVmRemediationRun>(
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
        FROM runtime_vm_remediation_runs
        WHERE runtime_vm_instance_id = $1
          AND status IN ('pending', 'running')
        ORDER BY started_at DESC
        LIMIT 1
        "#,
    )
    .bind(runtime_vm_instance_id)
    .fetch_optional(executor)
    .await
}

pub async fn try_acquire_next_run<'c, E>(
    executor: E,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_as::<_, RuntimeVmRemediationRun>(
        r#"
        WITH candidate AS (
            SELECT id
            FROM runtime_vm_remediation_runs
            WHERE status = 'pending'
              AND approval_state IN ('approved', 'auto-approved')
            ORDER BY COALESCE(sla_deadline, started_at), started_at
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE runtime_vm_remediation_runs AS runs
        SET
            status = 'running',
            version = runs.version + 1,
            updated_at = NOW()
        FROM candidate
        WHERE runs.id = candidate.id
        RETURNING
            runs.id,
            runs.runtime_vm_instance_id,
            runs.playbook,
            runs.playbook_id,
            runs.status,
            runs.automation_payload,
            runs.approval_required,
            runs.started_at,
            runs.completed_at,
            runs.last_error,
            runs.assigned_owner_id,
            runs.sla_deadline,
            runs.approval_state,
            runs.approval_decided_at,
            runs.approval_notes,
            runs.metadata,
            runs.workspace_id,
            runs.workspace_revision_id,
            runs.promotion_gate_context,
            runs.version,
            runs.updated_at,
            runs.cancelled_at,
            runs.cancellation_reason,
            runs.failure_reason
        "#,
    )
    .fetch_optional(executor)
    .await?;

    Ok(record)
}

pub async fn update_run_workspace_linkage<'c, E>(
    executor: E,
    run_id: i64,
    workspace_id: i64,
    workspace_revision_id: i64,
    promotion_gate_context: &Value,
    metadata: Option<&Value>,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_as::<_, RuntimeVmRemediationRun>(
        r#"
        UPDATE runtime_vm_remediation_runs
        SET
            workspace_id = $2,
            workspace_revision_id = $3,
            promotion_gate_context = $4,
            metadata = CASE
                WHEN $5 IS NULL THEN metadata
                ELSE COALESCE(metadata, '{}'::jsonb) || $5::jsonb
            END,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1
        RETURNING
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
        "#,
    )
    .bind(run_id)
    .bind(workspace_id)
    .bind(workspace_revision_id)
    .bind(promotion_gate_context)
    .bind(metadata)
    .fetch_optional(executor)
    .await?;

    Ok(record)
}

pub async fn mark_run_completed<'c, E>(
    executor: E,
    run_id: i64,
    metadata: Option<&Value>,
    payload: Option<&Value>,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_as::<_, RuntimeVmRemediationRun>(
        r#"
        UPDATE runtime_vm_remediation_runs
        SET
            status = 'completed',
            automation_payload = COALESCE($3, automation_payload),
            metadata = COALESCE($2, metadata),
            completed_at = NOW(),
            version = version + 1,
            updated_at = NOW(),
            failure_reason = NULL,
            last_error = NULL
        WHERE id = $1
          AND status = 'running'
        RETURNING
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
        "#,
    )
    .bind(run_id)
    .bind(metadata)
    .bind(payload)
    .fetch_optional(executor)
    .await?;

    Ok(record)
}

pub async fn mark_run_failed<'c, E>(
    executor: E,
    run_id: i64,
    failure_reason: &str,
    error: &str,
    metadata: Option<&Value>,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_as::<_, RuntimeVmRemediationRun>(
        r#"
        UPDATE runtime_vm_remediation_runs
        SET
            status = 'failed',
            last_error = $3,
            failure_reason = $2,
            metadata = COALESCE($4, metadata),
            completed_at = NOW(),
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1
          AND status = 'running'
        RETURNING
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
        "#,
    )
    .bind(run_id)
    .bind(failure_reason)
    .bind(error)
    .bind(metadata)
    .fetch_optional(executor)
    .await?;

    Ok(record)
}

pub struct UpdateApprovalState<'a> {
    pub run_id: i64,
    pub new_state: &'a str,
    pub approval_notes: Option<&'a str>,
    pub decided_at: DateTime<Utc>,
    pub expected_version: i64,
}

pub async fn update_approval_state<'c, E>(
    executor: E,
    update: UpdateApprovalState<'_>,
) -> Result<Option<RuntimeVmRemediationRun>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let record = sqlx::query_as::<_, RuntimeVmRemediationRun>(
        r#"
        UPDATE runtime_vm_remediation_runs
        SET
            approval_state = $2,
            approval_notes = $3,
            approval_decided_at = $4,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1
          AND version = $5
        RETURNING
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
        "#,
    )
    .bind(update.run_id)
    .bind(update.new_state)
    .bind(update.approval_notes)
    .bind(update.decided_at)
    .bind(update.expected_version)
    .fetch_optional(executor)
    .await?;

    Ok(record)
}
