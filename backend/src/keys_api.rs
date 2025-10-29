use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use base64::DecodeError;
use chrono::{DateTime, Utc};

use crate::keys::{
    ProviderKeyRecord, ProviderKeyRotationRecord, ProviderKeyService, ProviderKeyServiceConfig,
    RegisterProviderKey, RequestKeyRotation,
};

/// key: provider-keys-api
/// Placeholder HTTP handlers for provider BYOK lifecycle.
pub fn routes() -> Router {
    Router::new()
        .route(
            "/api/providers/:provider_id/keys",
            get(list_keys).post(register_key),
        )
        .route(
            "/api/providers/:provider_id/keys/:key_id/rotations",
            post(request_rotation),
        )
}

async fn register_key(
    Extension(pool): Extension<PgPool>,
    Path(provider_id): Path<Uuid>,
    Json(payload): Json<RegisterKeyRequest>,
) -> Result<Json<ProviderKeyRecord>, StatusCode> {
    let service = ProviderKeyService::new(pool, ProviderKeyServiceConfig::default());
    let rotation_due_at = match payload.rotation_due_at {
        Some(ref value) => Some(parse_rotation_due(value)?),
        None => None,
    };

    let request = RegisterProviderKey {
        alias: payload.alias,
        attestation_digest: payload.attestation_digest,
        attestation_signature: payload.attestation_signature,
        rotation_due_at,
    };

    let record = service
        .register_key(provider_id, request)
        .await
        .map_err(|err| {
            if err.downcast_ref::<DecodeError>().is_some()
                || err.to_string().contains("attestation")
                || err.to_string().contains("signature")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::NOT_IMPLEMENTED
            }
        })?;
    Ok(Json(record))
}

async fn list_keys(
    Extension(pool): Extension<PgPool>,
    Path(provider_id): Path<Uuid>,
) -> Result<Json<Vec<ProviderKeyRecord>>, StatusCode> {
    let service = ProviderKeyService::new(pool, ProviderKeyServiceConfig::default());
    let keys = service
        .list_keys(provider_id)
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;
    Ok(Json(keys))
}

async fn request_rotation(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, key_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<RequestRotation>,
) -> Result<Json<ProviderKeyRotationRecord>, StatusCode> {
    let service = ProviderKeyService::new(pool, ProviderKeyServiceConfig::default());
    let request = RequestKeyRotation {
        attestation_digest: payload.attestation_digest,
        attestation_signature: payload.attestation_signature,
        request_actor_ref: payload.request_actor_ref,
    };

    let rotation = service
        .request_rotation(provider_id, key_id, request)
        .await
        .map_err(|err| {
            if err
                .downcast_ref::<DecodeError>()
                .is_some()
                || err.to_string().contains("attestation")
                || err.to_string().contains("signature")
                || err.to_string().contains("actor")
            {
                StatusCode::BAD_REQUEST
            } else if err.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else if err.to_string().contains("not active") {
                StatusCode::CONFLICT
            } else {
                StatusCode::NOT_IMPLEMENTED
            }
        })?;

    Ok(Json(rotation))
}

#[derive(Debug, Deserialize)]
pub struct RegisterKeyRequest {
    pub alias: Option<String>,
    pub attestation_digest: Option<String>,
    pub attestation_signature: Option<String>,
    pub rotation_due_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RequestRotation {
    pub attestation_digest: Option<String>,
    pub attestation_signature: Option<String>,
    pub request_actor_ref: Option<String>,
}

fn parse_rotation_due(input: &str) -> Result<DateTime<Utc>, StatusCode> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| StatusCode::BAD_REQUEST)
}
