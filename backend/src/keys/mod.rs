//! Provider BYOK key management scaffolding.

pub mod events;
pub mod models;
pub mod policy;
pub mod service;

pub use events::ProviderKeyAuditEvent;
pub use models::{
    ProviderKeyBindingScope, ProviderKeyDecisionPosture, ProviderKeyRecord,
    ProviderKeyRotationRecord, ProviderKeyRotationStatus, ProviderKeyState,
    ProviderTierRequirement,
};
pub use policy::ProviderKeyPolicySummary;
pub use service::{
    ProviderKeyService, ProviderKeyServiceConfig, RegisterProviderKey, RequestKeyRotation,
};
