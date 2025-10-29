use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// key: provider-keys-model
/// Canonical provider key record surfaced by REST, CLI, and SSE contracts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderKeyRecord {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub alias: Option<String>,
    pub state: ProviderKeyState,
    pub rotation_due_at: Option<DateTime<Utc>>,
    pub attestation_digest: Option<String>,
    pub attestation_signature_registered: bool,
    pub attestation_verified_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub retired_at: Option<DateTime<Utc>>,
    pub compromised_at: Option<DateTime<Utc>>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKeyState {
    PendingRegistration,
    Active,
    Rotating,
    Retired,
    Compromised,
}

impl ProviderKeyState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKeyState::PendingRegistration => "pending_registration",
            ProviderKeyState::Active => "active",
            ProviderKeyState::Rotating => "rotating",
            ProviderKeyState::Retired => "retired",
            ProviderKeyState::Compromised => "compromised",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "active" => ProviderKeyState::Active,
            "rotating" => ProviderKeyState::Rotating,
            "retired" => ProviderKeyState::Retired,
            "compromised" => ProviderKeyState::Compromised,
            _ => ProviderKeyState::PendingRegistration,
        }
    }
}

/// key: provider-keys-binding-scope
/// Records the logical attachment for a key version (artifact, workspace, runtime decision, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderKeyBindingScope {
    pub binding_type: String,
    pub binding_target_id: Uuid,
    pub additional_context: Value,
}

/// key: provider-keys-binding-record
/// Durable binding record persisted for each attached scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderKeyBindingRecord {
    pub id: Uuid,
    pub provider_key_id: Uuid,
    pub binding_type: String,
    pub binding_target_id: Uuid,
    pub binding_scope: Value,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_reason: Option<String>,
    pub version: i64,
}

/// key: provider-keys-tier-requirement
/// Declares BYOK requirements for a runtime policy tier.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderTierRequirement {
    pub tier: String,
    pub provider_id: Uuid,
    pub byok_required: bool,
}

/// key: provider-keys-decision-posture
/// Serialized BYOK posture persisted alongside runtime policy decisions.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProviderKeyDecisionPosture {
    pub provider_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_key_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ProviderKeyState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_due_at: Option<DateTime<Utc>>,
    pub attestation_registered: bool,
    pub attestation_signature_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_verified_at: Option<DateTime<Utc>>,
    pub vetoed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// key: provider-keys-rotation-record
/// Rotation lifecycle entry persisted for each provider key rotation request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderKeyRotationRecord {
    pub id: Uuid,
    pub provider_key_id: Uuid,
    pub requested_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub status: ProviderKeyRotationStatus,
    pub evidence_uri: Option<String>,
    pub request_actor_ref: Option<String>,
    pub approval_actor_ref: Option<String>,
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_digest: Option<String>,
    pub attestation_signature_verified: bool,
    pub metadata: Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKeyRotationStatus {
    PendingApproval,
    Approved,
    Failed,
}
