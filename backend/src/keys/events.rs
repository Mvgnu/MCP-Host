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
    RotationSlaWarning,
    RotationSlaBreached,
    Compromised,
    Retired,
    RevocationInitiated,
    RevocationCompleted,
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
            ProviderKeyAuditEventType::RotationSlaWarning => "rotation_sla_warning",
            ProviderKeyAuditEventType::RotationSlaBreached => "rotation_sla_breached",
            ProviderKeyAuditEventType::Compromised => "compromised",
            ProviderKeyAuditEventType::Retired => "retired",
            ProviderKeyAuditEventType::RevocationInitiated => "revocation_initiated",
            ProviderKeyAuditEventType::RevocationCompleted => "revocation_completed",
            ProviderKeyAuditEventType::BindingAttached => "binding_attached",
            ProviderKeyAuditEventType::BindingRevoked => "binding_revoked",
            ProviderKeyAuditEventType::RuntimeVeto => "runtime_veto",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "registered" => Some(Self::Registered),
            "activation_approved" => Some(Self::ActivationApproved),
            "rotation_requested" => Some(Self::RotationRequested),
            "rotation_approved" => Some(Self::RotationApproved),
            "rotation_failed" => Some(Self::RotationFailed),
            "rotation_sla_warning" => Some(Self::RotationSlaWarning),
            "rotation_sla_breached" => Some(Self::RotationSlaBreached),
            "compromised" => Some(Self::Compromised),
            "retired" => Some(Self::Retired),
            "revocation_initiated" => Some(Self::RevocationInitiated),
            "revocation_completed" => Some(Self::RevocationCompleted),
            "binding_attached" => Some(Self::BindingAttached),
            "binding_revoked" => Some(Self::BindingRevoked),
            "runtime_veto" => Some(Self::RuntimeVeto),
            _ => None,
        }
    }
}
