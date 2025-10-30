use anyhow::Result;
use chrono::{DateTime, Duration, Months, Utc};
use sqlx::{FromRow, PgPool};
use tokio::time::{self, Duration as TokioDuration};
use tracing::{debug, info, warn};

use crate::config;

use super::service::BillingService;

/// key: billing-renewal-scheduler -> automate overdue handling
pub fn spawn(pool: PgPool) {
    let interval = TokioDuration::from_secs(*config::BILLING_RENEWAL_SCAN_INTERVAL_SECS);
    let grace_days = *config::BILLING_PAST_DUE_GRACE_DAYS;
    let fallback_plan_code = config::BILLING_FALLBACK_PLAN_CODE.clone();

    tokio::spawn(async move {
        let mut ticker = time::interval(interval);
        loop {
            ticker.tick().await;
            let now = Utc::now();
            if let Err(err) =
                process_tick(&pool, now, grace_days, fallback_plan_code.as_deref()).await
            {
                warn!(?err, "billing renewal automation tick failed");
            }
        }
    });
}

/// key: billing-renewal-scheduler -> tick handler
pub async fn process_tick(
    pool: &PgPool,
    now: DateTime<Utc>,
    grace_days: i64,
    fallback_plan_code: Option<&str>,
) -> Result<()> {
    let service = BillingService::new(pool.clone());
    let renewal_candidates = sqlx::query_as::<_, RenewalCandidate>(
        r#"
        SELECT
            s.id,
            s.organization_id,
            s.status,
            s.trial_ends_at,
            s.current_period_start,
            s.current_period_end,
            p.billing_period,
            p.code AS plan_code
        FROM organization_subscriptions s
        JOIN billing_plans p ON p.id = s.plan_id
        WHERE s.status IN ('trialing', 'active')
        "#,
    )
    .fetch_all(pool)
    .await?;

    for record in renewal_candidates {
        let mut should_mark_past_due = false;
        if record.status == "trialing" {
            if let Some(trial_end) = record.trial_ends_at {
                if trial_end < now {
                    should_mark_past_due = true;
                }
            }
        }

        let expected_end = compute_period_end(
            record.current_period_start,
            record.current_period_end,
            &record.billing_period,
        );
        if expected_end < now {
            should_mark_past_due = true;
        }

        if should_mark_past_due {
            match service
                .mark_subscription_overdue(record.organization_id)
                .await
            {
                Ok(Some(subscription)) => {
                    info!(
                        organization_id = subscription.organization_id,
                        subscription = %subscription.id,
                        "marked subscription past_due via renewal automation"
                    );
                }
                Ok(None) => {}
                Err(err) => warn!(
                    ?err,
                    organization_id = record.organization_id,
                    "failed to mark subscription past_due"
                ),
            }
        } else {
            debug!(
                organization_id = record.organization_id,
                status = %record.status,
                plan = %record.plan_code,
                "subscription within renewal window"
            );
        }
    }

    let grace_duration = Duration::days(grace_days);
    let past_due_accounts = sqlx::query_as::<_, PastDueAccount>(
        r#"
        SELECT
            s.id,
            s.organization_id,
            s.updated_at,
            p.code AS plan_code
        FROM organization_subscriptions s
        JOIN billing_plans p ON p.id = s.plan_id
        WHERE s.status = 'past_due'
        "#,
    )
    .fetch_all(pool)
    .await?;

    for record in past_due_accounts {
        if record.updated_at + grace_duration > now {
            continue;
        }

        if let Some(plan_code) = fallback_plan_code {
            if record.plan_code != plan_code {
                match service
                    .downgrade_subscription(record.organization_id, plan_code)
                    .await
                {
                    Ok(Some(subscription)) => {
                        info!(
                            organization_id = subscription.organization_id,
                            subscription = %subscription.id,
                            plan = %plan_code,
                            "downgraded subscription after grace period"
                        );
                        continue;
                    }
                    Ok(None) => {}
                    Err(err) => warn!(
                        ?err,
                        organization_id = record.organization_id,
                        plan = %plan_code,
                        "failed to downgrade subscription"
                    ),
                }
            }
        }

        match service.suspend_subscription(record.organization_id).await {
            Ok(Some(subscription)) => {
                info!(
                    organization_id = subscription.organization_id,
                    subscription = %subscription.id,
                    "suspended subscription after grace period"
                );
            }
            Ok(None) => {}
            Err(err) => warn!(
                ?err,
                organization_id = record.organization_id,
                "failed to suspend subscription"
            ),
        }
    }

    Ok(())
}

#[derive(Debug, FromRow)]
struct RenewalCandidate {
    id: uuid::Uuid,
    organization_id: i32,
    status: String,
    trial_ends_at: Option<DateTime<Utc>>,
    current_period_start: DateTime<Utc>,
    current_period_end: Option<DateTime<Utc>>,
    billing_period: String,
    plan_code: String,
}

#[derive(Debug, FromRow)]
struct PastDueAccount {
    id: uuid::Uuid,
    organization_id: i32,
    updated_at: DateTime<Utc>,
    plan_code: String,
}

fn compute_period_end(
    current_period_start: DateTime<Utc>,
    explicit_end: Option<DateTime<Utc>>,
    billing_period: &str,
) -> DateTime<Utc> {
    if let Some(end) = explicit_end {
        return end;
    }

    match billing_period {
        "daily" => current_period_start
            .checked_add_signed(Duration::days(1))
            .unwrap_or(current_period_start),
        "weekly" => current_period_start
            .checked_add_signed(Duration::days(7))
            .unwrap_or(current_period_start),
        "quarterly" => current_period_start
            .checked_add_months(Months::new(3))
            .unwrap_or(current_period_start),
        "annual" | "yearly" => current_period_start
            .checked_add_months(Months::new(12))
            .unwrap_or(current_period_start),
        _ => current_period_start
            .checked_add_months(Months::new(1))
            .unwrap_or(current_period_start),
    }
}
