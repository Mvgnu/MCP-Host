use anyhow::{anyhow, bail, Context};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::events::{ProviderKeyAuditEvent, ProviderKeyAuditEventType};
use super::models::{
    ProviderKeyBindingScope, ProviderKeyRecord, ProviderKeyRotationRecord,
    ProviderKeyRotationStatus, ProviderKeyState, ProviderTierRequirement,
};
use super::policy::ProviderKeyPolicySummary;

/// key: provider-keys-service
/// Entry point for BYOK operations across registration, rotation, and runtime policy integration.
#[derive(Clone)]
pub struct ProviderKeyService {
    pool: PgPool,
    config: ProviderKeyServiceConfig,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderKeyServiceConfig {
    pub notify_channel: String,
    pub feature_flag: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RegisterProviderKey {
    pub alias: Option<String>,
    pub attestation_digest: Option<String>,
    pub attestation_signature: Option<String>,
    pub rotation_due_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestKeyRotation {
    pub attestation_digest: Option<String>,
    pub attestation_signature: Option<String>,
    pub request_actor_ref: Option<String>,
}

#[async_trait]
pub trait ProviderKeyStore: Send + Sync {
    async fn insert_key(&self, record: &ProviderKeyRecord) -> sqlx::Result<()>;
    async fn list_keys(&self, provider_id: Uuid) -> sqlx::Result<Vec<ProviderKeyRecord>>;
    async fn append_audit_event(&self, event: &ProviderKeyAuditEvent) -> sqlx::Result<()>;
}

impl ProviderKeyService {
    pub fn new(pool: PgPool, config: ProviderKeyServiceConfig) -> Self {
        Self { pool, config }
    }

    pub fn config(&self) -> &ProviderKeyServiceConfig {
        &self.config
    }

    pub async fn register_key(
        &self,
        provider_id: Uuid,
        request: RegisterProviderKey,
    ) -> anyhow::Result<ProviderKeyRecord> {
        let now = chrono::Utc::now();
        let attestation_digest = request
            .attestation_digest
            .ok_or_else(|| anyhow!("attestation digest required"))?;
        let attestation_signature = request
            .attestation_signature
            .ok_or_else(|| anyhow!("attestation signature required"))?;

        // Ensure both digest and signature are valid base64 payloads before persisting.
        base64::decode(&attestation_digest).context("invalid attestation digest encoding")?;
        base64::decode(&attestation_signature).context("invalid attestation signature encoding")?;

        let state = ProviderKeyState::Active;
        let activated_at = Some(now);
        let attestation_verified_at = Some(now);

        let record = ProviderKeyRecord {
            id: Uuid::new_v4(),
            provider_id,
            alias: request.alias.clone(),
            state,
            rotation_due_at: request.rotation_due_at,
            attestation_digest: Some(attestation_digest.clone()),
            attestation_signature_registered: true,
            attestation_verified_at,
            activated_at,
            retired_at: None,
            compromised_at: None,
            version: 0,
            created_at: now,
            updated_at: now,
        };

        let event = ProviderKeyAuditEvent {
            id: Uuid::new_v4(),
            provider_id,
            provider_key_id: Some(record.id),
            event_type: ProviderKeyAuditEventType::Registered,
            payload: json!({
                "alias": record.alias,
                "rotation_due_at": record.rotation_due_at,
                "attestation_registered": true,
                "attestation_signature_registered": true,
                "attestation_verified_at": attestation_verified_at,
            }),
            occurred_at: now,
        };

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO provider_keys(id, provider_id, alias, state, rotation_due_at, attestation_digest, attestation_signature, attestation_verified_at, activated_at, retired_at, compromised_at, version, created_at, updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
        )
        .bind(record.id)
        .bind(record.provider_id)
        .bind(&record.alias)
        .bind(match record.state {
            ProviderKeyState::PendingRegistration => "pending_registration",
            ProviderKeyState::Active => "active",
            ProviderKeyState::Rotating => "rotating",
            ProviderKeyState::Retired => "retired",
            ProviderKeyState::Compromised => "compromised",
        })
        .bind(record.rotation_due_at)
        .bind(&record.attestation_digest)
        .bind(attestation_signature)
        .bind(attestation_verified_at)
        .bind(record.activated_at)
        .bind(record.retired_at)
        .bind(record.compromised_at)
        .bind(record.version)
        .bind(record.created_at)
        .bind(record.updated_at)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO provider_key_audit_events(id, provider_id, provider_key_id, event_type, payload, occurred_at) VALUES($1,$2,$3,$4,$5,$6)"
        )
        .bind(event.id)
        .bind(event.provider_id)
        .bind(event.provider_key_id)
        .bind(event.event_type.as_str())
        .bind(event.payload)
        .bind(event.occurred_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(record)
    }

    pub async fn list_keys(&self, provider_id: Uuid) -> sqlx::Result<Vec<ProviderKeyRecord>> {
        let rows = sqlx::query_as::<_, ProviderKeyRow>(
            r#"SELECT id, provider_id, alias, state, rotation_due_at, attestation_digest, attestation_signature IS NOT NULL AS attestation_signature_registered, attestation_verified_at, activated_at, retired_at, compromised_at, version, created_at, updated_at FROM provider_keys WHERE provider_id = $1 ORDER BY created_at DESC"#,
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(ProviderKeyRecord::from).collect())
    }

    pub async fn summarize_for_policy(
        &self,
        provider_id: Uuid,
    ) -> sqlx::Result<ProviderKeyPolicySummary> {
        let mut summary = ProviderKeyPolicySummary::default();
        let keys = self.list_keys(provider_id).await?;
        if keys.is_empty() {
            summary.add_veto_note("missing");
            return Ok(summary);
        }
        if let Some(active) = keys
            .iter()
            .find(|record| matches!(record.state, ProviderKeyState::Active))
            .cloned()
        {
            if !active.attestation_signature_registered {
                summary.add_veto_note("attestation-signature-missing");
            }
            if let Some(due) = active.rotation_due_at {
                if due < chrono::Utc::now() {
                    summary.add_veto_note("rotation-overdue");
                }
            }
            if active.attestation_verified_at.is_none() {
                summary.add_veto_note("attestation-unverified");
            }
            summary.record = Some(active);
        } else if let Some(pending) = keys.first().cloned() {
            summary.record = Some(pending);
            summary.add_veto_note("not-active");
        }

        Ok(summary)
    }

    pub async fn tier_requirement(
        &self,
        tier: &str,
    ) -> sqlx::Result<Option<ProviderTierRequirement>> {
        let requirement = sqlx::query_as::<_, ProviderTierRow>(
            r#"
            SELECT tier, provider_id, byok_required
            FROM provider_tiers
            WHERE tier = $1
            "#,
        )
        .bind(tier)
        .fetch_optional(&self.pool)
        .await?;

        Ok(requirement.map(|row| ProviderTierRequirement {
            tier: row.tier,
            provider_id: row.provider_id,
            byok_required: row.byok_required,
        }))
    }

    pub async fn request_rotation(
        &self,
        provider_id: Uuid,
        key_id: Uuid,
        request: RequestKeyRotation,
    ) -> anyhow::Result<ProviderKeyRotationRecord> {
        let attestation_digest = request
            .attestation_digest
            .ok_or_else(|| anyhow!("rotation attestation digest required"))?;
        let attestation_signature = request
            .attestation_signature
            .ok_or_else(|| anyhow!("rotation attestation signature required"))?;
        let actor_ref = request
            .request_actor_ref
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("rotation actor reference required"))?
            .to_string();

        base64::decode(&attestation_digest).context("invalid attestation digest encoding")?;
        base64::decode(&attestation_signature).context("invalid attestation signature encoding")?;

        let now = chrono::Utc::now();
        let metadata = json!({
            "attestation_digest": attestation_digest.clone(),
            "attestation_signature_registered": true,
        });

        let mut tx = self.pool.begin().await?;
        let key_row = sqlx::query_as::<_, ProviderKeyRow>(
            r#"SELECT id, provider_id, alias, state, rotation_due_at, attestation_digest, attestation_signature IS NOT NULL AS attestation_signature_registered, attestation_verified_at, activated_at, retired_at, compromised_at, version, created_at, updated_at FROM provider_keys WHERE id = $1 AND provider_id = $2 FOR UPDATE"#,
        )
        .bind(key_id)
        .bind(provider_id)
        .fetch_optional(&mut *tx)
        .await?;

        let mut key =
            key_row.ok_or_else(|| anyhow!("provider key not found for rotation request"))?;

        match key.state.as_str() {
            "active" | "rotating" => {}
            "pending_registration" => {
                bail!("provider key not active; cannot request rotation")
            }
            "retired" => bail!("provider key retired"),
            "compromised" => bail!("provider key compromised"),
            _ => bail!("provider key state invalid for rotation"),
        }

        let rotation_id = Uuid::new_v4();
        let rotation = ProviderKeyRotationRecord {
            id: rotation_id,
            provider_key_id: key.id,
            requested_at: now,
            approved_at: None,
            status: ProviderKeyRotationStatus::PendingApproval,
            evidence_uri: None,
            request_actor_ref: Some(actor_ref.clone()),
            approval_actor_ref: None,
            failure_reason: None,
            attestation_digest: Some(attestation_digest.clone()),
            attestation_signature_verified: true,
            metadata: metadata.clone(),
        };

        sqlx::query(
            "INSERT INTO provider_key_rotations(id, provider_key_id, requested_at, approved_at, status, evidence_uri, request_actor_ref, approval_actor_ref, failure_reason, metadata, attestation_digest, attestation_signature) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(rotation.id)
        .bind(rotation.provider_key_id)
        .bind(rotation.requested_at)
        .bind(rotation.approved_at)
        .bind("pending_approval")
        .bind(rotation.evidence_uri)
        .bind(actor_ref)
        .bind(rotation.approval_actor_ref.clone())
        .bind(rotation.failure_reason.clone())
        .bind(metadata)
        .bind(attestation_digest)
        .bind(attestation_signature)
        .execute(&mut *tx)
        .await?;

        let previous_state = key.state.clone();

        if previous_state != "rotating" {
            sqlx::query(
                "UPDATE provider_keys SET state = $1, updated_at = $2, version = version + 1 WHERE id = $3",
            )
            .bind("rotating")
            .bind(now)
            .bind(key.id)
            .execute(&mut *tx)
            .await?;
        }

        let event = ProviderKeyAuditEvent {
            id: Uuid::new_v4(),
            provider_id,
            provider_key_id: Some(key.id),
            event_type: ProviderKeyAuditEventType::RotationRequested,
            payload: json!({
                "rotation_id": rotation.id,
                "previous_state": previous_state,
                "request_actor_ref": rotation.request_actor_ref,
                "attestation_registered": true,
            }),
            occurred_at: now,
        };

        sqlx::query(
            "INSERT INTO provider_key_audit_events(id, provider_id, provider_key_id, event_type, payload, occurred_at) VALUES($1,$2,$3,$4,$5,$6)",
        )
        .bind(event.id)
        .bind(event.provider_id)
        .bind(event.provider_key_id)
        .bind(event.event_type.as_str())
        .bind(event.payload)
        .bind(event.occurred_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(rotation)
    }

    pub async fn record_runtime_veto(
        &self,
        provider_id: Uuid,
        provider_key_id: Option<Uuid>,
        notes: Vec<String>,
    ) -> anyhow::Result<()> {
        let now = chrono::Utc::now();
        let event = ProviderKeyAuditEvent {
            id: Uuid::new_v4(),
            provider_id,
            provider_key_id,
            event_type: ProviderKeyAuditEventType::RuntimeVeto,
            payload: json!({
                "notes": notes,
                "recorded_at": now,
            }),
            occurred_at: now,
        };

        sqlx::query(
            "INSERT INTO provider_key_audit_events(id, provider_id, provider_key_id, event_type, payload, occurred_at) VALUES($1,$2,$3,$4,$5,$6)",
        )
        .bind(event.id)
        .bind(event.provider_id)
        .bind(event.provider_key_id)
        .bind(event.event_type.as_str())
        .bind(&event.payload)
        .bind(event.occurred_at)
        .execute(&self.pool)
        .await?;

        if !self.config.notify_channel.is_empty() {
            let payload = serde_json::to_string(&event)?;
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(&self.config.notify_channel)
                .bind(payload)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    #[allow(unused_variables)]
    pub async fn record_binding(
        &self,
        key_id: Uuid,
        scope: ProviderKeyBindingScope,
    ) -> anyhow::Result<()> {
        // TODO: Implement binding persistence and optimistic locking
        let _ = (key_id, scope);
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ProviderKeyRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub alias: Option<String>,
    pub state: String,
    pub rotation_due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub attestation_digest: Option<String>,
    pub attestation_signature_registered: bool,
    pub attestation_verified_at: Option<chrono::DateTime<chrono::Utc>>,
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub retired_at: Option<chrono::DateTime<chrono::Utc>>,
    pub compromised_at: Option<chrono::DateTime<chrono::Utc>>,
    pub version: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<ProviderKeyRow> for ProviderKeyRecord {
    fn from(row: ProviderKeyRow) -> Self {
        Self {
            id: row.id,
            provider_id: row.provider_id,
            alias: row.alias,
            state: match row.state.as_str() {
                "active" => super::models::ProviderKeyState::Active,
                "rotating" => super::models::ProviderKeyState::Rotating,
                "retired" => super::models::ProviderKeyState::Retired,
                "compromised" => super::models::ProviderKeyState::Compromised,
                _ => super::models::ProviderKeyState::PendingRegistration,
            },
            rotation_due_at: row.rotation_due_at,
            attestation_digest: row.attestation_digest,
            attestation_signature_registered: row.attestation_signature_registered,
            attestation_verified_at: row.attestation_verified_at,
            activated_at: row.activated_at,
            retired_at: row.retired_at,
            compromised_at: row.compromised_at,
            version: row.version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ProviderKeyRotationRow {
    pub id: Uuid,
    pub provider_key_id: Uuid,
    pub requested_at: chrono::DateTime<chrono::Utc>,
    pub approved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String,
    pub evidence_uri: Option<String>,
    pub request_actor_ref: Option<String>,
    pub approval_actor_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub attestation_digest: Option<String>,
    pub attestation_signature_verified: bool,
    pub metadata: Value,
}

impl From<ProviderKeyRotationRow> for ProviderKeyRotationRecord {
    fn from(row: ProviderKeyRotationRow) -> Self {
        Self {
            id: row.id,
            provider_key_id: row.provider_key_id,
            requested_at: row.requested_at,
            approved_at: row.approved_at,
            status: match row.status.as_str() {
                "approved" => ProviderKeyRotationStatus::Approved,
                "failed" => ProviderKeyRotationStatus::Failed,
                _ => ProviderKeyRotationStatus::PendingApproval,
            },
            evidence_uri: row.evidence_uri,
            request_actor_ref: row.request_actor_ref,
            approval_actor_ref: row.approval_actor_ref,
            failure_reason: row.failure_reason,
            attestation_digest: row.attestation_digest,
            attestation_signature_verified: row.attestation_signature_verified,
            metadata: row.metadata,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ProviderTierRow {
    pub tier: String,
    pub provider_id: Uuid,
    pub byok_required: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[tokio::test]
    async fn record_runtime_veto_persists_audit_event() -> anyhow::Result<()> {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping record_runtime_veto_persists_audit_event: DATABASE_URL not set",
                );
                return Ok(());
            }
        };

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
        sqlx::migrate!("../backend/migrations").run(&pool).await?;

        let provider_id = Uuid::new_v4();
        let service = ProviderKeyService::new(pool.clone(), ProviderKeyServiceConfig::default());

        let digest = base64::encode(b"runtime-veto-test");
        let record = service
            .register_key(
                provider_id,
                RegisterProviderKey {
                    alias: Some("primary".to_string()),
                    attestation_digest: Some(digest),
                    rotation_due_at: None,
                },
            )
            .await?;

        service
            .record_runtime_veto(provider_id, Some(record.id), vec!["missing".to_string()])
            .await?;

        let row = sqlx::query(
            "SELECT event_type, payload FROM provider_key_audit_events WHERE provider_id = $1 AND event_type = 'runtime_veto' ORDER BY occurred_at DESC LIMIT 1",
        )
        .bind(provider_id)
        .fetch_one(&pool)
        .await?;

        let event_type: String = row.get("event_type");
        let payload: serde_json::Value = row.get("payload");
        assert_eq!(event_type, "runtime_veto");
        let notes = payload
            .get("notes")
            .and_then(|value| value.as_array())
            .expect("notes array missing from payload");
        assert!(notes.iter().any(|value| value.as_str() == Some("missing")));

        Ok(())
    }
}
