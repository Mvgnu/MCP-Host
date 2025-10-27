use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::db::runtime_vm_remediation_runs::{
    ensure_running_playbook, mark_run_completed, mark_run_failed,
};
use crate::db::runtime_vm_trust_registry::{
    get_state as get_registry_state, upsert_state as upsert_registry_state,
    UpsertRuntimeVmTrustRegistryState,
};
use crate::trust::{subscribe_registry_events, TrustRegistryEvent};

const DEFAULT_PLAYBOOK: &str = "default-vm-remediation";

// key: remediation-orchestrator -> automation-loop
pub fn spawn(pool: PgPool) {
    tokio::spawn(async move {
        let mut receiver = subscribe_registry_events();
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event.lifecycle_state == "quarantined" {
                        debug!(
                            vm_instance_id = event.vm_instance_id,
                            server_id = event.server_id,
                            status = %event.attestation_status,
                            lifecycle = %event.lifecycle_state,
                            "remediation orchestrator evaluating quarantine event",
                        );
                        if let Err(err) = handle_quarantine(&pool, &event).await {
                            error!(
                                ?err,
                                vm_instance_id = event.vm_instance_id,
                                server_id = event.server_id,
                                "remediation orchestration failed",
                            );
                        }
                    }
                }
                Err(err) => {
                    warn!(?err, "remediation orchestrator receiver dropped");
                    break;
                }
            }
        }
    });
}

async fn handle_quarantine(pool: &PgPool, event: &TrustRegistryEvent) -> Result<(), sqlx::Error> {
    let current_state = get_registry_state(pool, event.vm_instance_id).await?;
    if let Some(state) = &current_state {
        if state.lifecycle_state == "remediating"
            && state
                .remediation_state
                .as_deref()
                .map(|value| value == "remediation:automation-running")
                .unwrap_or(false)
        {
            debug!(
                vm_instance_id = event.vm_instance_id,
                "remediation automation already running; skipping new run",
            );
            return Ok(());
        }
    }

    let mut tx = pool.begin().await?;
    let fallback_provenance = event
        .provenance_ref
        .as_ref()
        .map(|value| json!({ "provenance_ref": value }));
    let provenance_payload = event.provenance.as_ref().or(fallback_provenance.as_ref());

    let started = ensure_running_playbook(
        &mut *tx,
        event.vm_instance_id,
        DEFAULT_PLAYBOOK,
        provenance_payload,
        false,
    )
    .await?;

    if !started {
        tx.commit().await?;
        return Ok(());
    }

    let attempts = current_state
        .as_ref()
        .map(|state| state.remediation_attempts + 1)
        .unwrap_or(event.remediation_attempts + 1);
    let expected_version = current_state.as_ref().map(|state| state.version);

    match upsert_registry_state(
        &mut *tx,
        UpsertRuntimeVmTrustRegistryState {
            runtime_vm_instance_id: event.vm_instance_id,
            attestation_status: event.attestation_status.as_str(),
            lifecycle_state: "remediating",
            remediation_state: Some("remediation:automation-running"),
            remediation_attempts: attempts,
            freshness_deadline: event.freshness_deadline,
            provenance_ref: event.provenance_ref.as_deref(),
            provenance: event.provenance.as_ref(),
            expected_version,
        },
    )
    .await
    {
        Ok(_) => {}
        Err(sqlx::Error::RowNotFound) => {
            warn!(
                vm_instance_id = event.vm_instance_id,
                "remediation orchestrator lost optimistic lock while updating registry",
            );
            tx.rollback().await?;
            return Ok(());
        }
        Err(err) => {
            tx.rollback().await?;
            return Err(err);
        }
    }

    tx.commit().await?;

    let pool_clone = pool.clone();
    let vm_instance_id = event.vm_instance_id;
    tokio::spawn(async move {
        if let Err(err) = complete_remediation(pool_clone, vm_instance_id).await {
            error!(
                ?err,
                vm_instance_id, "remediation automation completion failed"
            );
        }
    });

    Ok(())
}

async fn complete_remediation(pool: PgPool, vm_instance_id: i64) -> Result<(), sqlx::Error> {
    sleep(Duration::from_secs(1)).await;

    let mut tx = pool.begin().await?;
    if !mark_run_completed(&mut *tx, vm_instance_id, None).await? {
        tx.commit().await?;
        return Ok(());
    }

    if let Some(state) = get_registry_state(&mut *tx, vm_instance_id).await? {
        let expected_version = Some(state.version);
        upsert_registry_state(
            &mut *tx,
            UpsertRuntimeVmTrustRegistryState {
                runtime_vm_instance_id: vm_instance_id,
                attestation_status: state.attestation_status.as_str(),
                lifecycle_state: "remediating",
                remediation_state: Some("remediation:automation-complete"),
                remediation_attempts: state.remediation_attempts,
                freshness_deadline: state.freshness_deadline,
                provenance_ref: state.provenance_ref.as_deref(),
                provenance: state.provenance.as_ref(),
                expected_version,
            },
        )
        .await?;
    }

    tx.commit().await?;
    info!(vm_instance_id, "remediation automation completed");
    Ok(())
}

pub async fn record_failure(pool: &PgPool, vm_instance_id: i64, error_message: &str) {
    if let Ok(mut tx) = pool.begin().await {
        if mark_run_failed(&mut *tx, vm_instance_id, error_message)
            .await
            .unwrap_or(false)
        {
            if let Ok(Some(state)) = get_registry_state(&mut *tx, vm_instance_id).await {
                let _ = upsert_registry_state(
                    &mut *tx,
                    UpsertRuntimeVmTrustRegistryState {
                        runtime_vm_instance_id: vm_instance_id,
                        attestation_status: state.attestation_status.as_str(),
                        lifecycle_state: "quarantined",
                        remediation_state: Some("remediation:automation-failed"),
                        remediation_attempts: state.remediation_attempts,
                        freshness_deadline: state.freshness_deadline,
                        provenance_ref: state.provenance_ref.as_deref(),
                        provenance: state.provenance.as_ref(),
                        expected_version: Some(state.version),
                    },
                )
                .await;
            }
            let _ = tx.commit().await;
        }
    }
}
