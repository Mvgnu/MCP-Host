// key: vector-dbs-tests -> residency,attachments
use axum::{extract::Path, http::StatusCode, Extension, Json};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use backend::extractor::AuthUser;
use backend::keys::{
    ProviderKeyBindingScope, ProviderKeyService, ProviderKeyServiceConfig, RegisterProviderKey,
};
use backend::vector_dbs::{
    create_vector_db, create_vector_db_attachment, detach_vector_db_attachment,
    list_vector_db_attachments, list_vector_db_incidents, log_vector_db_incident,
    resolve_vector_db_incident, upsert_vector_db_residency_policy, CreateVectorDb,
    CreateVectorDbAttachment, CreateVectorDbIncident, DetachVectorDbAttachment,
    ResolveVectorDbIncident, UpsertVectorDbResidencyPolicy,
};

async fn seed_owner(pool: &PgPool) -> i32 {
    sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1,$2) RETURNING id")
        .bind("owner@example.com")
        .bind("hash")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn auth(user_id: i32) -> AuthUser {
    AuthUser {
        user_id,
        role: "user".into(),
    }
}

async fn seed_provider_binding(pool: &PgPool, binding_type: &str) -> (Uuid, Uuid, Uuid) {
    let service = ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());
    let provider_id = Uuid::new_v4();
    let rotation_due = Utc::now() + Duration::days(30);
    let key = service
        .register_key(
            provider_id,
            RegisterProviderKey {
                alias: Some("primary".into()),
                attestation_digest: Some(STANDARD.encode(b"digest")),
                attestation_signature: Some(STANDARD.encode(b"signature")),
                rotation_due_at: Some(rotation_due),
            },
        )
        .await
        .unwrap();

    let binding = service
        .record_binding(
            provider_id,
            key.id,
            ProviderKeyBindingScope {
                binding_type: binding_type.to_string(),
                binding_target_id: Uuid::new_v4(),
                additional_context: json!({ "scope": binding_type }),
            },
        )
        .await
        .unwrap();

    (binding.id, provider_id, key.id)
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn vector_db_attachment_requires_residency_and_vector_binding(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let owner_id = seed_owner(&pool).await;
    let vector = create_vector_db(
        Extension(pool.clone()),
        auth(owner_id),
        Json(CreateVectorDb {
            name: "governed".into(),
            db_type: "chroma".into(),
        }),
    )
    .await
    .unwrap()
    .0;

    let residency = upsert_vector_db_residency_policy(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(UpsertVectorDbResidencyPolicy {
            region: "us-east".into(),
            data_classification: "restricted".into(),
            enforcement_mode: "block".into(),
            active: false,
        }),
    )
    .await
    .unwrap()
    .0;

    let (wrong_binding_id, _, _) = seed_provider_binding(&pool, "workspace").await;

    let err = create_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbAttachment {
            attachment_type: "embedding".into(),
            attachment_ref: Uuid::new_v4(),
            residency_policy_id: residency.id,
            provider_key_binding_id: wrong_binding_id,
            metadata: json!({ "integration": "test" }),
        }),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.0, StatusCode::CONFLICT);

    let residency = upsert_vector_db_residency_policy(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(UpsertVectorDbResidencyPolicy {
            region: "us-east".into(),
            data_classification: "restricted".into(),
            enforcement_mode: "block".into(),
            active: true,
        }),
    )
    .await
    .unwrap()
    .0;

    let (binding_id, _provider_id, key_id) = seed_provider_binding(&pool, "vector_db").await;

    let attachment = create_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbAttachment {
            attachment_type: "embedding".into(),
            attachment_ref: Uuid::new_v4(),
            residency_policy_id: residency.id,
            provider_key_binding_id: binding_id,
            metadata: json!({ "integration": "prod" }),
        }),
    )
    .await
    .unwrap()
    .0;

    assert_eq!(attachment.vector_db_id, vector.id);
    assert_eq!(attachment.residency_policy_id, residency.id);
    assert_eq!(attachment.provider_key_binding_id, binding_id);
    assert_eq!(attachment.provider_key_id, key_id);
    assert!(attachment.provider_key_rotation_due_at.is_some());
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn vector_db_incident_requires_owned_attachment(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let owner_id = seed_owner(&pool).await;
    let vector = create_vector_db(
        Extension(pool.clone()),
        auth(owner_id),
        Json(CreateVectorDb {
            name: "audited".into(),
            db_type: "chroma".into(),
        }),
    )
    .await
    .unwrap()
    .0;

    let residency = upsert_vector_db_residency_policy(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(UpsertVectorDbResidencyPolicy {
            region: "eu-central".into(),
            data_classification: "confidential".into(),
            enforcement_mode: "monitor".into(),
            active: true,
        }),
    )
    .await
    .unwrap()
    .0;

    let (binding_id, _, _) = seed_provider_binding(&pool, "vector_db").await;
    let attachment = create_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbAttachment {
            attachment_type: "semantic".into(),
            attachment_ref: Uuid::new_v4(),
            residency_policy_id: residency.id,
            provider_key_binding_id: binding_id,
            metadata: json!({ "integration": "ops" }),
        }),
    )
    .await
    .unwrap()
    .0;

    let err = log_vector_db_incident(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbIncident {
            incident_type: "residency_breach".into(),
            severity: "high".into(),
            attachment_id: Some(Uuid::new_v4()),
            summary: Some("Unknown attachment referenced".into()),
            notes: json!({ "action": "investigate" }),
        }),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.0, StatusCode::NOT_FOUND);

    let incident = log_vector_db_incident(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbIncident {
            incident_type: "residency_breach".into(),
            severity: "high".into(),
            attachment_id: Some(attachment.id),
            summary: Some("Policy violation detected".into()),
            notes: json!({ "action": "remediate" }),
        }),
    )
    .await
    .unwrap()
    .0;

    assert_eq!(incident.vector_db_id, vector.id);
    assert_eq!(incident.attachment_id, Some(attachment.id));

    let incidents =
        list_vector_db_incidents(Extension(pool.clone()), auth(owner_id), Path(vector.id))
            .await
            .unwrap()
            .0;
    assert_eq!(incidents.len(), 1);
    assert_eq!(incidents[0].incident_type, "residency_breach");
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn vector_db_attachment_can_be_detached(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let owner_id = seed_owner(&pool).await;
    let vector = create_vector_db(
        Extension(pool.clone()),
        auth(owner_id),
        Json(CreateVectorDb {
            name: "detach-me".into(),
            db_type: "chroma".into(),
        }),
    )
    .await
    .unwrap()
    .0;

    let residency = upsert_vector_db_residency_policy(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(UpsertVectorDbResidencyPolicy {
            region: "ap-southeast".into(),
            data_classification: "confidential".into(),
            enforcement_mode: "block".into(),
            active: true,
        }),
    )
    .await
    .unwrap()
    .0;

    let (binding_id, _, _) = seed_provider_binding(&pool, "vector_db").await;

    let attachment = create_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbAttachment {
            attachment_type: "assistant".into(),
            attachment_ref: Uuid::new_v4(),
            residency_policy_id: residency.id,
            provider_key_binding_id: binding_id,
            metadata: json!({ "integration": "detach" }),
        }),
    )
    .await
    .unwrap()
    .0;

    let detached = detach_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path((vector.id, attachment.id)),
        Json(DetachVectorDbAttachment {
            reason: Some("Rolled credential".into()),
        }),
    )
    .await
    .unwrap()
    .0;

    assert!(detached.detached_at.is_some());
    assert_eq!(
        detached.detached_reason.as_deref(),
        Some("Rolled credential")
    );

    let err = detach_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path((vector.id, attachment.id)),
        Json(DetachVectorDbAttachment { reason: None }),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.0, StatusCode::CONFLICT);

    let attachments =
        list_vector_db_attachments(Extension(pool.clone()), auth(owner_id), Path(vector.id))
            .await
            .unwrap()
            .0;
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].detached_at.is_some());
}

#[sqlx::test]
#[ignore = "requires DATABASE_URL with Postgres server"]
async fn vector_db_incident_can_be_resolved(pool: PgPool) {
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let owner_id = seed_owner(&pool).await;
    let vector = create_vector_db(
        Extension(pool.clone()),
        auth(owner_id),
        Json(CreateVectorDb {
            name: "incident-lifecycle".into(),
            db_type: "chroma".into(),
        }),
    )
    .await
    .unwrap()
    .0;

    let residency = upsert_vector_db_residency_policy(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(UpsertVectorDbResidencyPolicy {
            region: "us-west".into(),
            data_classification: "restricted".into(),
            enforcement_mode: "monitor".into(),
            active: true,
        }),
    )
    .await
    .unwrap()
    .0;

    let (binding_id, _, _) = seed_provider_binding(&pool, "vector_db").await;
    let attachment = create_vector_db_attachment(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbAttachment {
            attachment_type: "assistant".into(),
            attachment_ref: Uuid::new_v4(),
            residency_policy_id: residency.id,
            provider_key_binding_id: binding_id,
            metadata: json!({ "integration": "incident" }),
        }),
    )
    .await
    .unwrap()
    .0;

    let incident = log_vector_db_incident(
        Extension(pool.clone()),
        auth(owner_id),
        Path(vector.id),
        Json(CreateVectorDbIncident {
            incident_type: "residency_breach".into(),
            severity: "high".into(),
            attachment_id: Some(attachment.id),
            summary: Some("Replicated into wrong region".into()),
            notes: json!({ "impact": "regional" }),
        }),
    )
    .await
    .unwrap()
    .0;

    let resolved = resolve_vector_db_incident(
        Extension(pool.clone()),
        auth(owner_id),
        Path((vector.id, incident.id)),
        Json(ResolveVectorDbIncident {
            resolution_summary: Some("Replica deleted".into()),
            resolution_notes: Some(json!({ "validated_by": "compliance" })),
        }),
    )
    .await
    .unwrap()
    .0;

    assert!(resolved.resolved_at.is_some());
    assert_eq!(resolved.summary.as_deref(), Some("Replica deleted"));
    assert_eq!(resolved.notes["validated_by"], "compliance");

    let err = resolve_vector_db_incident(
        Extension(pool.clone()),
        auth(owner_id),
        Path((vector.id, incident.id)),
        Json(ResolveVectorDbIncident {
            resolution_summary: None,
            resolution_notes: None,
        }),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err.0, StatusCode::CONFLICT);
}
