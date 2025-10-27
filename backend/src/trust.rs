use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{postgres::PgListener, PgPool, Row};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, warn};

use crate::{
    evaluations::scheduler::{self, TrustTransitionSignal},
    job_queue::{self, Job},
};

const TRUST_CHANNEL: &str = "runtime_vm_trust_transition";

#[derive(Debug, Deserialize)]
struct TrustNotification {
    runtime_vm_instance_id: i64,
    attestation_id: Option<i64>,
    previous_status: Option<String>,
    current_status: String,
    previous_lifecycle_state: Option<String>,
    current_lifecycle_state: String,
    transition_reason: Option<String>,
    remediation_state: Option<String>,
    remediation_attempts: Option<i32>,
    freshness_deadline: Option<chrono::DateTime<Utc>>,
    provenance_ref: Option<String>,
    provenance: Option<Value>,
    triggered_at: chrono::DateTime<Utc>,
}

pub fn spawn_trust_listener(pool: PgPool, job_tx: Sender<Job>) {
    tokio::spawn(async move {
        if let Err(err) = listen(pool, job_tx).await {
            error!(?err, "trust transition listener terminated");
        }
    });
}

async fn listen(pool: PgPool, job_tx: Sender<Job>) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(TRUST_CHANNEL).await?;

    loop {
        let notification = listener.recv().await?;
        let payload = notification.payload();
        match serde_json::from_str::<TrustNotification>(payload) {
            Ok(message) => {
                debug!(?message, "received trust transition notification");
                let instance_row =
                    sqlx::query("SELECT server_id FROM runtime_vm_instances WHERE id = $1")
                        .bind(message.runtime_vm_instance_id)
                        .fetch_optional(&pool)
                        .await?;

                let Some(instance_row) = instance_row else {
                    warn!(
                        vm_instance_id = message.runtime_vm_instance_id,
                        "ignoring trust notification for missing runtime VM instance"
                    );
                    continue;
                };

                let server_id: i32 = instance_row.get("server_id");
                let signal = TrustTransitionSignal {
                    server_id,
                    vm_instance_id: message.runtime_vm_instance_id,
                    current_status: message.current_status.clone(),
                    previous_status: message.previous_status.clone(),
                    lifecycle_state: message.current_lifecycle_state.clone(),
                    previous_lifecycle_state: message.previous_lifecycle_state.clone(),
                    transition_reason: message.transition_reason.clone(),
                    remediation_state: message.remediation_state.clone(),
                    triggered_at: message.triggered_at,
                    freshness_expires_at: message.freshness_deadline,
                    remediation_attempts: message.remediation_attempts.unwrap_or_default(),
                    provenance_ref: message.provenance_ref.clone(),
                    provenance: message.provenance.clone(),
                    posture_changed: message
                        .previous_status
                        .as_deref()
                        .map(|status| status != message.current_status)
                        .unwrap_or(true),
                };

                if let Err(err) = scheduler::handle_trust_transition(&pool, &job_tx, &signal).await
                {
                    warn!(
                        ?err,
                        server_id = signal.server_id,
                        vm_instance_id = signal.vm_instance_id,
                        "failed to apply trust transition"
                    );
                }

                job_queue::enqueue_intelligence_refresh(&pool, signal.server_id).await;
            }
            Err(err) => warn!(?err, payload, "failed to parse trust notification payload"),
        }
    }
}
