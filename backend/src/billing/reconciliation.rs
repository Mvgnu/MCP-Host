use anyhow::{anyhow, Result};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::mpsc::{channel, Sender};
use tracing::{error, info, warn};

use super::adapters::{BillingProviderAdapter, StripeLikeAdapter};
use super::service::BillingService;

/// key: billing-reconciliation -> background worker for provider callbacks
#[derive(Debug)]
pub enum ReconciliationJob {
    SubscriptionSync {
        organization_id: i32,
        payload: Value,
    },
    UsageReport {
        organization_id: i32,
        payload: Value,
    },
}

/// key: billing-reconciliation-handle -> enqueue interface
#[derive(Clone)]
pub struct ReconciliationHandle {
    sender: Sender<ReconciliationJob>,
}

impl ReconciliationHandle {
    pub async fn dispatch(&self, job: ReconciliationJob) -> Result<()> {
        self.sender
            .send(job)
            .await
            .map_err(|err| anyhow!("failed to enqueue billing reconciliation job: {err}"))
    }
}

pub fn start_reconciliation_worker(pool: PgPool) -> ReconciliationHandle {
    let (tx, mut rx) = channel(64);
    tokio::spawn(async move {
        let adapter = StripeLikeAdapter;
        let service = BillingService::new(pool.clone());
        while let Some(job) = rx.recv().await {
            match job {
                ReconciliationJob::SubscriptionSync {
                    organization_id,
                    payload,
                } => {
                    if let Err(err) = adapter
                        .sync_subscription(&service, organization_id, payload)
                        .await
                    {
                        error!(
                            ?err,
                            %organization_id,
                            "failed to reconcile subscription update from provider",
                        );
                    }
                }
                ReconciliationJob::UsageReport {
                    organization_id,
                    payload,
                } => match adapter.normalize_usage(payload) {
                    Ok(records) => {
                        if records.is_empty() {
                            warn!(
                                %organization_id,
                                "usage reconciliation received empty payload"
                            );
                            continue;
                        }
                        for record in records {
                            if let Err(err) =
                                service.settle_usage(organization_id, record.clone()).await
                            {
                                error!(
                                    ?err,
                                    %organization_id,
                                    entitlement = record.entitlement_key,
                                    subscription = %record.subscription_id,
                                    "failed to settle usage into ledger",
                                );
                            } else {
                                info!(
                                    %organization_id,
                                    entitlement = record.entitlement_key,
                                    subscription = %record.subscription_id,
                                    quantity = record.quantity,
                                    "usage record settled"
                                );
                            }
                        }
                    }
                    Err(err) => error!(
                        ?err,
                        %organization_id,
                        "failed to normalize usage payload from provider",
                    ),
                },
            }
        }
    });

    ReconciliationHandle { sender: tx }
}
