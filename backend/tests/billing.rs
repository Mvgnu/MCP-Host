use backend::billing::BillingService;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// key: billing-tests -> multi-entitlements,quota-gates
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn billing_multi_entitlement_quota_notes(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("billing@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let organization_id: i32 = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("Quota Driven Org")
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO billing_plans (id, code, name, description, amount_cents) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(plan_id)
    .bind("multi-entitlement")
    .bind("Multi Entitlement")
    .bind("Plan with mixed entitlements")
    .bind(9900_i32)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO billing_plan_entitlements (id, plan_id, entitlement_key, limit_quantity, reset_interval, metadata) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind("runtime.concurrent_servers")
    .bind(Some(10_i64))
    .bind("monthly")
    .bind(json!({}))
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO billing_plan_entitlements (id, plan_id, entitlement_key, limit_quantity, reset_interval, metadata) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(plan_id)
    .bind("marketplace.catalog.listings")
    .bind::<Option<i64>>(None)
    .bind("monthly")
    .bind(json!({}))
    .execute(&pool)
    .await
    .unwrap();

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO organization_subscriptions (id, organization_id, plan_id, status) VALUES ($1, $2, $3, 'active')",
    )
    .bind(subscription_id)
    .bind(organization_id)
    .bind(plan_id)
    .execute(&pool)
    .await
    .unwrap();

    let service = BillingService::new(pool.clone());

    let initial = service
        .enforce_quota(organization_id, "runtime.concurrent_servers", 3, true)
        .await
        .unwrap();
    assert!(
        initial.allowed,
        "initial entitlement check should be allowed"
    );
    assert_eq!(initial.limit_quantity, Some(10));
    assert_eq!(initial.used_quantity, 0);
    assert_eq!(initial.remaining_quantity, Some(7));
    assert!(
        initial
            .notes
            .contains(&"billing:quota:runtime.concurrent_servers:3/10".to_string()),
        "expected quota ratio note"
    );

    let over_limit = service
        .enforce_quota(organization_id, "runtime.concurrent_servers", 8, false)
        .await
        .unwrap();
    assert!(
        !over_limit.allowed,
        "usage above entitlement should veto launches"
    );
    assert_eq!(over_limit.limit_quantity, Some(10));
    assert_eq!(over_limit.used_quantity, 3);
    assert_eq!(over_limit.remaining_quantity, Some(7));
    assert!(
        over_limit
            .notes
            .contains(&"billing:quota-exceeded:runtime.concurrent_servers".to_string()),
        "expected actionable veto note"
    );

    let unlimited = service
        .enforce_quota(organization_id, "marketplace.catalog.listings", 25, false)
        .await
        .unwrap();
    assert!(unlimited.allowed);
    assert_eq!(unlimited.limit_quantity, None);
    assert!(unlimited
        .notes
        .contains(&"billing:quota:marketplace.catalog.listings:unlimited".to_string()));

    let ledger_entry: (Uuid, String, i64) = sqlx::query_as(
        "SELECT subscription_id, entitlement_key, used_quantity FROM subscription_usage_ledger WHERE subscription_id = $1",
    )
    .bind(subscription_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger_entry.0, subscription_id);
    assert_eq!(ledger_entry.1, "runtime.concurrent_servers");
    assert_eq!(ledger_entry.2, 3);
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn billing_missing_subscription_surfaces_veto(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let service = BillingService::new(pool.clone());
    let outcome = service
        .enforce_quota(42, "runtime.concurrent_servers", 1, false)
        .await
        .unwrap();

    assert!(!outcome.allowed);
    assert_eq!(outcome.limit_quantity, Some(0));
    assert_eq!(outcome.remaining_quantity, Some(0));
    assert!(outcome
        .notes
        .contains(&"billing:subscription-missing".to_string()));
}
