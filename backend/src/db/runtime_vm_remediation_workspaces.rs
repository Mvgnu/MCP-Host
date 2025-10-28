use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres};
use std::collections::HashMap;

// key: remediation-db -> workspace-lifecycle
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationWorkspace {
    pub id: i64,
    pub workspace_key: String,
    pub display_name: String,
    pub description: Option<String>,
    pub owner_id: i32,
    pub lifecycle_state: String,
    pub active_revision_id: Option<i64>,
    pub metadata: Value,
    pub lineage_tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationWorkspaceRevision {
    pub id: i64,
    pub workspace_id: i64,
    pub revision_number: i64,
    pub previous_revision_id: Option<i64>,
    pub created_by: i32,
    pub plan: Value,
    pub schema_status: String,
    pub schema_errors: Vec<String>,
    pub policy_status: String,
    pub policy_veto_reasons: Vec<String>,
    pub simulation_status: String,
    pub promotion_status: String,
    pub metadata: Value,
    pub lineage_labels: Vec<String>,
    pub schema_validated_at: Option<DateTime<Utc>>,
    pub policy_evaluated_at: Option<DateTime<Utc>>,
    pub simulated_at: Option<DateTime<Utc>>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationWorkspaceSandboxExecution {
    pub id: i64,
    pub workspace_revision_id: i64,
    pub simulator_kind: String,
    pub execution_state: String,
    pub requested_by: i32,
    pub gate_context: Value,
    pub diff_snapshot: Option<Value>,
    pub metadata: Value,
    pub requested_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RuntimeVmRemediationWorkspaceValidationSnapshot {
    pub id: i64,
    pub workspace_revision_id: i64,
    pub snapshot_type: String,
    pub status: String,
    pub gate_context: Value,
    pub notes: Vec<String>,
    pub recorded_at: DateTime<Utc>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRevisionDetails {
    pub revision: RuntimeVmRemediationWorkspaceRevision,
    pub sandbox_executions: Vec<RuntimeVmRemediationWorkspaceSandboxExecution>,
    pub validation_snapshots: Vec<RuntimeVmRemediationWorkspaceValidationSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceDetails {
    pub workspace: RuntimeVmRemediationWorkspace,
    pub revisions: Vec<WorkspaceRevisionDetails>,
}

#[derive(Debug, Clone)]
pub struct CreateWorkspace<'a> {
    pub workspace_key: &'a str,
    pub display_name: &'a str,
    pub description: Option<&'a str>,
    pub owner_id: i32,
    pub plan: &'a Value,
    pub metadata: Option<&'a Value>,
    pub lineage_tags: &'a [&'a str],
    pub lineage_labels: &'a [&'a str],
}

#[derive(Debug, Clone)]
pub struct CreateWorkspaceRevision<'a> {
    pub workspace_id: i64,
    pub previous_revision_id: Option<i64>,
    pub created_by: i32,
    pub plan: &'a Value,
    pub metadata: Option<&'a Value>,
    pub lineage_labels: &'a [&'a str],
    pub expected_workspace_version: i64,
}

#[derive(Debug, Clone)]
pub struct SchemaValidationUpdate<'a> {
    pub workspace_id: i64,
    pub revision_id: i64,
    pub validator_id: i32,
    pub result_status: &'a str,
    pub errors: &'a [&'a str],
    pub gate_context: &'a Value,
    pub metadata: Option<&'a Value>,
    pub expected_revision_version: i64,
}

#[derive(Debug, Clone)]
pub struct PolicyFeedbackUpdate<'a> {
    pub workspace_id: i64,
    pub revision_id: i64,
    pub reviewer_id: i32,
    pub policy_status: &'a str,
    pub veto_reasons: &'a [&'a str],
    pub gate_context: &'a Value,
    pub metadata: Option<&'a Value>,
    pub expected_revision_version: i64,
}

#[derive(Debug, Clone)]
pub struct SandboxSimulationUpdate<'a> {
    pub workspace_id: i64,
    pub revision_id: i64,
    pub simulator_kind: &'a str,
    pub requested_by: i32,
    pub execution_state: &'a str,
    pub gate_context: &'a Value,
    pub diff_snapshot: Option<&'a Value>,
    pub metadata: Option<&'a Value>,
    pub expected_revision_version: i64,
}

#[derive(Debug, Clone)]
pub struct PromotionUpdate<'a> {
    pub workspace_id: i64,
    pub revision_id: i64,
    pub requested_by: i32,
    pub promotion_status: &'a str,
    pub notes: &'a [&'a str],
    pub expected_workspace_version: i64,
    pub expected_revision_version: i64,
}

pub async fn create_workspace(
    pool: &PgPool,
    params: CreateWorkspace<'_>,
) -> Result<WorkspaceDetails, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let workspace = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        INSERT INTO runtime_vm_remediation_workspaces (
            workspace_key,
            display_name,
            description,
            owner_id,
            lifecycle_state,
            metadata,
            lineage_tags
        )
        VALUES ($1, $2, $3, $4, 'draft', COALESCE($5, '{}'::JSONB), $6)
        RETURNING id, workspace_key, display_name, description, owner_id, lifecycle_state,
                  active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        "#,
    )
    .bind(params.workspace_key)
    .bind(params.display_name)
    .bind(params.description)
    .bind(params.owner_id)
    .bind(params.metadata)
    .bind(params.lineage_tags)
    .fetch_one(&mut *tx)
    .await?;

    let next_number = 1_i64;

    let revision = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        INSERT INTO runtime_vm_remediation_workspace_revisions (
            workspace_id,
            revision_number,
            previous_revision_id,
            created_by,
            plan,
            metadata,
            lineage_labels
        )
        VALUES ($1, $2, NULL, $3, $4, COALESCE($5, '{}'::JSONB), $6)
        RETURNING id, workspace_id, revision_number, previous_revision_id, created_by, plan,
                  schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
                  promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
                  simulated_at, promoted_at, created_at, updated_at, version
        "#,
    )
    .bind(workspace.id)
    .bind(next_number)
    .bind(params.owner_id)
    .bind(params.plan)
    .bind(params.metadata)
    .bind(params.lineage_labels)
    .fetch_one(&mut *tx)
    .await?;

    let updated_workspace = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        UPDATE runtime_vm_remediation_workspaces
        SET active_revision_id = $2,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1
        RETURNING id, workspace_key, display_name, description, owner_id, lifecycle_state,
                  active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        "#,
    )
    .bind(workspace.id)
    .bind(revision.id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    load_workspace_details(pool, updated_workspace.id)
        .await?
        .ok_or_else(|| sqlx::Error::RowNotFound)
}

pub async fn list_workspaces(
    pool: &PgPool,
) -> Result<Vec<RuntimeVmRemediationWorkspace>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        SELECT id, workspace_key, display_name, description, owner_id, lifecycle_state,
               active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        FROM runtime_vm_remediation_workspaces
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
}

pub async fn list_workspace_details(pool: &PgPool) -> Result<Vec<WorkspaceDetails>, sqlx::Error> {
    let workspaces = list_workspaces(pool).await?;
    let mut details = Vec::with_capacity(workspaces.len());
    for workspace in workspaces {
        if let Some(view) = load_workspace_details(pool, workspace.id).await? {
            details.push(view);
        }
    }
    Ok(details)
}

pub async fn get_workspace(
    pool: &PgPool,
    workspace_id: i64,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    load_workspace_details(pool, workspace_id).await
}

pub async fn get_workspace_by_key(
    pool: &PgPool,
    workspace_key: &str,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let workspace = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        SELECT id, workspace_key, display_name, description, owner_id, lifecycle_state,
               active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        FROM runtime_vm_remediation_workspaces
        WHERE workspace_key = $1
        "#,
    )
    .bind(workspace_key)
    .fetch_optional(pool)
    .await?;

    if let Some(workspace) = workspace {
        let details = load_workspace_details(pool, workspace.id).await?;
        return Ok(details);
    }

    Ok(None)
}

pub async fn create_revision(
    pool: &PgPool,
    params: CreateWorkspaceRevision<'_>,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let current = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        SELECT id, workspace_key, display_name, description, owner_id, lifecycle_state,
               active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        FROM runtime_vm_remediation_workspaces
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(params.workspace_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(current_workspace) = current else {
        tx.rollback().await?;
        return Ok(None);
    };

    if current_workspace.version != params.expected_workspace_version {
        tx.rollback().await?;
        return Ok(None);
    }

    let next_number: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(revision_number), 0) + 1 as next_number \
         FROM runtime_vm_remediation_workspace_revisions \
         WHERE workspace_id = $1",
    )
    .bind(params.workspace_id)
    .fetch_one(&mut *tx)
    .await?;

    let revision = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        INSERT INTO runtime_vm_remediation_workspace_revisions (
            workspace_id,
            revision_number,
            previous_revision_id,
            created_by,
            plan,
            metadata,
            lineage_labels
        )
        VALUES ($1, $2, $3, $4, $5, COALESCE($6, '{}'::JSONB), $7)
        RETURNING id, workspace_id, revision_number, previous_revision_id, created_by, plan,
                  schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
                  promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
                  simulated_at, promoted_at, created_at, updated_at, version
        "#,
    )
    .bind(params.workspace_id)
    .bind(next_number)
    .bind(params.previous_revision_id)
    .bind(params.created_by)
    .bind(params.plan)
    .bind(params.metadata)
    .bind(params.lineage_labels)
    .fetch_one(&mut *tx)
    .await?;

    let updated = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        UPDATE runtime_vm_remediation_workspaces
        SET active_revision_id = $2,
            lifecycle_state = 'draft',
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1 AND version = $3
        RETURNING id, workspace_key, display_name, description, owner_id, lifecycle_state,
                  active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        "#,
    )
    .bind(params.workspace_id)
    .bind(revision.id)
    .bind(params.expected_workspace_version)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(_workspace) = updated else {
        tx.rollback().await?;
        return Ok(None);
    };

    tx.commit().await?;
    load_workspace_details(pool, params.workspace_id).await
}

pub async fn apply_schema_validation(
    pool: &PgPool,
    params: SchemaValidationUpdate<'_>,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let revision = lock_revision(&mut tx, params.workspace_id, params.revision_id).await?;
    let Some(revision) = revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    if revision.version != params.expected_revision_version {
        tx.rollback().await?;
        return Ok(None);
    }

    let result_status = params.result_status;
    let errors: Vec<String> = params.errors.iter().map(|s| s.to_string()).collect();
    let notes = vec![format!("validator_id={}", params.validator_id)];

    let updated_revision = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        UPDATE runtime_vm_remediation_workspace_revisions
        SET schema_status = $3,
            schema_errors = $4,
            schema_validated_at = NOW(),
            metadata = CASE WHEN $5 IS NULL THEN metadata ELSE $5 END,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1 AND workspace_id = $2 AND version = $6
        RETURNING id, workspace_id, revision_number, previous_revision_id, created_by, plan,
                  schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
                  promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
                  simulated_at, promoted_at, created_at, updated_at, version
        "#,
    )
    .bind(params.revision_id)
    .bind(params.workspace_id)
    .bind(result_status)
    .bind(&errors)
    .bind(params.metadata)
    .bind(params.expected_revision_version)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(_rev) = updated_revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    sqlx::query(
        "INSERT INTO runtime_vm_remediation_workspace_validation_snapshots (\
            workspace_revision_id,\
            snapshot_type,\
            status,\
            gate_context,\
            notes,\
            metadata\
        ) VALUES ($1, 'schema', $2, $3, $4, COALESCE($5, '{}'::JSONB))",
    )
    .bind(params.revision_id)
    .bind(result_status)
    .bind(params.gate_context)
    .bind(&notes)
    .bind(params.metadata)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    load_workspace_details(pool, params.workspace_id).await
}

pub async fn apply_policy_feedback(
    pool: &PgPool,
    params: PolicyFeedbackUpdate<'_>,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let revision = lock_revision(&mut tx, params.workspace_id, params.revision_id).await?;
    let Some(revision) = revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    if revision.version != params.expected_revision_version {
        tx.rollback().await?;
        return Ok(None);
    }

    let veto_reasons: Vec<String> = params.veto_reasons.iter().map(|s| s.to_string()).collect();
    let mut notes = Vec::with_capacity(veto_reasons.len() + 1);
    notes.push(format!("reviewer_id={}", params.reviewer_id));
    notes.extend(veto_reasons.clone());

    let updated_revision = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        UPDATE runtime_vm_remediation_workspace_revisions
        SET policy_status = $3,
            policy_veto_reasons = $4,
            policy_evaluated_at = NOW(),
            metadata = CASE WHEN $5 IS NULL THEN metadata ELSE $5 END,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1 AND workspace_id = $2 AND version = $6
        RETURNING id, workspace_id, revision_number, previous_revision_id, created_by, plan,
                  schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
                  promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
                  simulated_at, promoted_at, created_at, updated_at, version
        "#,
    )
    .bind(params.revision_id)
    .bind(params.workspace_id)
    .bind(params.policy_status)
    .bind(&veto_reasons)
    .bind(params.metadata)
    .bind(params.expected_revision_version)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(_rev) = updated_revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    sqlx::query(
        "INSERT INTO runtime_vm_remediation_workspace_validation_snapshots (\
            workspace_revision_id,\
            snapshot_type,\
            status,\
            gate_context,\
            notes,\
            metadata\
        ) VALUES ($1, 'policy', $2, $3, $4, COALESCE($5, '{}'::JSONB))",
    )
    .bind(params.revision_id)
    .bind(&params.policy_status)
    .bind(params.gate_context)
    .bind(&notes)
    .bind(params.metadata)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    load_workspace_details(pool, params.workspace_id).await
}

pub async fn apply_sandbox_simulation(
    pool: &PgPool,
    params: SandboxSimulationUpdate<'_>,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let revision = lock_revision(&mut tx, params.workspace_id, params.revision_id).await?;
    let Some(revision) = revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    if revision.version != params.expected_revision_version {
        tx.rollback().await?;
        return Ok(None);
    }

    let simulation_status = params.execution_state;

    let updated_revision = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        UPDATE runtime_vm_remediation_workspace_revisions
        SET simulation_status = $3,
            simulated_at = CASE WHEN $3 IN ('succeeded', 'failed') THEN NOW() ELSE simulated_at END,
            metadata = CASE WHEN $5 IS NULL THEN metadata ELSE $5 END,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1 AND workspace_id = $2 AND version = $4
        RETURNING id, workspace_id, revision_number, previous_revision_id, created_by, plan,
                  schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
                  promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
                  simulated_at, promoted_at, created_at, updated_at, version
        "#,
    )
    .bind(params.revision_id)
    .bind(params.workspace_id)
    .bind(simulation_status)
    .bind(params.expected_revision_version)
    .bind(params.metadata)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(_rev) = updated_revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    sqlx::query(
        "INSERT INTO runtime_vm_remediation_workspace_sandbox_executions (\
            workspace_revision_id,\
            simulator_kind,\
            execution_state,\
            requested_by,\
            gate_context,\
            diff_snapshot,\
            metadata\
        ) VALUES ($1, $2, $3, $4, $5, $6, COALESCE($7, '{}'::JSONB))",
    )
    .bind(params.revision_id)
    .bind(&params.simulator_kind)
    .bind(&params.execution_state)
    .bind(params.requested_by)
    .bind(params.gate_context)
    .bind(params.diff_snapshot)
    .bind(params.metadata)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    load_workspace_details(pool, params.workspace_id).await
}

pub async fn apply_promotion(
    pool: &PgPool,
    params: PromotionUpdate<'_>,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let revision = lock_revision(&mut tx, params.workspace_id, params.revision_id).await?;
    let Some(revision) = revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    if revision.version != params.expected_revision_version {
        tx.rollback().await?;
        return Ok(None);
    }

    let notes: Vec<String> = params.notes.iter().map(|s| s.to_string()).collect();
    let mut snapshot_notes = Vec::with_capacity(notes.len() + 1);
    snapshot_notes.push(format!("requested_by={}", params.requested_by));
    snapshot_notes.extend(notes.clone());

    let updated_revision = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        UPDATE runtime_vm_remediation_workspace_revisions
        SET promotion_status = $3,
            promoted_at = CASE WHEN $3 = 'completed' THEN NOW() ELSE promoted_at END,
            metadata = metadata,
            version = version + 1,
            updated_at = NOW()
        WHERE id = $1 AND workspace_id = $2 AND version = $4
        RETURNING id, workspace_id, revision_number, previous_revision_id, created_by, plan,
                  schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
                  promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
                  simulated_at, promoted_at, created_at, updated_at, version
        "#,
    )
    .bind(params.revision_id)
    .bind(params.workspace_id)
    .bind(params.promotion_status)
    .bind(params.expected_revision_version)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(updated_revision) = updated_revision else {
        tx.rollback().await?;
        return Ok(None);
    };

    if params.promotion_status == "completed" {
        let updated_workspace = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
            r#"
            UPDATE runtime_vm_remediation_workspaces
            SET active_revision_id = $2,
                lifecycle_state = 'promoted',
                version = version + 1,
                updated_at = NOW()
            WHERE id = $1 AND version = $3
            RETURNING id, workspace_key, display_name, description, owner_id, lifecycle_state,
                      active_revision_id, metadata, lineage_tags, created_at, updated_at, version
            "#,
        )
        .bind(params.workspace_id)
        .bind(updated_revision.id)
        .bind(params.expected_workspace_version)
        .fetch_optional(&mut *tx)
        .await?;

        if updated_workspace.is_none() {
            tx.rollback().await?;
            return Ok(None);
        }
    }

    sqlx::query(
        "INSERT INTO runtime_vm_remediation_workspace_validation_snapshots (\
            workspace_revision_id,\
            snapshot_type,\
            status,\
            gate_context,\
            notes,\
            metadata\
        ) VALUES ($1, 'promotion', $2, '{}'::JSONB, $3, '{}'::JSONB)",
    )
    .bind(params.revision_id)
    .bind(&params.promotion_status)
    .bind(&snapshot_notes)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    load_workspace_details(pool, params.workspace_id).await
}

async fn load_workspace_details(
    pool: &PgPool,
    workspace_id: i64,
) -> Result<Option<WorkspaceDetails>, sqlx::Error> {
    let workspace = sqlx::query_as::<_, RuntimeVmRemediationWorkspace>(
        r#"
        SELECT id, workspace_key, display_name, description, owner_id, lifecycle_state,
               active_revision_id, metadata, lineage_tags, created_at, updated_at, version
        FROM runtime_vm_remediation_workspaces
        WHERE id = $1
        "#,
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;

    let Some(workspace) = workspace else {
        return Ok(None);
    };

    let revisions = sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        SELECT id, workspace_id, revision_number, previous_revision_id, created_by, plan,
               schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
               promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
               simulated_at, promoted_at, created_at, updated_at, version
        FROM runtime_vm_remediation_workspace_revisions
        WHERE workspace_id = $1
        ORDER BY revision_number DESC
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    let revision_ids: Vec<i64> = revisions.iter().map(|r| r.id).collect();

    let sandboxes = if revision_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, RuntimeVmRemediationWorkspaceSandboxExecution>(
            r#"
            SELECT id, workspace_revision_id, simulator_kind, execution_state, requested_by,
                   gate_context, diff_snapshot, metadata, requested_at, started_at, completed_at,
                   failure_reason, created_at, updated_at, version
            FROM runtime_vm_remediation_workspace_sandbox_executions
            WHERE workspace_revision_id = ANY($1)
            ORDER BY requested_at DESC
            "#,
        )
        .bind(&revision_ids)
        .fetch_all(pool)
        .await?
    };

    let snapshots = if revision_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, RuntimeVmRemediationWorkspaceValidationSnapshot>(
            r#"
            SELECT id, workspace_revision_id, snapshot_type, status, gate_context, notes,
                   recorded_at, metadata, created_at, updated_at, version
            FROM runtime_vm_remediation_workspace_validation_snapshots
            WHERE workspace_revision_id = ANY($1)
            ORDER BY recorded_at DESC
            "#,
        )
        .bind(&revision_ids)
        .fetch_all(pool)
        .await?
    };

    let mut sandbox_map: HashMap<i64, Vec<RuntimeVmRemediationWorkspaceSandboxExecution>> =
        HashMap::new();
    for item in sandboxes {
        sandbox_map
            .entry(item.workspace_revision_id)
            .or_default()
            .push(item);
    }

    let mut snapshot_map: HashMap<i64, Vec<RuntimeVmRemediationWorkspaceValidationSnapshot>> =
        HashMap::new();
    for item in snapshots {
        snapshot_map
            .entry(item.workspace_revision_id)
            .or_default()
            .push(item);
    }

    let mut revision_details = Vec::with_capacity(revisions.len());
    for revision in revisions {
        let sandbox_executions = sandbox_map.remove(&revision.id).unwrap_or_default();
        let validation_snapshots = snapshot_map.remove(&revision.id).unwrap_or_default();
        revision_details.push(WorkspaceRevisionDetails {
            revision,
            sandbox_executions,
            validation_snapshots,
        });
    }

    Ok(Some(WorkspaceDetails {
        workspace,
        revisions: revision_details,
    }))
}

async fn lock_revision<'c, E>(
    executor: E,
    workspace_id: i64,
    revision_id: i64,
) -> Result<Option<RuntimeVmRemediationWorkspaceRevision>, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query_as::<_, RuntimeVmRemediationWorkspaceRevision>(
        r#"
        SELECT id, workspace_id, revision_number, previous_revision_id, created_by, plan,
               schema_status, schema_errors, policy_status, policy_veto_reasons, simulation_status,
               promotion_status, metadata, lineage_labels, schema_validated_at, policy_evaluated_at,
               simulated_at, promoted_at, created_at, updated_at, version
        FROM runtime_vm_remediation_workspace_revisions
        WHERE workspace_id = $1 AND id = $2
        FOR UPDATE
        "#,
    )
    .bind(workspace_id)
    .bind(revision_id)
    .fetch_optional(executor)
    .await
}
