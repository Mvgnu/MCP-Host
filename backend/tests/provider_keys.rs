use backend::keys::{ProviderKeyService, ProviderKeyServiceConfig, RegisterProviderKey};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn revoke_key_marks_compromised_and_emits_events(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let service = ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());
    let provider_id = Uuid::new_v4();
    let record = service
        .register_key(
            provider_id,
            RegisterProviderKey {
                alias: Some("primary".to_string()),
                attestation_digest: Some(STANDARD.encode(b"digest")),
                attestation_signature: Some(STANDARD.encode(b"signature")),
                rotation_due_at: Some(Utc::now() + Duration::hours(24)),
            },
        )
        .await
        .unwrap();

    let updated = service
        .revoke_key(provider_id, record.id, Some("emergency".to_string()), true)
        .await
        .unwrap();

    assert!(matches!(
        updated.state,
        backend::keys::ProviderKeyState::Compromised
    ));
    assert!(updated.compromised_at.is_some());
    assert!(updated.retired_at.is_some());

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_key_audit_events WHERE provider_key_id = $1 AND event_type IN ('revocation_initiated','revocation_completed')",
    )
    .bind(record.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 2);
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn enforce_rotation_slas_emits_breach_once(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let service = ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());
    let provider_id = Uuid::new_v4();
    let record = service
        .register_key(
            provider_id,
            RegisterProviderKey {
                alias: Some("rotating".to_string()),
                attestation_digest: Some(STANDARD.encode(b"digest")),
                attestation_signature: Some(STANDARD.encode(b"signature")),
                rotation_due_at: Some(Utc::now() - Duration::hours(1)),
            },
        )
        .await
        .unwrap();

    let report = service
        .enforce_rotation_slas(Duration::hours(12), Duration::hours(6))
        .await
        .unwrap();
    assert_eq!(report.breached.len(), 1);
    assert!(report.breached[0].event_emitted);

    let second = service
        .enforce_rotation_slas(Duration::hours(12), Duration::hours(6))
        .await
        .unwrap();
    assert_eq!(second.breached.len(), 1);
    assert!(!second.breached[0].event_emitted);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_key_audit_events WHERE provider_key_id = $1 AND event_type = 'rotation_sla_breached'",
    )
    .bind(record.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 1);
}
