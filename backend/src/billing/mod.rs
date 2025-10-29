pub mod adapters;
pub mod api;
pub mod models;
pub mod service;

pub use adapters::{BillingProviderAdapter, StripeLikeAdapter};
pub use api::{
    check_quota as billing_check_quota, get_subscription as billing_get_subscription,
    list_plans as billing_list_plans, upsert_subscription as billing_upsert_subscription,
    QuotaCheckRequest, QuotaCheckResponse, SubscriptionEnvelope, UpsertSubscriptionRequest,
};
pub use models::{
    BillingPlan, BillingQuotaOutcome, OrganizationSubscription, PlanEntitlement,
    SubscriptionUsageWindow,
};
pub use service::BillingService;
