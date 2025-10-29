use crate::keys::models::{ProviderKeyRecord, ProviderKeyState};

/// key: provider-keys-policy
/// Policy helpers for integrating BYOK posture into runtime decisions.
#[derive(Debug, Default)]
pub struct ProviderKeyPolicySummary {
    pub record: Option<ProviderKeyRecord>,
    pub notes: Vec<String>,
    pub vetoed: bool,
}

impl ProviderKeyPolicySummary {
    pub fn add_note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    pub fn add_veto_note(&mut self, note: impl Into<String>) {
        self.vetoed = true;
        self.notes.push(note.into());
    }

    pub fn posture_state(&self) -> Option<ProviderKeyState> {
        self.record.as_ref().map(|record| record.state)
    }
}
