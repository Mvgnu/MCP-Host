use backend::billing::scheduler;
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

// key: billing-scheduler-tests -> automated renewal flows
#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn billing_scheduler_marks_past_due(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let now = Utc::now();
    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("scheduler@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let organization_id: i32 = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("Scheduler Org")
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let plan_id = Uuid::new_v4();
    sqlx::query("INSERT INTO billing_plans (id, code, name, amount_cents) VALUES ($1, $2, $3, $4)")
        .bind(plan_id)
        .bind("pro-plan")
        .bind("Pro Plan")
        .bind(19900_i32)
        .execute(&pool)
        .await
        .unwrap();

    let subscription_id = Uuid::new_v4();
    let start_at = now - Duration::days(45);
    sqlx::query(
        "INSERT INTO organization_subscriptions (id, organization_id, plan_id, status, current_period_start, current_period_end) VALUES ($1, $2, $3, 'active', $4, NULL)",
    )
    .bind(subscription_id)
    .bind(organization_id)
    .bind(plan_id)
    .bind(start_at)
    .execute(&pool)
    .await
    .unwrap();

    scheduler::process_tick(&pool, now, 3, None).await.unwrap();

    let status: String =
        sqlx::query_scalar("SELECT status FROM organization_subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(status, "past_due");
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn billing_scheduler_downgrades_with_fallback(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let now = Utc::now();
    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("fallback@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let organization_id: i32 = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("Fallback Org")
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let pro_plan_id = Uuid::new_v4();
    sqlx::query("INSERT INTO billing_plans (id, code, name, amount_cents) VALUES ($1, $2, $3, $4)")
        .bind(pro_plan_id)
        .bind("pro-plan")
        .bind("Pro Plan")
        .bind(19900_i32)
        .execute(&pool)
        .await
        .unwrap();

    let fallback_plan_id = Uuid::new_v4();
    sqlx::query("INSERT INTO billing_plans (id, code, name, amount_cents) VALUES ($1, $2, $3, $4)")
        .bind(fallback_plan_id)
        .bind("free-plan")
        .bind("Free Plan")
        .bind(0_i32)
        .execute(&pool)
        .await
        .unwrap();

    let subscription_id = Uuid::new_v4();
    let start_at = now - Duration::days(45);
    let updated_at = now - Duration::days(5);
    sqlx::query(
        "INSERT INTO organization_subscriptions (id, organization_id, plan_id, status, current_period_start, current_period_end, updated_at) VALUES ($1, $2, $3, 'past_due', $4, NULL, $5)",
    )
    .bind(subscription_id)
    .bind(organization_id)
    .bind(pro_plan_id)
    .bind(start_at)
    .bind(updated_at)
    .execute(&pool)
    .await
    .unwrap();

    scheduler::process_tick(&pool, now, 0, Some("free-plan"))
        .await
        .unwrap();

    let (plan_id, status): (Uuid, String) =
        sqlx::query_as("SELECT plan_id, status FROM organization_subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(plan_id, fallback_plan_id);
    assert_eq!(status, "active");
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn billing_scheduler_suspends_without_fallback(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let now = Utc::now();
    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind("suspend@example.com")
            .bind("hashed")
            .fetch_one(&pool)
            .await
            .unwrap();

    let organization_id: i32 = sqlx::query_scalar(
        "INSERT INTO organizations (name, owner_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("Suspend Org")
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let plan_id = Uuid::new_v4();
    sqlx::query("INSERT INTO billing_plans (id, code, name, amount_cents) VALUES ($1, $2, $3, $4)")
        .bind(plan_id)
        .bind("suspend-plan")
        .bind("Suspend Plan")
        .bind(9900_i32)
        .execute(&pool)
        .await
        .unwrap();

    let subscription_id = Uuid::new_v4();
    let start_at = now - Duration::days(60);
    let updated_at = now - Duration::days(10);
    sqlx::query(
        "INSERT INTO organization_subscriptions (id, organization_id, plan_id, status, current_period_start, current_period_end, updated_at) VALUES ($1, $2, $3, 'past_due', $4, NULL, $5)",
    )
    .bind(subscription_id)
    .bind(organization_id)
    .bind(plan_id)
    .bind(start_at)
    .bind(updated_at)
    .execute(&pool)
    .await
    .unwrap();

    scheduler::process_tick(&pool, now, 3, None).await.unwrap();

    let (status, current_period_end): (String, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, current_period_end FROM organization_subscriptions WHERE id = $1",
    )
    .bind(subscription_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, "suspended");
    assert!(current_period_end.is_some());
}
