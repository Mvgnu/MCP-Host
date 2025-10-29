use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, QueryBuilder};
use uuid::Uuid;

use crate::keys::events::{ProviderKeyAuditEvent, ProviderKeyAuditEventType};
use crate::keys::models::ProviderKeyState;

/// key: audit-provider-key-filter
/// Filter envelope applied to BYOK audit queries sourced by CLI and console workflows.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProviderKeyAuditFilter {
    pub provider_id: Uuid,
    pub provider_key_id: Option<Uuid>,
    pub state: Option<ProviderKeyState>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

/// key: audit-provider-key-log
/// Structured BYOK audit entry with optional state context for UX surfaces.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderScopedAuditLog {
    pub event: ProviderKeyAuditEvent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_key_state: Option<ProviderKeyState>,
}

pub async fn query_provider_key_events(
    pool: &PgPool,
    filter: ProviderKeyAuditFilter,
) -> Result<Vec<ProviderScopedAuditLog>> {
    let mut builder = QueryBuilder::new(
        "SELECT e.id, e.provider_id, e.provider_key_id, e.event_type, e.payload, e.occurred_at, k.state \
         FROM provider_key_audit_events e \
         LEFT JOIN provider_keys k ON e.provider_key_id = k.id ",
    );
    builder.push("WHERE e.provider_id = ");
    builder.push_bind(filter.provider_id);

    if let Some(key_id) = filter.provider_key_id {
        builder.push(" AND e.provider_key_id = ");
        builder.push_bind(key_id);
    }

    if let Some(state) = filter.state {
        builder.push(" AND (k.state = ");
        builder.push_bind(state.as_str());
        builder.push(" OR (e.payload ? 'final_state' AND e.payload ->> 'final_state' = ");
        builder.push_bind(state.as_str());
        builder.push(") OR (e.payload ? 'state' AND e.payload ->> 'state' = ");
        builder.push_bind(state.as_str());
        builder.push("))");
    }

    if let Some(start) = filter.start {
        builder.push(" AND e.occurred_at >= ");
        builder.push_bind(start);
    }

    if let Some(end) = filter.end {
        builder.push(" AND e.occurred_at <= ");
        builder.push_bind(end);
    }

    builder.push(" ORDER BY e.occurred_at DESC");

    if let Some(limit) = filter.limit {
        builder.push(" LIMIT ");
        builder.push_bind(limit);
    }

    let rows = builder
        .build_query_as::<ProviderKeyAuditQueryRow>()
        .fetch_all(pool)
        .await?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let event_type = ProviderKeyAuditEventType::from_str(&row.event_type)
            .ok_or_else(|| anyhow!("unknown provider key audit event type: {}", row.event_type))?;
        let event = ProviderKeyAuditEvent {
            id: row.id,
            provider_id: row.provider_id,
            provider_key_id: row.provider_key_id,
            event_type,
            payload: row.payload,
            occurred_at: row.occurred_at,
        };
        let state = row.state.map(|value| ProviderKeyState::from_str(&value));
        events.push(ProviderScopedAuditLog {
            event,
            provider_key_state: state,
        });
    }

    Ok(events)
}

#[derive(sqlx::FromRow)]
struct ProviderKeyAuditQueryRow {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub provider_key_id: Option<Uuid>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
    pub state: Option<String>,
}
