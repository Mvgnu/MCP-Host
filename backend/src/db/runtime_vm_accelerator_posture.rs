use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

// key: remediation-db -> accelerator-posture
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RuntimeVmAcceleratorPostureRecord {
    pub id: i64,
    pub runtime_vm_instance_id: i64,
    pub accelerator_id: String,
    pub accelerator_type: String,
    pub posture: String,
    pub policy_feedback: Vec<String>,
    pub metadata: Value,
    pub collected_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAcceleratorPosture<'a> {
    pub runtime_vm_instance_id: i64,
    pub accelerator_id: &'a str,
    pub accelerator_type: &'a str,
    pub posture: &'a str,
    pub policy_feedback: &'a [String],
    pub metadata: &'a Value,
}

pub async fn replace_instance_posture(
    executor: &mut Transaction<'_, Postgres>,
    runtime_vm_instance_id: i64,
    entries: &[NewAcceleratorPosture<'_>],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM runtime_vm_accelerator_posture WHERE runtime_vm_instance_id = $1")
        .bind(runtime_vm_instance_id)
        .execute(&mut *executor)
        .await?;

    for entry in entries {
        sqlx::query(
            r#"
            INSERT INTO runtime_vm_accelerator_posture (
                runtime_vm_instance_id,
                accelerator_id,
                accelerator_type,
                posture,
                policy_feedback,
                metadata
            ) VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (runtime_vm_instance_id, accelerator_id) DO UPDATE
            SET
                accelerator_type = EXCLUDED.accelerator_type,
                posture = EXCLUDED.posture,
                policy_feedback = EXCLUDED.policy_feedback,
                metadata = EXCLUDED.metadata,
                updated_at = NOW()
            "#,
        )
        .bind(entry.runtime_vm_instance_id)
        .bind(entry.accelerator_id)
        .bind(entry.accelerator_type)
        .bind(entry.posture)
        .bind(entry.policy_feedback)
        .bind(entry.metadata)
        .execute(&mut *executor)
        .await?;
    }

    Ok(())
}

pub async fn list_for_instance(
    pool: &PgPool,
    runtime_vm_instance_id: i64,
) -> Result<Vec<RuntimeVmAcceleratorPostureRecord>, sqlx::Error> {
    sqlx::query_as::<_, RuntimeVmAcceleratorPostureRecord>(
        r#"
        SELECT
            id,
            runtime_vm_instance_id,
            accelerator_id,
            accelerator_type,
            posture,
            policy_feedback,
            metadata,
            collected_at,
            updated_at
        FROM runtime_vm_accelerator_posture
        WHERE runtime_vm_instance_id = $1
        ORDER BY accelerator_id
        "#,
    )
    .bind(runtime_vm_instance_id)
    .fetch_all(pool)
    .await
}

pub async fn list_for_instances(
    pool: &PgPool,
    instance_ids: &[i64],
) -> Result<Vec<RuntimeVmAcceleratorPostureRecord>, sqlx::Error> {
    if instance_ids.is_empty() {
        return Ok(Vec::new());
    }

    sqlx::query_as::<_, RuntimeVmAcceleratorPostureRecord>(
        r#"
        SELECT
            id,
            runtime_vm_instance_id,
            accelerator_id,
            accelerator_type,
            posture,
            policy_feedback,
            metadata,
            collected_at,
            updated_at
        FROM runtime_vm_accelerator_posture
        WHERE runtime_vm_instance_id = ANY($1)
        ORDER BY runtime_vm_instance_id, accelerator_id
        "#,
    )
    .bind(instance_ids)
    .fetch_all(pool)
    .await
}
