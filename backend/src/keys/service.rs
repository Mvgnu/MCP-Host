use anyhow::{anyhow, bail, Context};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::events::{ProviderKeyAuditEvent, ProviderKeyAuditEventType};
use super::models::{
    ProviderKeyBindingRecord, ProviderKeyBindingScope, ProviderKeyRecord,
    ProviderKeyRotationRecord, ProviderKeyRotationStatus, ProviderKeyState,
    ProviderTierRequirement,
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

#[derive(Clone, Debug)]
pub struct RotationSlaSnapshot {
    pub provider_id: Uuid,
    pub provider_key_id: Uuid,
    pub rotation_due_at: DateTime<Utc>,
    pub state: ProviderKeyState,
    pub event_emitted: bool,
}

#[derive(Clone, Debug)]
pub struct RotationSlaReport {
    pub evaluated_at: DateTime<Utc>,
    pub approaching: Vec<RotationSlaSnapshot>,
    pub breached: Vec<RotationSlaSnapshot>,
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
        STANDARD
            .decode(&attestation_digest)
            .context("invalid attestation digest encoding")?;
        STANDARD
            .decode(&attestation_signature)
            .context("invalid attestation signature encoding")?;

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
                } else if due <= chrono::Utc::now() + Duration::hours(24) {
                    summary.add_veto_note("rotation-approaching");
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

        STANDARD
            .decode(&attestation_digest)
            .context("invalid attestation digest encoding")?;
        STANDARD
            .decode(&attestation_signature)
            .context("invalid attestation signature encoding")?;

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
        .bind(rotation.evidence_uri.clone())
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

    pub async fn enforce_rotation_slas(
        &self,
        warning_window: Duration,
        dedupe_interval: Duration,
    ) -> anyhow::Result<RotationSlaReport> {
        let now = Utc::now();
        let rows = sqlx::query_as::<_, ProviderKeyRow>(
            r#"SELECT id, provider_id, alias, state, rotation_due_at, attestation_digest, attestation_signature IS NOT NULL AS attestation_signature_registered, attestation_verified_at, activated_at, retired_at, compromised_at, version, created_at, updated_at FROM provider_keys WHERE rotation_due_at IS NOT NULL AND state IN ('active','rotating')"#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut report = RotationSlaReport {
            evaluated_at: now,
            approaching: Vec::new(),
            breached: Vec::new(),
        };

        for record in rows.into_iter().map(ProviderKeyRecord::from) {
            let due = match record.rotation_due_at {
                Some(value) => value,
                None => continue,
            };
            let mut snapshot = RotationSlaSnapshot {
                provider_id: record.provider_id,
                provider_key_id: record.id,
                rotation_due_at: due,
                state: record.state,
                event_emitted: false,
            };

            if due <= now {
                let event = ProviderKeyAuditEvent {
                    id: Uuid::new_v4(),
                    provider_id: record.provider_id,
                    provider_key_id: Some(record.id),
                    event_type: ProviderKeyAuditEventType::RotationSlaBreached,
                    payload: json!({
                        "rotation_due_at": due,
                        "state": record.state.as_str(),
                        "evaluated_at": now,
                    }),
                    occurred_at: now,
                };
                snapshot.event_emitted = self
                    .insert_audit_event_if_absent(&event, dedupe_interval)
                    .await?;
                report.breached.push(snapshot);
            } else if due <= now + warning_window {
                let event = ProviderKeyAuditEvent {
                    id: Uuid::new_v4(),
                    provider_id: record.provider_id,
                    provider_key_id: Some(record.id),
                    event_type: ProviderKeyAuditEventType::RotationSlaWarning,
                    payload: json!({
                        "rotation_due_at": due,
                        "state": record.state.as_str(),
                        "warning_window_seconds": warning_window.num_seconds(),
                        "evaluated_at": now,
                    }),
                    occurred_at: now,
                };
                snapshot.event_emitted = self
                    .insert_audit_event_if_absent(&event, dedupe_interval)
                    .await?;
                report.approaching.push(snapshot);
            }
        }

        Ok(report)
    }

    pub async fn revoke_key(
        &self,
        provider_id: Uuid,
        key_id: Uuid,
        reason: Option<String>,
        mark_compromised: bool,
    ) -> anyhow::Result<ProviderKeyRecord> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let key_row = sqlx::query_as::<_, ProviderKeyRow>(
            r#"SELECT id, provider_id, alias, state, rotation_due_at, attestation_digest, attestation_signature IS NOT NULL AS attestation_signature_registered, attestation_verified_at, activated_at, retired_at, compromised_at, version, created_at, updated_at FROM provider_keys WHERE id = $1 FOR UPDATE"#,
        )
        .bind(key_id)
        .fetch_optional(&mut *tx)
        .await?;

        let mut key = key_row.ok_or_else(|| anyhow!("provider key not found for revocation"))?;
        if key.provider_id != provider_id {
            bail!("provider mismatch");
        }

        let previous_state = key.state.clone();
        let target_state = if mark_compromised {
            "compromised"
        } else {
            "retired"
        };

        sqlx::query(
            "UPDATE provider_keys SET state = $1, retired_at = $2, compromised_at = CASE WHEN $1 = 'compromised' THEN $2 ELSE compromised_at END, updated_at = $2, version = version + 1 WHERE id = $3"
        )
        .bind(target_state)
        .bind(now)
        .bind(key.id)
        .execute(&mut *tx)
        .await?;

        let initiated_reason = reason.clone();
        let initiated = ProviderKeyAuditEvent {
            id: Uuid::new_v4(),
            provider_id,
            provider_key_id: Some(key.id),
            event_type: ProviderKeyAuditEventType::RevocationInitiated,
            payload: json!({
                "previous_state": previous_state,
                "reason": initiated_reason,
                "mark_compromised": mark_compromised,
            }),
            occurred_at: now,
        };
        sqlx::query(
            "INSERT INTO provider_key_audit_events(id, provider_id, provider_key_id, event_type, payload, occurred_at) VALUES($1,$2,$3,$4,$5,$6)"
        )
        .bind(initiated.id)
        .bind(initiated.provider_id)
        .bind(initiated.provider_key_id)
        .bind(initiated.event_type.as_str())
        .bind(&initiated.payload)
        .bind(initiated.occurred_at)
        .execute(&mut *tx)
        .await?;

        let completed = ProviderKeyAuditEvent {
            id: Uuid::new_v4(),
            provider_id,
            provider_key_id: Some(key.id),
            event_type: ProviderKeyAuditEventType::RevocationCompleted,
            payload: json!({
                "final_state": target_state,
                "reason": reason,
            }),
            occurred_at: now,
        };
        sqlx::query(
            "INSERT INTO provider_key_audit_events(id, provider_id, provider_key_id, event_type, payload, occurred_at) VALUES($1,$2,$3,$4,$5,$6)"
        )
        .bind(completed.id)
        .bind(completed.provider_id)
        .bind(completed.provider_key_id)
        .bind(completed.event_type.as_str())
        .bind(&completed.payload)
        .bind(completed.occurred_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        self.dispatch_notification(&initiated).await?;
        self.dispatch_notification(&completed).await?;

        let mut updated: ProviderKeyRecord = key.into();
        updated.state = if mark_compromised {
            ProviderKeyState::Compromised
        } else {
            ProviderKeyState::Retired
        };
        updated.retired_at = Some(now);
        if mark_compromised {
            updated.compromised_at = Some(now);
        }
        updated.updated_at = now;

        Ok(updated)
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

        self.dispatch_notification(&event).await?;

        Ok(())
    }

    pub async fn record_binding(
        &self,
        provider_id: Uuid,
        key_id: Uuid,
        scope: ProviderKeyBindingScope,
    ) -> anyhow::Result<ProviderKeyBindingRecord> {
        if scope.binding_type.trim().is_empty() {
            bail!("binding type required");
        }

        let mut tx = self.pool.begin().await?;

        let owner: Option<Uuid> =
            sqlx::query_scalar("SELECT provider_id FROM provider_keys WHERE id = $1 FOR UPDATE")
                .bind(key_id)
                .fetch_optional(&mut *tx)
                .await?;

        let owner = owner.ok_or_else(|| anyhow!("provider key not found"))?;
        if owner != provider_id {
            bail!("provider mismatch");
        }

        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM provider_key_bindings WHERE provider_key_id = $1 AND binding_type = $2 AND binding_target_id = $3 AND revoked_at IS NULL",
        )
        .bind(key_id)
        .bind(&scope.binding_type)
        .bind(scope.binding_target_id)
        .fetch_optional(&mut *tx)
        .await?;

        if existing.is_some() {
            bail!("binding already exists");
        }

        let now = chrono::Utc::now();
        let binding_id = Uuid::new_v4();
        let scope_payload = if scope.additional_context.is_null() {
            json!({})
        } else {
            scope.additional_context
        };

        let row = sqlx::query_as::<_, ProviderKeyBindingRow>(
            r#"INSERT INTO provider_key_bindings(id, provider_key_id, binding_type, binding_target_id, binding_scope, created_at, revoked_at, revoked_reason, version)
            VALUES($1,$2,$3,$4,$5,$6,NULL,NULL,0)
            RETURNING id, provider_key_id, binding_type, binding_target_id, binding_scope, created_at, revoked_at, revoked_reason, version"#,
        )
        .bind(binding_id)
        .bind(key_id)
        .bind(&scope.binding_type)
        .bind(scope.binding_target_id)
        .bind(&scope_payload)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;

        let record = ProviderKeyBindingRecord::from(row);

        let event = ProviderKeyAuditEvent {
            id: Uuid::new_v4(),
            provider_id,
            provider_key_id: Some(key_id),
            event_type: ProviderKeyAuditEventType::BindingAttached,
            payload: json!({
                "binding_type": record.binding_type,
                "binding_target_id": record.binding_target_id,
                "binding_scope": record.binding_scope,
                "created_at": record.created_at,
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
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        if !self.config.notify_channel.is_empty() {
            let payload = serde_json::to_string(&event)?;
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(&self.config.notify_channel)
                .bind(payload)
                .execute(&self.pool)
                .await?;
        }

        Ok(record)
    }

    pub async fn list_bindings(
        &self,
        provider_id: Uuid,
        key_id: Uuid,
    ) -> anyhow::Result<Vec<ProviderKeyBindingRecord>> {
        let owner: Option<Uuid> =
            sqlx::query_scalar("SELECT provider_id FROM provider_keys WHERE id = $1")
                .bind(key_id)
                .fetch_optional(&self.pool)
                .await?;

        let owner = owner.ok_or_else(|| anyhow!("provider key not found"))?;
        if owner != provider_id {
            bail!("provider mismatch");
        }

        let rows = sqlx::query_as::<_, ProviderKeyBindingRow>(
            r#"SELECT id, provider_key_id, binding_type, binding_target_id, binding_scope, created_at, revoked_at, revoked_reason, version
            FROM provider_key_bindings
            WHERE provider_key_id = $1
            ORDER BY created_at DESC"#,
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(ProviderKeyBindingRecord::from)
            .collect())
    }

    async fn insert_audit_event_if_absent(
        &self,
        event: &ProviderKeyAuditEvent,
        dedupe_interval: Duration,
    ) -> anyhow::Result<bool> {
        let key_id = event
            .provider_key_id
            .ok_or_else(|| anyhow!("provider key id required for deduplicated audit event"))?;
        let threshold = event.occurred_at - dedupe_interval;
        let result = sqlx::query(
            "WITH candidate AS (SELECT $1::uuid AS id, $2::uuid AS provider_id, $3::uuid AS provider_key_id, $4::text AS event_type, $5::jsonb AS payload, $6::timestamptz AS occurred_at) INSERT INTO provider_key_audit_events(id, provider_id, provider_key_id, event_type, payload, occurred_at) SELECT id, provider_id, provider_key_id, event_type, payload, occurred_at FROM candidate WHERE NOT EXISTS (SELECT 1 FROM provider_key_audit_events WHERE provider_key_id = $3 AND event_type = $4 AND occurred_at >= $7)"
        )
        .bind(event.id)
        .bind(event.provider_id)
        .bind(key_id)
        .bind(event.event_type.as_str())
        .bind(&event.payload)
        .bind(event.occurred_at)
        .bind(threshold)
        .execute(&self.pool)
        .await?;
        let inserted = result.rows_affected() > 0;
        if inserted {
            self.dispatch_notification(event).await?;
        }
        Ok(inserted)
    }

    async fn dispatch_notification(&self, event: &ProviderKeyAuditEvent) -> anyhow::Result<()> {
        if self.config.notify_channel.is_empty() {
            return Ok(());
        }
        let payload =
            serde_json::to_string(event).context("serialize audit event for notification")?;
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(&self.config.notify_channel)
            .bind(payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ProviderKeyBindingRow {
    pub id: Uuid,
    pub provider_key_id: Uuid,
    pub binding_type: String,
    pub binding_target_id: Uuid,
    pub binding_scope: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_reason: Option<String>,
    pub version: i64,
}

impl From<ProviderKeyBindingRow> for ProviderKeyBindingRecord {
    fn from(row: ProviderKeyBindingRow) -> Self {
        Self {
            id: row.id,
            provider_key_id: row.provider_key_id,
            binding_type: row.binding_type,
            binding_target_id: row.binding_target_id,
            binding_scope: row.binding_scope,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
            revoked_reason: row.revoked_reason,
            version: row.version,
        }
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

        let attestation = STANDARD.encode(b"runtime-veto-test");
        let record = service
            .register_key(
                provider_id,
                RegisterProviderKey {
                    alias: Some("primary".to_string()),
                    attestation_digest: Some(attestation.clone()),
                    attestation_signature: Some(attestation),
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

    #[tokio::test]
    async fn record_binding_persists_binding_and_audit_event() -> anyhow::Result<()> {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping record_binding_persists_binding_and_audit_event: DATABASE_URL not set",
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

        let attestation = STANDARD.encode(b"binding-test");
        let key = service
            .register_key(
                provider_id,
                RegisterProviderKey {
                    alias: Some("primary".to_string()),
                    attestation_digest: Some(attestation.clone()),
                    attestation_signature: Some(attestation.clone()),
                    rotation_due_at: None,
                },
            )
            .await?;

        let target_id = Uuid::new_v4();
        let binding = service
            .record_binding(
                provider_id,
                key.id,
                ProviderKeyBindingScope {
                    binding_type: "workspace".to_string(),
                    binding_target_id: target_id,
                    additional_context: json!({"workspace_name": "alpha"}),
                },
            )
            .await?;

        assert_eq!(binding.provider_key_id, key.id);
        assert_eq!(binding.binding_type, "workspace");
        assert_eq!(binding.binding_target_id, target_id);
        assert!(binding.revoked_at.is_none());

        let listed = service
            .list_bindings(provider_id, key.id)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        assert!(!listed.is_empty());

        let row = sqlx::query(
            "SELECT event_type, payload FROM provider_key_audit_events WHERE provider_key_id = $1 AND event_type = 'binding_attached' ORDER BY occurred_at DESC LIMIT 1",
        )
        .bind(key.id)
        .fetch_one(&pool)
        .await?;

        let event_type: String = row.get("event_type");
        let payload: serde_json::Value = row.get("payload");
        assert_eq!(event_type, "binding_attached");
        assert_eq!(
            payload
                .get("binding_target_id")
                .and_then(|value| value.as_str()),
            Some(target_id.to_string().as_str())
        );

        Ok(())
    }
}
