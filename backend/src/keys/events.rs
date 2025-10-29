use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// key: provider-keys-audit-event
/// Durable audit event envelope emitted whenever a provider key transitions state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderKeyAuditEvent {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub provider_key_id: Option<Uuid>,
    pub event_type: ProviderKeyAuditEventType,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKeyAuditEventType {
    Registered,
    ActivationApproved,
    RotationRequested,
    RotationApproved,
    RotationFailed,
    Compromised,
    Retired,
    BindingAttached,
    BindingRevoked,
    RuntimeVeto,
}

impl ProviderKeyAuditEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKeyAuditEventType::Registered => "registered",
            ProviderKeyAuditEventType::ActivationApproved => "activation_approved",
            ProviderKeyAuditEventType::RotationRequested => "rotation_requested",
            ProviderKeyAuditEventType::RotationApproved => "rotation_approved",
            ProviderKeyAuditEventType::RotationFailed => "rotation_failed",
            ProviderKeyAuditEventType::Compromised => "compromised",
            ProviderKeyAuditEventType::Retired => "retired",
            ProviderKeyAuditEventType::BindingAttached => "binding_attached",
            ProviderKeyAuditEventType::BindingRevoked => "binding_revoked",
            ProviderKeyAuditEventType::RuntimeVeto => "runtime_veto",
        }
    }
}
