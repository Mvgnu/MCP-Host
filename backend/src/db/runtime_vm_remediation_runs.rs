use serde_json::Value;
use sqlx::{Executor, Postgres};

// key: remediation-db -> automation-tracking
pub async fn ensure_running_playbook<'c, E>(
    executor: E,
    runtime_vm_instance_id: i64,
    playbook: &str,
    payload: Option<&Value>,
    approval_required: bool,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let row = sqlx::query(
        r#"
        INSERT INTO runtime_vm_remediation_runs (
            runtime_vm_instance_id,
            playbook,
            status,
            automation_payload,
            approval_required
        )
        SELECT $1, $2, 'running', $3, $4
        WHERE NOT EXISTS (
            SELECT 1
            FROM runtime_vm_remediation_runs
            WHERE runtime_vm_instance_id = $1
              AND status = 'running'
        )
        RETURNING id
        "#,
    )
    .bind(runtime_vm_instance_id)
    .bind(playbook)
    .bind(payload)
    .bind(approval_required)
    .fetch_optional(executor)
    .await?;

    Ok(row.is_some())
}

pub async fn mark_run_completed<'c, E>(
    executor: E,
    runtime_vm_instance_id: i64,
    payload: Option<&Value>,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let row = sqlx::query(
        r#"
        UPDATE runtime_vm_remediation_runs
        SET
            status = 'completed',
            automation_payload = COALESCE($2, automation_payload),
            completed_at = NOW()
        WHERE runtime_vm_instance_id = $1
          AND status = 'running'
        RETURNING id
        "#,
    )
    .bind(runtime_vm_instance_id)
    .bind(payload)
    .fetch_optional(executor)
    .await?;

    Ok(row.is_some())
}

pub async fn mark_run_failed<'c, E>(
    executor: E,
    runtime_vm_instance_id: i64,
    error: &str,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let row = sqlx::query(
        r#"
        UPDATE runtime_vm_remediation_runs
        SET
            status = 'failed',
            last_error = $2,
            completed_at = NOW()
        WHERE runtime_vm_instance_id = $1
          AND status = 'running'
        RETURNING id
        "#,
    )
    .bind(runtime_vm_instance_id)
    .bind(error)
    .fetch_optional(executor)
    .await?;

    Ok(row.is_some())
}
