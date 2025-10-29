use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, TimeZone, Timelike, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::models::{
    BillingPlan, BillingQuotaOutcome, OrganizationSubscription, PlanEntitlement,
    SubscriptionUsageWindow,
};

/// key: billing-service -> subscription lifecycle
#[derive(Clone)]
pub struct BillingService {
    pool: PgPool,
}

impl BillingService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn active_subscription(
        &self,
        organization_id: i32,
        now: DateTime<Utc>,
    ) -> Result<Option<(OrganizationSubscription, BillingPlan)>> {
        let row = sqlx::query(
            r#"
            SELECT
                s.id,
                s.organization_id,
                s.plan_id,
                s.status,
                s.trial_ends_at,
                s.current_period_start,
                s.current_period_end,
                s.canceled_at,
                s.created_at,
                s.updated_at,
                p.id as plan_id_row,
                p.code,
                p.name,
                p.description,
                p.billing_period,
                p.currency,
                p.amount_cents,
                p.active,
                p.created_at as plan_created_at,
                p.updated_at as plan_updated_at
            FROM organization_subscriptions s
            JOIN billing_plans p ON p.id = s.plan_id
            WHERE s.organization_id = $1
            ORDER BY s.updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(organization_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let subscription = OrganizationSubscription {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            plan_id: row.get("plan_id"),
            status: row.get("status"),
            trial_ends_at: row.get("trial_ends_at"),
            current_period_start: row.get("current_period_start"),
            current_period_end: row.get("current_period_end"),
            canceled_at: row.get("canceled_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        };

        if !subscription.is_active(now) {
            return Ok(None);
        }

        let plan = BillingPlan {
            id: row.get("plan_id_row"),
            code: row.get("code"),
            name: row.get("name"),
            description: row.get("description"),
            billing_period: row.get("billing_period"),
            currency: row.get("currency"),
            amount_cents: row.get("amount_cents"),
            active: row.get("active"),
            created_at: row.get("plan_created_at"),
            updated_at: row.get("plan_updated_at"),
        };

        Ok(Some((subscription, plan)))
    }

    pub async fn upsert_subscription(
        &self,
        organization_id: i32,
        plan_id: Uuid,
        status: &str,
        trial_ends_at: Option<DateTime<Utc>>,
    ) -> Result<OrganizationSubscription> {
        let existing_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM organization_subscriptions WHERE organization_id = $1 ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(organization_id)
        .fetch_optional(&self.pool)
        .await?;

        let subscription_id = existing_id.unwrap_or_else(Uuid::new_v4);
        let row = sqlx::query_as::<_, OrganizationSubscription>(
            r#"
            INSERT INTO organization_subscriptions (
                id,
                organization_id,
                plan_id,
                status,
                trial_ends_at,
                current_period_start,
                current_period_end
            ) VALUES ($1, $2, $3, $4, $5, NOW(), NULL)
            ON CONFLICT (id)
            DO UPDATE SET
                status = EXCLUDED.status,
                trial_ends_at = EXCLUDED.trial_ends_at,
                plan_id = EXCLUDED.plan_id,
                updated_at = NOW()
            RETURNING *
            "#,
        )
        .bind(subscription_id)
        .bind(organization_id)
        .bind(plan_id)
        .bind(status)
        .bind(trial_ends_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn enforce_quota(
        &self,
        organization_id: i32,
        entitlement_key: &str,
        requested_quantity: i64,
        record_usage: bool,
    ) -> Result<BillingQuotaOutcome> {
        if requested_quantity < 0 {
            return Err(anyhow!("requested quantity must be non-negative"));
        }

        let now = Utc::now();
        let Some((subscription, _plan)) = self.active_subscription(organization_id, now).await?
        else {
            return Ok(BillingQuotaOutcome {
                allowed: false,
                entitlement_key: entitlement_key.to_string(),
                limit_quantity: Some(0),
                used_quantity: 0,
                remaining_quantity: Some(0),
                notes: vec!["billing:subscription-missing".to_string()],
            });
        };

        let entitlement = self
            .plan_entitlement(subscription.plan_id, entitlement_key)
            .await?;

        let Some(entitlement) = entitlement else {
            return Ok(BillingQuotaOutcome {
                allowed: true,
                entitlement_key: entitlement_key.to_string(),
                limit_quantity: None,
                used_quantity: 0,
                remaining_quantity: None,
                notes: vec!["billing:entitlement-unlimited".to_string()],
            });
        };

        let (window_start, window_end) = window_bounds(now, entitlement.reset_interval.as_str());
        let usage = self
            .usage_window(subscription.id, entitlement_key, window_start, window_end)
            .await?;
        let current_used = usage.as_ref().map(|u| u.used_quantity).unwrap_or(0);
        let limit = entitlement.limit_quantity;

        let mut notes = Vec::new();
        let mut allowed = true;
        let mut remaining = limit.map(|limit| limit.saturating_sub(current_used));

        if let Some(limit) = limit {
            let future_used = current_used + requested_quantity;
            if future_used > limit {
                allowed = false;
                remaining = Some(limit.saturating_sub(current_used));
                notes.push(format!("billing:quota-exceeded:{entitlement_key}"));
            } else {
                remaining = Some(limit - future_used);
                notes.push(format!(
                    "billing:quota:{entitlement_key}:{future_used}/{limit}"
                ));
            }
        } else {
            notes.push(format!("billing:quota:{entitlement_key}:unlimited"));
        }

        if allowed && record_usage && requested_quantity > 0 {
            self.record_usage(
                subscription.id,
                entitlement_key,
                window_start,
                window_end,
                requested_quantity,
            )
            .await?;
        }

        Ok(BillingQuotaOutcome {
            allowed,
            entitlement_key: entitlement_key.to_string(),
            limit_quantity: limit,
            used_quantity: current_used,
            remaining_quantity: remaining,
            notes,
        })
    }

    pub async fn record_usage(
        &self,
        subscription_id: Uuid,
        entitlement_key: &str,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        delta: i64,
    ) -> Result<SubscriptionUsageWindow> {
        let row = sqlx::query_as::<_, SubscriptionUsageWindow>(
            r#"
            INSERT INTO subscription_usage_ledger (
                id,
                subscription_id,
                entitlement_key,
                window_start,
                window_end,
                used_quantity
            ) VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (subscription_id, entitlement_key, window_start, window_end)
            DO UPDATE SET
                used_quantity = subscription_usage_ledger.used_quantity + EXCLUDED.used_quantity,
                updated_at = NOW()
            RETURNING *
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(subscription_id)
        .bind(entitlement_key)
        .bind(window_start)
        .bind(window_end)
        .bind(delta)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    async fn plan_entitlement(
        &self,
        plan_id: Uuid,
        entitlement_key: &str,
    ) -> Result<Option<PlanEntitlement>> {
        let record = sqlx::query_as::<_, PlanEntitlement>(
            r#"SELECT * FROM billing_plan_entitlements WHERE plan_id = $1 AND entitlement_key = $2"#,
        )
        .bind(plan_id)
        .bind(entitlement_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(record)
    }

    async fn usage_window(
        &self,
        subscription_id: Uuid,
        entitlement_key: &str,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<Option<SubscriptionUsageWindow>> {
        let record = sqlx::query_as::<_, SubscriptionUsageWindow>(
            r#"
            SELECT * FROM subscription_usage_ledger
            WHERE subscription_id = $1
              AND entitlement_key = $2
              AND window_start = $3
              AND window_end = $4
            "#,
        )
        .bind(subscription_id)
        .bind(entitlement_key)
        .bind(window_start)
        .bind(window_end)
        .fetch_optional(&self.pool)
        .await?;
        Ok(record)
    }
}

fn window_bounds(now: DateTime<Utc>, interval: &str) -> (DateTime<Utc>, DateTime<Utc>) {
    match interval {
        "daily" => {
            let start = Utc
                .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
                .single()
                .unwrap();
            let end = start + Duration::days(1);
            (start, end)
        }
        "weekly" => {
            let weekday = now.weekday().num_days_from_monday() as i64;
            let start = now
                - Duration::days(weekday)
                - Duration::seconds(now.num_seconds_from_midnight() as i64);
            let start = Utc
                .with_ymd_and_hms(start.year(), start.month(), start.day(), 0, 0, 0)
                .single()
                .unwrap();
            let end = start + Duration::days(7);
            (start, end)
        }
        _ => {
            let start_date = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let start = Utc.from_utc_datetime(&start_date);
            let end = start + Months::new(1);
            (start, end)
        }
    }
}
