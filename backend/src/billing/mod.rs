pub mod adapters;
pub mod api;
pub mod models;
pub mod reconciliation;
pub mod scheduler;
pub mod service;

pub use adapters::{BillingProviderAdapter, StripeLikeAdapter, UsageReconciliationRecord};
pub use api::{
    check_quota as billing_check_quota, get_subscription as billing_get_subscription,
    list_plan_catalog as billing_list_plan_catalog, list_plans as billing_list_plans,
    upsert_subscription as billing_upsert_subscription, QuotaCheckRequest, QuotaCheckResponse,
    SubscriptionEnvelope, UpsertSubscriptionRequest,
};
pub use models::{
    BillingPlan, BillingPlanCatalogEntry, BillingQuotaOutcome, OrganizationSubscription,
    PlanEntitlement, SubscriptionUsageWindow,
};
pub use reconciliation::{start_reconciliation_worker, ReconciliationHandle, ReconciliationJob};
pub use scheduler::{
    process_tick as run_billing_automation_tick, spawn as spawn_billing_scheduler,
};
pub use service::BillingService;
