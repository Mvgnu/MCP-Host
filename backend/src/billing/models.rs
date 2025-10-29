use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// key: billing-models -> plans,subscriptions,usage
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BillingPlan {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub billing_period: String,
    pub currency: String,
    pub amount_cents: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// key: billing-entitlement-model -> plan bindings
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PlanEntitlement {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub entitlement_key: String,
    pub limit_quantity: Option<i64>,
    pub reset_interval: String,
    pub metadata: serde_json::Value,
}

/// key: billing-subscription-model -> organization
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct OrganizationSubscription {
    pub id: Uuid,
    pub organization_id: i32,
    pub plan_id: Uuid,
    pub status: String,
    pub trial_ends_at: Option<DateTime<Utc>>,
    pub current_period_start: DateTime<Utc>,
    pub current_period_end: Option<DateTime<Utc>>,
    pub canceled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OrganizationSubscription {
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if self.status != "active" && self.status != "trialing" {
            return false;
        }
        if let Some(end) = self.current_period_end {
            if end < now {
                return false;
            }
        }
        true
    }
}

/// key: billing-usage-window -> entitlement usage snapshots
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SubscriptionUsageWindow {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub entitlement_key: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub used_quantity: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BillingQuotaOutcome {
    pub allowed: bool,
    pub entitlement_key: String,
    pub limit_quantity: Option<i64>,
    pub used_quantity: i64,
    pub remaining_quantity: Option<i64>,
    pub notes: Vec<String>,
}
