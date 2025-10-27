use std::cmp::Ordering;
use std::collections::HashMap;

use axum::{
    extract::{Extension, Path},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use thiserror::Error;

use crate::error::{AppError, AppResult};

// key: capability-intelligence -> module:v0.1

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntelligenceStatus {
    Healthy,
    Warning,
    Critical,
}

impl IntelligenceStatus {
    fn from_score(score: f32) -> Self {
        if score >= 80.0 {
            IntelligenceStatus::Healthy
        } else if score >= 60.0 {
            IntelligenceStatus::Warning
        } else {
            IntelligenceStatus::Critical
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            IntelligenceStatus::Healthy => "healthy",
            IntelligenceStatus::Warning => "warning",
            IntelligenceStatus::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceScore {
    pub server_id: i32,
    pub capability: String,
    pub backend: Option<String>,
    pub tier: Option<String>,
    pub score: f32,
    pub status: IntelligenceStatus,
    pub confidence: f32,
    pub last_observed_at: DateTime<Utc>,
    pub notes: Vec<String>,
    pub evidence: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct IntelligenceScorePayload {
    pub capability: String,
    pub backend: Option<String>,
    pub tier: Option<String>,
    pub score: f32,
    pub status: String,
    pub confidence: f32,
    pub last_observed_at: DateTime<Utc>,
    pub notes: Vec<String>,
    pub evidence: Vec<Value>,
}

#[derive(Debug, Error)]
pub enum IntelligenceError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct ScoreSignals {
    pub artifact_status: Option<String>,
    pub artifact_health: Option<String>,
    pub artifact_completed_at: Option<DateTime<Utc>>,
    pub credential_health: Option<String>,
    pub multi_arch: Option<bool>,
    pub policy_backend: Option<String>,
    pub tier: Option<String>,
    pub policy_health: Option<String>,
    pub policy_capabilities: Vec<String>,
    pub capabilities_satisfied: Option<bool>,
    pub policy_notes: Vec<String>,
    pub policy_decided_at: Option<DateTime<Utc>>,
}

impl Default for ScoreSignals {
    fn default() -> Self {
        Self {
            artifact_status: None,
            artifact_health: None,
            artifact_completed_at: None,
            credential_health: None,
            multi_arch: None,
            policy_backend: None,
            tier: None,
            policy_health: None,
            policy_capabilities: Vec::new(),
            capabilities_satisfied: None,
            policy_notes: Vec::new(),
            policy_decided_at: None,
        }
    }
}

pub struct RecomputeContext<'a> {
    pub server_id: i32,
    pub backend: &'a str,
    pub tier: Option<&'a str>,
    pub capability_keys: &'a [String],
    pub fallback_capabilities_satisfied: bool,
}

pub async fn ensure_scores(
    pool: &PgPool,
    context: &RecomputeContext<'_>,
) -> Result<HashMap<String, IntelligenceScore>, IntelligenceError> {
    let mut scores = load_scores(pool, context.server_id).await?;
    let stale_threshold = Utc::now() - Duration::minutes(15);
    let needs_refresh = context
        .capability_keys
        .iter()
        .any(|cap| matches!(scores.get(cap), Some(score) if score.last_observed_at < stale_threshold))
        || context
            .capability_keys
            .iter()
            .any(|cap| !scores.contains_key(cap));

    if needs_refresh {
        let recomputed = recompute_scores(pool, context).await?;
        for score in &recomputed {
            scores.insert(score.capability.clone(), score.clone());
        }
    }

    Ok(scores)
}

pub async fn recompute_scores(
    pool: &PgPool,
    context: &RecomputeContext<'_>,
) -> Result<Vec<IntelligenceScore>, IntelligenceError> {
    let signals = load_signals(pool, context.server_id).await?;
    let mut results = Vec::new();
    let base = build_base_score(&signals);
    let capabilities_satisfied = signals
        .capabilities_satisfied
        .unwrap_or(context.fallback_capabilities_satisfied);

    let mut capabilities = context.capability_keys.to_vec();
    if !capabilities.iter().any(|cap| cap == "runtime") {
        capabilities.push("runtime".to_string());
    }

    for capability in capabilities {
        let (score, notes, evidence) = compute_capability_score(
            &capability,
            base,
            &signals,
            context.backend,
            context.tier,
            capabilities_satisfied,
        );
        let status = IntelligenceStatus::from_score(score);
        let last_observed_at = signals
            .policy_decided_at
            .or(signals.artifact_completed_at)
            .unwrap_or_else(Utc::now);
        let record = upsert_score(
            pool,
            context.server_id,
            &capability,
            Some(context.backend),
            context.tier,
            score,
            status,
            0.85,
            last_observed_at,
            &notes,
            &evidence,
        )
        .await?;
        results.push(record);
    }

    Ok(results)
}

fn build_base_score(signals: &ScoreSignals) -> f32 {
    let mut score = 85.0;
    if let Some(health) = signals.artifact_health.as_deref() {
        match health {
            "healthy" => score += 5.0,
            "degraded" => score -= 15.0,
            "error" | "failed" => score -= 30.0,
            other if other.contains("watch") => score -= 10.0,
            _ => {}
        }
    } else {
        score -= 20.0;
    }

    if let Some(status) = signals.artifact_status.as_deref() {
        if !matches_success(status) {
            score -= 15.0;
        }
    } else {
        score -= 10.0;
    }

    if let Some(cred) = signals.credential_health.as_deref() {
        if cred != "healthy" {
            score -= 10.0;
        }
    }

    if let Some(policy_health) = signals.policy_health.as_deref() {
        if policy_health != "healthy" {
            score -= 10.0;
        }
    }

    score.clamp(0.0, 100.0)
}

fn compute_capability_score(
    capability: &str,
    base: f32,
    signals: &ScoreSignals,
    backend: &str,
    tier: Option<&str>,
    fallback_capabilities_satisfied: bool,
) -> (f32, Vec<String>, Vec<Value>) {
    let mut score = base;
    let mut notes = Vec::new();
    let mut evidence = Vec::new();

    if let Some(policy_backend) = signals.policy_backend.as_deref() {
        if policy_backend != backend {
            score -= 5.0;
            notes.push(format!("backend:diverged:{}->{}", policy_backend, backend));
        }
    }

    for note in &signals.policy_notes {
        if note.contains("capabilities:unsatisfied") && capability != "runtime" {
            score -= 25.0;
            notes.push(format!("unsatisfied:{note}"));
        }
        if note.contains("evaluation:reason") {
            score -= 5.0;
        }
        if note.contains("health:") {
            notes.push(format!("health-note:{note}"));
        }
    }

    if capability == "runtime" {
        if let Some(policy_health) = signals.policy_health.as_deref() {
            evidence.push(Value::String(format!("policy-health:{policy_health}")));
        }
        if let Some(artifact_status) = signals.artifact_status.as_deref() {
            evidence.push(Value::String(format!("artifact-status:{artifact_status}")));
        }
    }

    if !fallback_capabilities_satisfied && capability != "runtime" {
        score -= 15.0;
        notes.push("fallback-capabilities-unsatisfied".into());
    }

    if let Some(tier_value) = tier {
        if tier_value.starts_with("gold") {
            score -= 0.0;
        } else if tier_value.starts_with("silver") {
            score -= 2.5;
        } else if tier_value.starts_with("watchlist") {
            score -= 5.0;
        }
        evidence.push(Value::String(format!("tier:{tier_value}")));
    }

    if !signals.policy_capabilities.is_empty() {
        evidence.push(json!({
            "policy_capabilities": signals.policy_capabilities.clone()
        }));
    }

    if let Some(multi_arch) = signals.multi_arch {
        evidence.push(json!({
            "multi_arch": multi_arch
        }));
    }

    (score.clamp(0.0, 100.0), notes, evidence)
}

pub fn minimum_threshold(capability: &str, tier: Option<&str>) -> f32 {
    let base = match capability {
        "runtime" => 65.0,
        "gpu" => 75.0,
        "image-build" => 70.0,
        _ => 60.0,
    };

    if let Some(tier_value) = tier {
        if tier_value.starts_with("gold") {
            base + 10.0
        } else if tier_value.starts_with("silver") {
            base + 5.0
        } else {
            base
        }
    } else {
        base
    }
}

pub async fn load_scores(
    pool: &PgPool,
    server_id: i32,
) -> Result<HashMap<String, IntelligenceScore>, IntelligenceError> {
    let rows = sqlx::query(
        r#"
        SELECT
            capability,
            backend,
            tier,
            score,
            status,
            confidence,
            last_observed_at,
            notes,
            evidence
        FROM capability_intelligence_scores
        WHERE server_id = $1
        "#,
    )
    .bind(server_id)
    .fetch_all(pool)
    .await?;

    let mut map = HashMap::new();
    for row in rows {
        let capability: String = row.get("capability");
        let backend: Option<String> = row.get("backend");
        let tier: Option<String> = row.get("tier");
        let score_value: f32 = row.get::<f64, _>("score") as f32;
        let status_text: String = row.get("status");
        let status = match status_text.as_str() {
            "healthy" => IntelligenceStatus::Healthy,
            "warning" => IntelligenceStatus::Warning,
            "critical" => IntelligenceStatus::Critical,
            _ => IntelligenceStatus::Warning,
        };
        let confidence = row.get::<f64, _>("confidence") as f32;
        let last_observed_at: DateTime<Utc> = row.get("last_observed_at");
        let notes_json: Value = row.get("notes");
        let notes = match notes_json {
            Value::Array(values) => values
                .into_iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        let evidence_json: Value = row.get("evidence");
        let evidence = match evidence_json {
            Value::Array(values) => values,
            other => vec![other],
        };

        map.insert(
            capability.clone(),
            IntelligenceScore {
                server_id,
                capability,
                backend,
                tier,
                score: score_value,
                status,
                confidence,
                last_observed_at,
                notes,
                evidence,
            },
        );
    }

    Ok(map)
}

pub async fn list_scores(
    Extension(pool): Extension<PgPool>,
    Path(server_id): Path<i32>,
) -> AppResult<Json<Vec<IntelligenceScorePayload>>> {
    let scores = load_scores(&pool, server_id)
        .await
        .map_err(|err| match err {
            IntelligenceError::Database(db_err) => AppError::Db(db_err),
        })?;

    let mut payload: Vec<_> = scores
        .into_iter()
        .map(|(_, score)| IntelligenceScorePayload {
            capability: score.capability,
            backend: score.backend,
            tier: score.tier,
            score: score.score,
            status: score.status.as_str().to_string(),
            confidence: score.confidence,
            last_observed_at: score.last_observed_at,
            notes: score.notes,
            evidence: score.evidence,
        })
        .collect();

    payload.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
    });

    Ok(Json(payload))
}

pub async fn recompute_from_history(
    pool: &PgPool,
    server_id: i32,
) -> Result<Vec<IntelligenceScore>, IntelligenceError> {
    if let Some(row) = sqlx::query(
        r#"
        SELECT
            backend,
            tier,
            capability_requirements,
            capabilities_satisfied
        FROM runtime_policy_decisions
        WHERE server_id = $1
        ORDER BY decided_at DESC
        LIMIT 1
        "#,
    )
    .bind(server_id)
    .fetch_optional(pool)
    .await?
    {
        let backend: String = row.get("backend");
        let tier_value: Option<String> = row.get("tier");
        let cap_json: Value = row.get("capability_requirements");
        let capability_keys: Vec<String> = match cap_json {
            Value::Array(values) => values
                .into_iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        let satisfied: bool = row.get("capabilities_satisfied");

        let context = RecomputeContext {
            server_id,
            backend: backend.as_str(),
            tier: tier_value.as_deref(),
            capability_keys: &capability_keys,
            fallback_capabilities_satisfied: satisfied,
        };

        let records = recompute_scores(pool, &context).await?;
        return Ok(records);
    }

    Ok(Vec::new())
}

async fn upsert_score(
    pool: &PgPool,
    server_id: i32,
    capability: &str,
    backend: Option<&str>,
    tier: Option<&str>,
    score: f32,
    status: IntelligenceStatus,
    confidence: f32,
    last_observed_at: DateTime<Utc>,
    notes: &[String],
    evidence: &[Value],
) -> Result<IntelligenceScore, IntelligenceError> {
    let status_text = match status {
        IntelligenceStatus::Healthy => "healthy",
        IntelligenceStatus::Warning => "warning",
        IntelligenceStatus::Critical => "critical",
    };
    let notes_json = Value::Array(notes.iter().map(|note| Value::String(note.clone())).collect());
    let evidence_json = Value::Array(evidence.iter().cloned().collect());

    let row = sqlx::query(
        r#"
        INSERT INTO capability_intelligence_scores (
            server_id,
            capability,
            backend,
            tier,
            score,
            status,
            confidence,
            last_observed_at,
            notes,
            evidence
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (server_id, capability, backend, tier)
        DO UPDATE SET
            score = EXCLUDED.score,
            status = EXCLUDED.status,
            confidence = EXCLUDED.confidence,
            last_observed_at = EXCLUDED.last_observed_at,
            notes = EXCLUDED.notes,
            evidence = EXCLUDED.evidence,
            updated_at = NOW()
        RETURNING
            capability,
            backend,
            tier,
            score,
            status,
            confidence,
            last_observed_at,
            notes,
            evidence
        "#,
    )
    .bind(server_id)
    .bind(capability)
    .bind(backend)
    .bind(tier)
    .bind(score as f64)
    .bind(status_text)
    .bind(confidence as f64)
    .bind(last_observed_at)
    .bind(notes_json)
    .bind(evidence_json)
    .fetch_one(pool)
    .await?;

    let backend_value: Option<String> = row.get("backend");
    let tier_value: Option<String> = row.get("tier");
    let score_value: f32 = row.get::<f64, _>("score") as f32;
    let status_value: String = row.get("status");
    let last_observed_at: DateTime<Utc> = row.get("last_observed_at");
    let notes_value: Value = row.get("notes");
    let evidence_value: Value = row.get("evidence");

    let status_enum = match status_value.as_str() {
        "healthy" => IntelligenceStatus::Healthy,
        "warning" => IntelligenceStatus::Warning,
        "critical" => IntelligenceStatus::Critical,
        _ => IntelligenceStatus::Warning,
    };

    let notes_vec = match notes_value {
        Value::Array(values) => values
            .into_iter()
            .filter_map(|value| value.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };

    let evidence_vec = match evidence_value {
        Value::Array(values) => values,
        other => vec![other],
    };

    Ok(IntelligenceScore {
        server_id,
        capability: capability.to_string(),
        backend: backend_value,
        tier: tier_value,
        score: score_value,
        status: status_enum,
        confidence,
        last_observed_at,
        notes: notes_vec,
        evidence: evidence_vec,
    })
}

async fn load_signals(pool: &PgPool, server_id: i32) -> Result<ScoreSignals, IntelligenceError> {
    let mut signals = ScoreSignals::default();

    if let Some(row) = sqlx::query(
        r#"
        SELECT
            status,
            credential_health_status,
            completed_at,
            multi_arch
        FROM build_artifact_runs
        WHERE server_id = $1
        ORDER BY completed_at DESC
        LIMIT 1
        "#,
    )
    .bind(server_id)
    .fetch_optional(pool)
    .await?
    {
        signals.artifact_status = Some(row.get("status"));
        signals.credential_health = Some(row.get("credential_health_status"));
        signals.artifact_completed_at = Some(row.get("completed_at"));
        signals.multi_arch = Some(row.get("multi_arch"));
        signals.artifact_health = Some(derive_artifact_health(
            signals.artifact_status.as_deref(),
            signals.credential_health.as_deref(),
        ));
    }

    if let Some(row) = sqlx::query(
        r#"
        SELECT
            backend,
            tier,
            health_overall,
            capability_requirements,
            capabilities_satisfied,
            notes,
            decided_at
        FROM runtime_policy_decisions
        WHERE server_id = $1
        ORDER BY decided_at DESC
        LIMIT 1
        "#,
    )
    .bind(server_id)
    .fetch_optional(pool)
    .await?
    {
        signals.policy_backend = Some(row.get("backend"));
        signals.tier = row.get("tier");
        signals.policy_health = row.get("health_overall");
        signals.capabilities_satisfied = Some(row.get("capabilities_satisfied"));
        let cap_json: Value = row.get("capability_requirements");
        signals.policy_capabilities = match cap_json {
            Value::Array(values) => values
                .into_iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        let notes_json: Value = row.get("notes");
        signals.policy_notes = match notes_json {
            Value::Array(values) => values
                .into_iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        signals.policy_decided_at = Some(row.get("decided_at"));
    }

    Ok(signals)
}

fn derive_artifact_health(status: Option<&str>, credential: Option<&str>) -> String {
    match (status, credential) {
        (Some(status), Some(credential)) if matches_success(status) && matches_healthy(credential) => {
            "healthy".to_string()
        }
        (Some(status), Some(credential)) if matches_success(status) => {
            format!("watch:{credential}")
        }
        (Some(status), _) if !matches_success(status) => "degraded".to_string(),
        _ => "unknown".to_string(),
    }
}

fn matches_success(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "succeeded" | "success" | "completed"
    )
}

fn matches_healthy(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "healthy" | "ok" | "success" | "succeeded" | "passing"
    )
}
