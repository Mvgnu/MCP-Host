use crate::runtime::ContainerRuntime;
use crate::{evaluations, intelligence};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::time::{sleep, Duration};

#[derive(Debug, Serialize, Deserialize)]
pub enum Job {
    Start {
        server_id: i32,
        server_type: String,
        config: Option<Value>,
        api_key: String,
        use_gpu: bool,
    },
    Stop {
        server_id: i32,
    },
    Delete {
        server_id: i32,
    },
    IntelligenceRefresh {
        server_id: i32,
    },
    EvaluationRefresh {
        certification_id: i32,
    },
}

pub async fn enqueue_job(pool: &PgPool, job: &Job) {
    if let Ok(payload) = serde_json::to_value(job) {
        let _ = sqlx::query("INSERT INTO job_queue (payload) VALUES ($1)")
            .bind(payload)
            .execute(pool)
            .await;
    }
}

pub async fn enqueue_intelligence_refresh(pool: &PgPool, server_id: i32) {
    let job = Job::IntelligenceRefresh { server_id };
    enqueue_job(pool, &job).await;
}

pub fn start_worker(pool: PgPool, runtime: Arc<dyn ContainerRuntime>) -> Sender<Job> {
    let (tx, mut rx): (Sender<Job>, Receiver<Job>) = channel(32);

    // Load queued jobs from the database on startup
    let db_pool = pool.clone();
    let replay_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let rows = sqlx::query(
                "SELECT id, payload FROM job_queue WHERE status = 'queued' ORDER BY id",
            )
            .fetch_all(&db_pool)
            .await
            .unwrap_or_default();
            for row in rows {
                let id: i32 = row.get("id");
                let payload: Value = row.get("payload");
                if let Ok(job) = serde_json::from_value::<Job>(payload) {
                    let _ = sqlx::query("UPDATE job_queue SET status = 'processing' WHERE id = $1")
                        .bind(id)
                        .execute(&db_pool)
                        .await;
                    let _ = replay_tx.send(job).await;
                    let _ = sqlx::query("DELETE FROM job_queue WHERE id = $1")
                        .bind(id)
                        .execute(&db_pool)
                        .await;
                }
            }
            sleep(Duration::from_secs(5)).await;
        }
    });

    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            match job {
                Job::Start {
                    server_id,
                    server_type,
                    config,
                    api_key,
                    use_gpu,
                } => {
                    let rt = runtime.clone();
                    rt.spawn_server_task(
                        server_id,
                        server_type,
                        config,
                        api_key,
                        use_gpu,
                        pool.clone(),
                    );
                }
                Job::Stop { server_id } => {
                    let rt = runtime.clone();
                    rt.stop_server_task(server_id, pool.clone());
                }
                Job::Delete { server_id } => {
                    let rt = runtime.clone();
                    rt.delete_server_task(server_id, pool.clone());
                }
                Job::IntelligenceRefresh { server_id } => {
                    let db = pool.clone();
                    tokio::spawn(async move {
                        if let Err(err) = intelligence::recompute_from_history(&db, server_id).await
                        {
                            tracing::warn!(
                                ?err,
                                %server_id,
                                "intelligence recompute job failed",
                            );
                        } else {
                            tracing::info!(
                                %server_id,
                                "intelligence recompute job completed",
                            );
                        }
                    });
                }
                Job::EvaluationRefresh { certification_id } => {
                    let db = pool.clone();
                    tokio::spawn(async move {
                        match evaluations::retry_certification(&db, certification_id).await {
                            Ok(Some(_)) => {
                                tracing::info!(
                                    %certification_id,
                                    "evaluation certification marked for refresh",
                                );
                            }
                            Ok(None) => {
                                tracing::warn!(
                                    %certification_id,
                                    "evaluation refresh job referenced missing certification",
                                );
                            }
                            Err(err) => {
                                tracing::warn!(
                                    ?err,
                                    %certification_id,
                                    "evaluation refresh job failed",
                                );
                            }
                        }
                    });
                }
            }
        }
    });
    tx
}
