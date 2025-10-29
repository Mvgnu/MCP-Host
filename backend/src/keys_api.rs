use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use base64::DecodeError;
use chrono::{DateTime, Utc};

use crate::audit::{query_provider_key_events, ProviderKeyAuditFilter, ProviderScopedAuditLog};
use crate::keys::{
    ProviderKeyBindingRecord, ProviderKeyBindingScope, ProviderKeyRecord,
    ProviderKeyRotationRecord, ProviderKeyService, ProviderKeyServiceConfig, ProviderKeyState,
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
        .route(
            "/api/providers/:provider_id/keys/:key_id/revocations",
            post(revoke_key),
        )
        .route(
            "/api/providers/:provider_id/keys/:key_id/bindings",
            get(list_bindings).post(create_binding),
        )
        .route(
            "/api/providers/:provider_id/keys/audit",
            get(list_audit_events),
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
            if err.downcast_ref::<DecodeError>().is_some()
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

async fn revoke_key(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, key_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<RevokeKeyRequest>,
) -> Result<Json<ProviderKeyRecord>, StatusCode> {
    let service = ProviderKeyService::new(pool, ProviderKeyServiceConfig::default());
    let reason = payload.reason.clone();
    let record = service
        .revoke_key(provider_id, key_id, reason, payload.mark_compromised)
        .await
        .map_err(|err| {
            if err.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else if err.to_string().contains("provider mismatch") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::NOT_IMPLEMENTED
            }
        })?;

    Ok(Json(record))
}

async fn list_audit_events(
    Extension(pool): Extension<PgPool>,
    Path(provider_id): Path<Uuid>,
    Query(params): Query<AuditQueryParams>,
) -> Result<Json<Vec<ProviderScopedAuditLog>>, StatusCode> {
    let filter = ProviderKeyAuditFilter {
        provider_id,
        provider_key_id: params.key_id,
        state: params.state,
        start: params.start,
        end: params.end,
        limit: params.limit.or(Some(100)),
    };

    let events = query_provider_key_events(&pool, filter)
        .await
        .map_err(|_| StatusCode::NOT_IMPLEMENTED)?;

    Ok(Json(events))
}

async fn create_binding(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, key_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<RecordBindingRequest>,
) -> Result<Json<ProviderKeyBindingRecord>, StatusCode> {
    let service = ProviderKeyService::new(pool, ProviderKeyServiceConfig::default());
    let scope = ProviderKeyBindingScope {
        binding_type: payload.binding_type,
        binding_target_id: payload.binding_target_id,
        additional_context: payload.additional_context.unwrap_or_else(|| json!({})),
    };

    let record = service
        .record_binding(provider_id, key_id, scope)
        .await
        .map_err(|err| {
            if err.to_string().contains("binding type") {
                StatusCode::BAD_REQUEST
            } else if err.to_string().contains("already exists") {
                StatusCode::CONFLICT
            } else if err.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else if err.to_string().contains("provider mismatch") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::NOT_IMPLEMENTED
            }
        })?;

    Ok(Json(record))
}

async fn list_bindings(
    Extension(pool): Extension<PgPool>,
    Path((provider_id, key_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ProviderKeyBindingRecord>>, StatusCode> {
    let service = ProviderKeyService::new(pool, ProviderKeyServiceConfig::default());
    let bindings = service
        .list_bindings(provider_id, key_id)
        .await
        .map_err(|err| {
            if err.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else if err.to_string().contains("provider mismatch") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::NOT_IMPLEMENTED
            }
        })?;

    Ok(Json(bindings))
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

#[derive(Debug, Deserialize)]
pub struct RevokeKeyRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub mark_compromised: bool,
}

#[derive(Debug, Deserialize)]
pub struct AuditQueryParams {
    #[serde(default)]
    pub key_id: Option<Uuid>,
    #[serde(default)]
    pub state: Option<ProviderKeyState>,
    #[serde(default)]
    pub start: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RecordBindingRequest {
    pub binding_type: String,
    pub binding_target_id: Uuid,
    #[serde(default)]
    pub additional_context: Option<serde_json::Value>,
}

fn parse_rotation_due(input: &str) -> Result<DateTime<Utc>, StatusCode> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| StatusCode::BAD_REQUEST)
}
