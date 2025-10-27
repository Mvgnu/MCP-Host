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

#[derive(Debug, Clone)]
pub struct TrustTransitionSignal {
    pub server_id: i32,
    pub vm_instance_id: i64,
    pub current_status: String,
    pub previous_status: Option<String>,
    pub transition_reason: Option<String>,
    pub remediation_state: Option<String>,
    pub triggered_at: DateTime<Utc>,
    pub freshness_expires_at: Option<DateTime<Utc>>,
    pub posture_changed: bool,
}

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

pub async fn handle_trust_transition(
    pool: &PgPool,
    job_tx: &Sender<Job>,
    signal: &TrustTransitionSignal,
) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            ec.id,
            ec.remediation_attempts,
            ec.fallback_launched_at,
            ec.status
        FROM evaluation_certifications ec
        JOIN build_artifact_runs bar ON ec.build_artifact_run_id = bar.id
        WHERE bar.server_id = $1
        "#,
    )
    .bind(signal.server_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    match signal.current_status.as_str() {
        "trusted" if signal.posture_changed => {
            for row in rows {
                let certification_id: i32 = row.get("id");
                let status: String = row.get("status");
                if status == "pending" {
                    continue;
                }
                schedule_refresh(pool, job_tx, certification_id).await?;
            }
        }
        "untrusted" | "unknown" => {
            let mut certification_ids = Vec::with_capacity(rows.len());
            for row in &rows {
                let certification_id: i32 = row.get("id");
                let attempts: i32 = row.try_get("remediation_attempts").unwrap_or(0);
                let fallback_launched_at: Option<DateTime<Utc>> =
                    row.try_get("fallback_launched_at").unwrap_or(None);
                record_trust_block(pool, certification_id, attempts, fallback_launched_at).await?;
                certification_ids.push(certification_id);
            }

            clear_pending_refresh_jobs(pool, &certification_ids).await?;
        }
        _ => {}
    }

    Ok(())
}

async fn scan_and_schedule(pool: &PgPool, job_tx: &Sender<Job>) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, last_attestation_status, fallback_launched_at, remediation_attempts
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
        let last_attestation_status: Option<String> =
            row.try_get("last_attestation_status").unwrap_or(None);
        if matches!(last_attestation_status.as_deref(), Some("untrusted")) {
            let fallback_launched_at: Option<DateTime<Utc>> =
                row.try_get("fallback_launched_at").unwrap_or(None);
            record_trust_block(
                pool,
                certification_id,
                row.try_get("remediation_attempts").unwrap_or(0),
                fallback_launched_at,
            )
            .await?;
            continue;
        }
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

async fn clear_pending_refresh_jobs(
    pool: &PgPool,
    certification_ids: &[i32],
) -> Result<(), sqlx::Error> {
    if certification_ids.is_empty() {
        return Ok(());
    }

    sqlx::query(
        r#"
        DELETE FROM job_queue
        WHERE payload ? 'EvaluationRefresh'
          AND (payload -> 'EvaluationRefresh' ->> 'certification_id')::int = ANY($1)
        "#,
    )
    .bind(certification_ids)
    .execute(pool)
    .await?;

    Ok(())
}

pub(crate) async fn record_trust_block(
    pool: &PgPool,
    certification_id: i32,
    attempts: i32,
    fallback_launched_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    let note = format!(
        "{} trust block maintained after {} remediation attempts",
        Utc::now().to_rfc3339(),
        attempts
    );
    sqlx::query(
        r#"
        UPDATE evaluation_certifications
        SET
            governance_notes = CASE
                WHEN governance_notes IS NULL OR governance_notes = '' THEN $2
                ELSE governance_notes || E'\n' || $2
            END,
            fallback_launched_at = COALESCE(fallback_launched_at, $3),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(certification_id)
    .bind(&note)
    .bind(fallback_launched_at.unwrap_or_else(Utc::now))
    .execute(pool)
    .await?;

    warn!(
        %certification_id,
        "skipping refresh because associated VM trust posture is untrusted"
    );
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

#[cfg(test)]
mod tests {
    use super::{handle_trust_transition, TrustTransitionSignal};
    use crate::job_queue::Job;
    use chrono::{DateTime, Duration, Utc};
    use sqlx::PgPool;
    use tokio::sync::mpsc::channel;

    async fn seed_certification(pool: &PgPool) -> (i32, i32) {
        let user_id: i32 = sqlx::query_scalar(
            "INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id",
        )
        .bind("trust@example.com")
        .bind("hash")
        .fetch_one(pool)
        .await
        .expect("user");

        let server_id: i32 = sqlx::query_scalar(
            "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key) VALUES ($1, 'vm', 'virtual-machine', '{}'::jsonb, 'active', 'key') RETURNING id",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("server");

        let run_id: i32 = sqlx::query_scalar(
            "INSERT INTO build_artifact_runs (server_id, local_image, started_at, completed_at, status, credential_health_status) VALUES ($1, 'image', NOW(), NOW(), 'completed', 'healthy') RETURNING id",
        )
        .bind(server_id)
        .fetch_one(pool)
        .await
        .expect("run");

        let certification_id: i32 = sqlx::query_scalar(
            "INSERT INTO evaluation_certifications (build_artifact_run_id, manifest_digest, tier, policy_requirement, status, refresh_cadence_seconds, next_refresh_at, governance_notes, last_attestation_status, remediation_attempts) VALUES ($1, 'sha256:abc', 'confidential', 'runtime', 'pass', 3600, NOW(), NULL, 'trusted', 1) RETURNING id",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("certification");

        (server_id, certification_id)
    }

    #[sqlx::test]
    #[ignore = "requires DATABASE_URL with Postgres server"]
    async fn trust_transition_blocks_refresh(pool: PgPool) {
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let (server_id, certification_id) = seed_certification(&pool).await;

        sqlx::query("INSERT INTO job_queue (payload) VALUES ($1)")
            .bind(serde_json::json!({
                "EvaluationRefresh": {"certification_id": certification_id}
            }))
            .execute(&pool)
            .await
            .expect("seed job");

        let (tx, mut rx) = channel::<Job>(1);
        let signal = TrustTransitionSignal {
            server_id,
            vm_instance_id: 42,
            current_status: "untrusted".to_string(),
            previous_status: Some("trusted".to_string()),
            transition_reason: Some("attestation".to_string()),
            remediation_state: Some("remediation:investigate".to_string()),
            triggered_at: Utc::now(),
            freshness_expires_at: None,
            posture_changed: true,
        };

        handle_trust_transition(&pool, &tx, &signal)
            .await
            .expect("handle transition");

        assert!(
            rx.try_recv().is_err(),
            "no refresh jobs should be dispatched"
        );

        let (notes, fallback_exists, attempts, queued_jobs): (Option<String>, bool, i32, i64) = sqlx::query_as(
            "SELECT governance_notes, fallback_launched_at IS NOT NULL, remediation_attempts, (SELECT COUNT(*) FROM job_queue) FROM evaluation_certifications WHERE id = $1",
        )
        .bind(certification_id)
        .fetch_one(&pool)
        .await
        .expect("fetch certification");

        assert!(fallback_exists, "fallback timestamp should be recorded");
        assert!(notes.unwrap_or_default().contains("trust block maintained"));
        assert!(attempts >= 1);
        assert_eq!(queued_jobs, 0);
    }

    #[sqlx::test]
    #[ignore = "requires DATABASE_URL with Postgres server"]
    async fn trust_transition_reschedules_after_recovery(pool: PgPool) {
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let (server_id, certification_id) = seed_certification(&pool).await;

        sqlx::query("UPDATE evaluation_certifications SET status = 'fail', last_attestation_status = 'untrusted' WHERE id = $1")
            .bind(certification_id)
            .execute(&pool)
            .await
            .expect("set fail");

        let (tx, mut rx) = channel::<Job>(4);
        let signal = TrustTransitionSignal {
            server_id,
            vm_instance_id: 42,
            current_status: "trusted".to_string(),
            previous_status: Some("untrusted".to_string()),
            transition_reason: Some("attestation".to_string()),
            remediation_state: Some("remediation:none".to_string()),
            triggered_at: Utc::now(),
            freshness_expires_at: None,
            posture_changed: true,
        };

        handle_trust_transition(&pool, &tx, &signal)
            .await
            .expect("handle transition");

        let dispatched = rx.recv().await.expect("job dispatched");
        match dispatched {
            Job::EvaluationRefresh {
                certification_id: queued,
            } => {
                assert_eq!(queued, certification_id);
            }
            other => panic!("unexpected job {:?}", other),
        }

        let (notes, next_refresh_at): (Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT governance_notes, next_refresh_at FROM evaluation_certifications WHERE id = $1",
        )
        .bind(certification_id)
        .fetch_one(&pool)
        .await
        .expect("fetch certification");

        assert!(notes
            .unwrap_or_default()
            .contains("auto-scheduled evidence refresh"));
        assert!(next_refresh_at
            .expect("next refresh")
            .gt(&(Utc::now() - Duration::minutes(1))));
    }
}
