use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use super::models::OrganizationSubscription;
use super::service::BillingService;

/// key: billing-adapter -> provider integration
#[async_trait]
pub trait BillingProviderAdapter: Send + Sync {
    async fn provision_customer(&self, organization_id: i32, metadata: Value) -> Result<Value>;
    async fn sync_subscription(
        &self,
        service: &BillingService,
        organization_id: i32,
        payload: Value,
    ) -> Result<OrganizationSubscription>;
}

/// key: billing-adapter-stripe -> stub implementation
pub struct StripeLikeAdapter;

#[async_trait]
impl BillingProviderAdapter for StripeLikeAdapter {
    async fn provision_customer(&self, _organization_id: i32, metadata: Value) -> Result<Value> {
        Ok(json_stub("customer", metadata))
    }

    async fn sync_subscription(
        &self,
        service: &BillingService,
        organization_id: i32,
        payload: Value,
    ) -> Result<OrganizationSubscription> {
        let plan_id = payload
            .get("plan_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("plan_id missing"))?;
        let plan_id = Uuid::parse_str(plan_id)?;
        let status = payload
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("active");
        let trial_end = payload
            .get("trial_ends_at")
            .and_then(|v| v.as_str())
            .and_then(|value| value.parse().ok());

        service
            .upsert_subscription(organization_id, plan_id, status, trial_end)
            .await
    }
}

fn json_stub(kind: &str, metadata: Value) -> Value {
    serde_json::json!({
        "kind": kind,
        "metadata": metadata,
        "integration": "stubbed",
    })
}
