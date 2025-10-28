use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres};

// key: remediation-db -> playbook-catalog
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationPlaybook {
    pub id: i64,
    pub playbook_key: String,
    pub display_name: String,
    pub description: Option<String>,
    pub executor_type: String,
    pub owner_id: i32,
    pub approval_required: bool,
    pub sla_duration_seconds: Option<i32>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

pub async fn list_playbooks(
    pool: &PgPool,
) -> Result<Vec<RuntimeVmRemediationPlaybook>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmRemediationPlaybook>(
        r#"
        SELECT
            id,
            playbook_key,
            display_name,
            description,
            executor_type,
            owner_id,
            approval_required,
            sla_duration_seconds,
            metadata,
            created_at,
            updated_at,
            version
        FROM runtime_vm_remediation_playbooks
        ORDER BY playbook_key
        "#,
    )
    .fetch_all(pool)
    .await
}

pub struct CreateRuntimeVmRemediationPlaybook<'a> {
    pub playbook_key: &'a str,
    pub display_name: &'a str,
    pub description: Option<&'a str>,
    pub executor_type: &'a str,
    pub owner_id: i32,
    pub approval_required: bool,
    pub sla_duration_seconds: Option<i32>,
    pub metadata: &'a Value,
}

pub async fn create_playbook<'c, E>(
    executor: E,
    input: CreateRuntimeVmRemediationPlaybook<'_>,
) -> Result<RuntimeVmRemediationPlaybook, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query_as::<_, RuntimeVmRemediationPlaybook>(
        r#"
        INSERT INTO runtime_vm_remediation_playbooks (
            playbook_key,
            display_name,
            description,
            executor_type,
            owner_id,
            approval_required,
            sla_duration_seconds,
            metadata
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING
            id,
            playbook_key,
            display_name,
            description,
            executor_type,
            owner_id,
            approval_required,
            sla_duration_seconds,
            metadata,
            created_at,
            updated_at,
            version
        "#,
    )
    .bind(input.playbook_key)
    .bind(input.display_name)
    .bind(input.description)
    .bind(input.executor_type)
    .bind(input.owner_id)
    .bind(input.approval_required)
    .bind(input.sla_duration_seconds)
    .bind(input.metadata)
    .fetch_one(executor)
    .await
}

pub async fn delete_playbook<'c, E>(executor: E, playbook_id: i64) -> Result<bool, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let deleted = sqlx::query_scalar::<_, i64>(
        r#"
        DELETE FROM runtime_vm_remediation_playbooks
        WHERE id = $1
        RETURNING id
        "#,
    )
    .bind(playbook_id)
    .fetch_optional(executor)
    .await?;

    Ok(deleted.is_some())
}

pub async fn get_by_key(
    pool: &PgPool,
    key: &str,
) -> Result<Option<RuntimeVmRemediationPlaybook>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmRemediationPlaybook>(
        r#"
        SELECT
            id,
            playbook_key,
            display_name,
            description,
            executor_type,
            owner_id,
            approval_required,
            sla_duration_seconds,
            metadata,
            created_at,
            updated_at,
            version
        FROM runtime_vm_remediation_playbooks
        WHERE playbook_key = $1
        "#,
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

pub async fn get_by_id(
    pool: &PgPool,
    playbook_id: i64,
) -> Result<Option<RuntimeVmRemediationPlaybook>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmRemediationPlaybook>(
        r#"
        SELECT
            id,
            playbook_key,
            display_name,
            description,
            executor_type,
            owner_id,
            approval_required,
            sla_duration_seconds,
            metadata,
            created_at,
            updated_at,
            version
        FROM runtime_vm_remediation_playbooks
        WHERE id = $1
        "#,
    )
    .bind(playbook_id)
    .fetch_optional(pool)
    .await
}

pub struct UpdateRuntimeVmRemediationPlaybook<'a> {
    pub display_name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub executor_type: Option<&'a str>,
    pub owner_id: Option<i32>,
    pub approval_required: Option<bool>,
    pub sla_duration_seconds: Option<Option<i32>>,
    pub metadata: Option<&'a Value>,
    pub expected_version: i64,
}

pub async fn update_playbook<'c, E>(
    executor: E,
    playbook_id: i64,
    update: UpdateRuntimeVmRemediationPlaybook<'_>,
) -> Result<Option<RuntimeVmRemediationPlaybook>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    let should_update_sla = update.sla_duration_seconds.is_some();
    let sla_value = update.sla_duration_seconds.flatten();
    let record = sqlx::query_as::<_, RuntimeVmRemediationPlaybook>(
        r#"
        UPDATE runtime_vm_remediation_playbooks
        SET
            display_name = COALESCE($3, display_name),
            description = CASE WHEN $4 IS NULL THEN description ELSE $4 END,
            executor_type = COALESCE($5, executor_type),
            owner_id = COALESCE($6, owner_id),
            approval_required = COALESCE($7, approval_required),
            sla_duration_seconds = CASE
                WHEN $8 THEN $9
                ELSE sla_duration_seconds
            END,
            metadata = COALESCE($10, metadata),
            version = version + 1
        WHERE id = $1
          AND version = $2
        RETURNING
            id,
            playbook_key,
            display_name,
            description,
            executor_type,
            owner_id,
            approval_required,
            sla_duration_seconds,
            metadata,
            created_at,
            updated_at,
            version
        "#,
    )
    .bind(playbook_id)
    .bind(update.expected_version)
    .bind(update.display_name)
    .bind(update.description)
    .bind(update.executor_type)
    .bind(update.owner_id)
    .bind(update.approval_required)
    .bind(should_update_sla)
    .bind(sla_value)
    .bind(update.metadata)
    .fetch_optional(executor)
    .await?;

    Ok(record)
}
