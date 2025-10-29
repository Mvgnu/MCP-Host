use axum::{extract::Extension, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::billing::{BillingProviderAdapter, BillingService, StripeLikeAdapter};

/// key: webhooks-billing -> adapter entrypoint
#[derive(Debug, Deserialize)]
pub struct BillingWebhookRequest {
    pub organization_id: i32,
    pub event: String,
    #[serde(default)]
    pub data: Value,
}

pub async fn billing_webhook(
    Extension(pool): Extension<PgPool>,
    Json(payload): Json<BillingWebhookRequest>,
) -> Result<StatusCode, StatusCode> {
    let adapter = StripeLikeAdapter;
    match payload.event.as_str() {
        "subscription.updated" | "subscription.created" => {
            let service = BillingService::new(pool);
            adapter
                .sync_subscription(&service, payload.organization_id, payload.data)
                .await
                .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;
            Ok(StatusCode::ACCEPTED)
        }
        _ => Ok(StatusCode::ACCEPTED),
    }
}
