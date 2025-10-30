use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    BillingPlan, BillingPlanCatalogEntry, BillingQuotaOutcome, BillingService,
    OrganizationSubscription,
};

/// key: billing-api -> rest endpoints
pub async fn list_plans(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<BillingPlan>>, StatusCode> {
    let plans = sqlx::query_as::<_, BillingPlan>(
        "SELECT * FROM billing_plans WHERE active = TRUE ORDER BY created_at ASC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;

    Ok(Json(plans))
}

pub async fn list_plan_catalog(
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<BillingPlanCatalogEntry>>, StatusCode> {
    let service = BillingService::new(pool);
    let catalog = service
        .plan_catalog()
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;
    Ok(Json(catalog))
}

pub async fn get_subscription(
    Extension(pool): Extension<PgPool>,
    Path(organization_id): Path<i32>,
) -> Result<Json<Option<SubscriptionEnvelope>>, StatusCode> {
    let service = BillingService::new(pool.clone());
    let subscription = service
        .active_subscription(organization_id, Utc::now())
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;

    let response =
        subscription.map(|(subscription, plan)| SubscriptionEnvelope { subscription, plan });
    Ok(Json(response))
}

pub async fn upsert_subscription(
    Extension(pool): Extension<PgPool>,
    Path(organization_id): Path<i32>,
    Json(payload): Json<UpsertSubscriptionRequest>,
) -> Result<Json<SubscriptionEnvelope>, StatusCode> {
    let service = BillingService::new(pool.clone());
    let status = payload.status.unwrap_or_else(|| "active".to_string());
    let record = service
        .upsert_subscription(
            organization_id,
            payload.plan_id,
            &status,
            payload.trial_ends_at,
        )
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;

    let plan = sqlx::query_as::<_, BillingPlan>("SELECT * FROM billing_plans WHERE id = $1")
        .bind(payload.plan_id)
        .fetch_one(&pool)
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;

    Ok(Json(SubscriptionEnvelope {
        subscription: record,
        plan,
    }))
}

pub async fn check_quota(
    Extension(pool): Extension<PgPool>,
    Path(organization_id): Path<i32>,
    Json(payload): Json<QuotaCheckRequest>,
) -> Result<Json<QuotaCheckResponse>, StatusCode> {
    let service = BillingService::new(pool);
    let requested = payload.requested_quantity.unwrap_or(0);
    let record_usage = payload.record_usage.unwrap_or(false);
    let outcome = service
        .enforce_quota(
            organization_id,
            &payload.entitlement_key,
            requested,
            record_usage,
        )
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;

    let recorded = record_usage && requested > 0 && outcome.allowed;
    Ok(Json(QuotaCheckResponse { outcome, recorded }))
}

#[derive(Debug, Serialize)]
pub struct SubscriptionEnvelope {
    pub subscription: OrganizationSubscription,
    pub plan: BillingPlan,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSubscriptionRequest {
    pub plan_id: Uuid,
    #[serde(default)]
    pub status: Option<String>,
    pub trial_ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct QuotaCheckRequest {
    pub entitlement_key: String,
    #[serde(default)]
    pub requested_quantity: Option<i64>,
    #[serde(default)]
    pub record_usage: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct QuotaCheckResponse {
    pub outcome: BillingQuotaOutcome,
    pub recorded: bool,
}
