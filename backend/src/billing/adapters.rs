use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::models::OrganizationSubscription;
use super::service::BillingService;

/// key: billing-usage-record -> normalized usage entry
#[derive(Clone, Debug)]
pub struct UsageReconciliationRecord {
    pub subscription_id: Uuid,
    pub entitlement_key: String,
    pub quantity: i64,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

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
    fn normalize_usage(&self, payload: Value) -> Result<Vec<UsageReconciliationRecord>>;
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
            .ok_or_else(|| anyhow!("plan_id missing"))?;
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

    fn normalize_usage(&self, payload: Value) -> Result<Vec<UsageReconciliationRecord>> {
        let entries = payload
            .get("entries")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow!("usage entries missing"))?;

        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            let subscription_id = entry
                .get("subscription_id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("subscription_id missing"))?;
            let subscription_id = Uuid::parse_str(subscription_id)?;
            let entitlement_key = entry
                .get("entitlement_key")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("entitlement_key missing"))?
                .to_string();
            let quantity = entry
                .get("quantity")
                .and_then(|value| value.as_i64())
                .ok_or_else(|| anyhow!("quantity missing"))?;
            if quantity < 0 {
                return Err(anyhow!("usage quantity must be non-negative"));
            }
            let window = entry
                .get("window")
                .and_then(|value| value.as_object())
                .ok_or_else(|| anyhow!("usage window missing"))?;
            let window_start = window
                .get("start")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("window start missing"))?;
            let window_end = window
                .get("end")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("window end missing"))?;
            let window_start = DateTime::parse_from_rfc3339(window_start)?.with_timezone(&Utc);
            let window_end = DateTime::parse_from_rfc3339(window_end)?.with_timezone(&Utc);
            if window_end <= window_start {
                return Err(anyhow!("usage window end must be after start"));
            }

            records.push(UsageReconciliationRecord {
                subscription_id,
                entitlement_key,
                quantity,
                window_start,
                window_end,
            });
        }

        Ok(records)
    }
}

fn json_stub(kind: &str, metadata: Value) -> Value {
    serde_json::json!({
        "kind": kind,
        "metadata": metadata,
        "integration": "stubbed",
    })
}
