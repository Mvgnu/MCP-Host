use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use tokio::{sync::mpsc::Sender, time};
use tracing::{info, warn};

use crate::job_queue::{enqueue_job, Job};

const SCAN_INTERVAL_SECS: u64 = 60;
const LOOKAHEAD_MINUTES: i64 = 60;
const FALLBACK_MINUTES: i64 = 30;
const MAX_BATCH: i64 = 50;

// key: evaluation-scheduler -> periodic refresh coordination
pub fn spawn(pool: PgPool, job_tx: Sender<Job>) {
    tokio::spawn(async move {
        let mut ticker = time::interval(StdDuration::from_secs(SCAN_INTERVAL_SECS));
        loop {
            ticker.tick().await;
            if let Err(err) = scan_and_schedule(&pool, &job_tx).await {
                warn!(?err, "evaluation scheduler tick failed");
            }
        }
    });
}

async fn scan_and_schedule(pool: &PgPool, job_tx: &Sender<Job>) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id
        FROM evaluation_certifications
        WHERE next_refresh_at IS NOT NULL
          AND next_refresh_at <= NOW() + make_interval(mins => $1::double precision)
          AND status <> 'pending'
        ORDER BY next_refresh_at ASC
        LIMIT $2
        "#,
    )
    .bind(LOOKAHEAD_MINUTES)
    .bind(MAX_BATCH)
    .fetch_all(pool)
    .await?;

    for row in rows {
        let certification_id: i32 = row.get("id");
        schedule_refresh(pool, job_tx, certification_id).await?;
    }

    Ok(())
}

async fn schedule_refresh(
    pool: &PgPool,
    job_tx: &Sender<Job>,
    certification_id: i32,
) -> Result<(), sqlx::Error> {
    let job = Job::EvaluationRefresh { certification_id };
    enqueue_job(pool, &job).await;
    if let Err(err) = job_tx.send(job).await {
        warn!(?err, %certification_id, "failed to dispatch evaluation refresh job");
        return Ok(());
    }

    let note = format!(
        "{} auto-scheduled evidence refresh",
        Utc::now().to_rfc3339(),
    );
    sqlx::query(
        r#"
        UPDATE evaluation_certifications
        SET
            governance_notes = CASE
                WHEN governance_notes IS NULL OR governance_notes = '' THEN $2
                ELSE governance_notes || E'\n' || $2
            END,
            next_refresh_at = CASE
                WHEN refresh_cadence_seconds IS NOT NULL THEN NOW() + make_interval(secs => refresh_cadence_seconds::double precision)
                ELSE NOW() + make_interval(mins => $3::double precision)
            END,
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(certification_id)
    .bind(&note)
    .bind(FALLBACK_MINUTES)
    .execute(pool)
    .await?;

    info!(%certification_id, "queued evaluation refresh job");
    Ok(())
}

pub async fn record_anomaly(
    pool: &PgPool,
    certification_id: i32,
    details: &str,
) -> Result<(), sqlx::Error> {
    let note = format!("{} anomaly detected: {}", Utc::now().to_rfc3339(), details,);
    sqlx::query(
        r#"
        UPDATE evaluation_certifications
        SET
            governance_notes = CASE
                WHEN governance_notes IS NULL OR governance_notes = '' THEN $2
                ELSE governance_notes || E'\n' || $2
            END,
            next_refresh_at = NOW(),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(certification_id)
    .bind(&note)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn lineage(
    pool: &PgPool,
    certification_id: i32,
) -> Result<Option<(DateTime<Utc>, Option<DateTime<Utc>>, Option<Value>)>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT valid_from, valid_until, evidence_lineage
        FROM evaluation_certifications
        WHERE id = $1
        "#,
    )
    .bind(certification_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| {
        let valid_from: DateTime<Utc> = row.get("valid_from");
        let valid_until: Option<DateTime<Utc>> = row.try_get("valid_until").ok();
        let lineage: Option<Value> = row.try_get("evidence_lineage").unwrap_or(None);
        (valid_from, valid_until, lineage)
    }))
}
