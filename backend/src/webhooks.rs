use axum::{extract::Extension, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::Value;

use crate::billing::{ReconciliationHandle, ReconciliationJob};

/// key: webhooks-billing -> adapter entrypoint
#[derive(Debug, Deserialize)]
pub struct BillingWebhookRequest {
    pub organization_id: i32,
    pub event: String,
    #[serde(default)]
    pub data: Value,
}

pub async fn billing_webhook(
    Extension(reconciliation): Extension<ReconciliationHandle>,
    Json(payload): Json<BillingWebhookRequest>,
) -> Result<StatusCode, StatusCode> {
    match payload.event.as_str() {
        "subscription.updated" | "subscription.created" => {
            reconciliation
                .dispatch(ReconciliationJob::SubscriptionSync {
                    organization_id: payload.organization_id,
                    payload: payload.data,
                })
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(StatusCode::ACCEPTED)
        }
        "usage.reported" | "usage.reconciled" => {
            reconciliation
                .dispatch(ReconciliationJob::UsageReport {
                    organization_id: payload.organization_id,
                    payload: payload.data,
                })
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(StatusCode::ACCEPTED)
        }
        _ => Ok(StatusCode::ACCEPTED),
    }
}
