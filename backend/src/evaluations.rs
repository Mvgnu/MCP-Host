pub mod scheduler;

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use scheduler::record_trust_block;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{postgres::PgRow, PgPool, Row};

// key: evaluation-certifications -> evaluation_certifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificationStatus {
    Pending,
    Pass,
    Fail,
}

impl CertificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CertificationStatus::Pending => "pending",
            CertificationStatus::Pass => "pass",
            CertificationStatus::Fail => "fail",
        }
    }

    fn from_db(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(CertificationStatus::Pending),
            "pass" => Some(CertificationStatus::Pass),
            "fail" => Some(CertificationStatus::Fail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationCertification {
    pub id: i32,
    pub build_artifact_run_id: i32,
    pub manifest_digest: String,
    pub tier: String,
    pub policy_requirement: String,
    pub status: CertificationStatus,
    pub evidence: Option<Value>,
    pub evidence_source: Option<Value>,
    pub evidence_lineage: Option<Value>,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub refresh_cadence_seconds: Option<i64>,
    pub next_refresh_at: Option<DateTime<Utc>>,
    pub governance_notes: Option<String>,
    pub last_attestation_status: Option<String>,
    pub fallback_launched_at: Option<DateTime<Utc>>,
    pub remediation_attempts: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EvaluationCertification {
    pub fn is_active(&self, moment: DateTime<Utc>) -> bool {
        self.valid_from <= moment
            && self
                .valid_until
                .map(|until| until >= moment)
                .unwrap_or(true)
    }
}

#[derive(Debug, Clone)]
pub struct CertificationUpsert {
    pub build_artifact_run_id: i32,
    pub manifest_digest: String,
    pub tier: String,
    pub policy_requirement: String,
    pub status: CertificationStatus,
    pub evidence: Option<Value>,
    pub evidence_source: Option<Value>,
    pub evidence_lineage: Option<Value>,
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub refresh_cadence_seconds: Option<i64>,
    pub next_refresh_at: Option<DateTime<Utc>>,
    pub governance_notes: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CertificationPlanDelta {
    pub evidence_source: Option<Option<Value>>,
    pub evidence_lineage: Option<Option<Value>>,
    pub refresh_cadence_seconds: Option<Option<i64>>,
    pub next_refresh_at: Option<Option<DateTime<Utc>>>,
    pub governance_notes: Option<Option<String>>,
}

pub async fn get_certification(
    pool: &PgPool,
    certification_id: i32,
) -> Result<Option<EvaluationCertification>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            id,
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            evidence_source,
            evidence_lineage,
            valid_from,
            valid_until,
            refresh_cadence_seconds,
            next_refresh_at,
            governance_notes,
            last_attestation_status,
            fallback_launched_at,
            remediation_attempts,
            created_at,
            updated_at
        FROM evaluation_certifications
        WHERE id = $1
        "#,
    )
    .bind(certification_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_certification))
}

pub async fn upsert_certification(
    pool: &PgPool,
    payload: CertificationUpsert,
) -> Result<EvaluationCertification, sqlx::Error> {
    let next_refresh_at = compute_next_refresh(
        payload.next_refresh_at,
        payload.valid_from,
        payload.refresh_cadence_seconds,
    );
    let row = sqlx::query(
        r#"
        INSERT INTO evaluation_certifications (
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            evidence_source,
            evidence_lineage,
            valid_from,
            valid_until,
            refresh_cadence_seconds,
            next_refresh_at,
            governance_notes
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (manifest_digest, tier, policy_requirement)
        DO UPDATE SET
            build_artifact_run_id = EXCLUDED.build_artifact_run_id,
            status = EXCLUDED.status,
            evidence = EXCLUDED.evidence,
            evidence_source = EXCLUDED.evidence_source,
            evidence_lineage = EXCLUDED.evidence_lineage,
            valid_from = EXCLUDED.valid_from,
            valid_until = EXCLUDED.valid_until,
            refresh_cadence_seconds = EXCLUDED.refresh_cadence_seconds,
            next_refresh_at = EXCLUDED.next_refresh_at,
            governance_notes = EXCLUDED.governance_notes,
            updated_at = NOW()
        RETURNING
            id,
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            evidence_source,
            evidence_lineage,
            valid_from,
            valid_until,
            refresh_cadence_seconds,
            next_refresh_at,
            governance_notes,
            last_attestation_status,
            fallback_launched_at,
            remediation_attempts,
            created_at,
            updated_at
        "#,
    )
    .bind(payload.build_artifact_run_id)
    .bind(&payload.manifest_digest)
    .bind(&payload.tier)
    .bind(&payload.policy_requirement)
    .bind(payload.status.as_str())
    .bind(payload.evidence)
    .bind(payload.evidence_source)
    .bind(payload.evidence_lineage)
    .bind(payload.valid_from)
    .bind(payload.valid_until)
    .bind(payload.refresh_cadence_seconds)
    .bind(next_refresh_at)
    .bind(payload.governance_notes)
    .fetch_one(pool)
    .await?;

    Ok(map_certification(row))
}

pub async fn list_for_run(
    pool: &PgPool,
    run_id: i32,
) -> Result<Vec<EvaluationCertification>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            evidence_source,
            evidence_lineage,
            valid_from,
            valid_until,
            refresh_cadence_seconds,
            next_refresh_at,
            governance_notes,
            last_attestation_status,
            fallback_launched_at,
            remediation_attempts,
            created_at,
            updated_at
        FROM evaluation_certifications
        WHERE build_artifact_run_id = $1
        ORDER BY policy_requirement, valid_from DESC, id DESC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_certification).collect())
}

pub async fn list_for_digest_and_tier(
    pool: &PgPool,
    manifest_digest: &str,
    tier: &str,
) -> Result<Vec<EvaluationCertification>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            evidence_source,
            evidence_lineage,
            valid_from,
            valid_until,
            refresh_cadence_seconds,
            next_refresh_at,
            governance_notes,
            last_attestation_status,
            fallback_launched_at,
            remediation_attempts,
            created_at,
            updated_at
        FROM evaluation_certifications
        WHERE manifest_digest = $1 AND tier = $2
        ORDER BY policy_requirement, valid_from DESC, id DESC
        "#,
    )
    .bind(manifest_digest)
    .bind(tier)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_certification).collect())
}

pub async fn retry_certification(
    pool: &PgPool,
    certification_id: i32,
) -> Result<Option<EvaluationCertification>, sqlx::Error> {
    let state = sqlx::query(
        r#"
        SELECT last_attestation_status, remediation_attempts, fallback_launched_at
        FROM evaluation_certifications
        WHERE id = $1
        "#,
    )
    .bind(certification_id)
    .fetch_optional(pool)
    .await?;

    let Some(state) = state else {
        return Ok(None);
    };

    let last_status: Option<String> = state.try_get("last_attestation_status").unwrap_or(None);
    if matches!(last_status.as_deref(), Some("untrusted")) {
        let attempts: i32 = state.try_get("remediation_attempts").unwrap_or(0);
        let fallback_launched_at = state.try_get("fallback_launched_at").unwrap_or(None);
        record_trust_block(pool, certification_id, attempts, fallback_launched_at).await?;
        return Ok(None);
    }

    let row = sqlx::query(
        r#"
        UPDATE evaluation_certifications
        SET
            status = 'pending',
            valid_from = NOW(),
            valid_until = NULL,
            next_refresh_at = CASE
                WHEN refresh_cadence_seconds IS NOT NULL THEN NOW() + make_interval(secs => refresh_cadence_seconds::double precision)
                ELSE NULL
            END,
            updated_at = NOW()
        WHERE id = $1
        RETURNING
            id,
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            evidence_source,
            evidence_lineage,
            valid_from,
            valid_until,
            refresh_cadence_seconds,
            next_refresh_at,
            governance_notes,
            created_at,
            updated_at
        "#,
    )
    .bind(certification_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_certification))
}

pub async fn update_plan(
    pool: &PgPool,
    certification_id: i32,
    delta: CertificationPlanDelta,
) -> Result<Option<EvaluationCertification>, sqlx::Error> {
    let Some(mut certification) = get_certification(pool, certification_id).await? else {
        return Ok(None);
    };

    if let Some(value) = delta.evidence_source {
        certification.evidence_source = value;
    }
    if let Some(value) = delta.evidence_lineage {
        certification.evidence_lineage = value;
    }
    if let Some(value) = delta.refresh_cadence_seconds {
        certification.refresh_cadence_seconds = value;
    }
    if let Some(value) = delta.next_refresh_at {
        certification.next_refresh_at = value;
    }
    if let Some(value) = delta.governance_notes {
        certification.governance_notes = value;
    }

    let updated = upsert_certification(
        pool,
        CertificationUpsert {
            build_artifact_run_id: certification.build_artifact_run_id,
            manifest_digest: certification.manifest_digest.clone(),
            tier: certification.tier.clone(),
            policy_requirement: certification.policy_requirement.clone(),
            status: certification.status,
            evidence: certification.evidence.clone(),
            evidence_source: certification.evidence_source.clone(),
            evidence_lineage: certification.evidence_lineage.clone(),
            valid_from: certification.valid_from,
            valid_until: certification.valid_until,
            refresh_cadence_seconds: certification.refresh_cadence_seconds,
            next_refresh_at: certification.next_refresh_at,
            governance_notes: certification.governance_notes.clone(),
        },
    )
    .await?;

    Ok(Some(updated))
}

fn map_certification(row: PgRow) -> EvaluationCertification {
    let evidence: Option<Value> = row.try_get("evidence").ok();
    let status_str: String = row.get("status");
    let status = CertificationStatus::from_db(&status_str).unwrap_or(CertificationStatus::Pending);

    EvaluationCertification {
        id: row.get("id"),
        build_artifact_run_id: row.get("build_artifact_run_id"),
        manifest_digest: row.get("manifest_digest"),
        tier: row.get("tier"),
        policy_requirement: row.get("policy_requirement"),
        status,
        evidence,
        evidence_source: row
            .try_get::<Option<Value>, _>("evidence_source")
            .unwrap_or(None),
        evidence_lineage: row
            .try_get::<Option<Value>, _>("evidence_lineage")
            .unwrap_or(None),
        valid_from: row.get("valid_from"),
        valid_until: row.get("valid_until"),
        refresh_cadence_seconds: row
            .try_get::<Option<i64>, _>("refresh_cadence_seconds")
            .unwrap_or(None),
        next_refresh_at: row
            .try_get::<Option<DateTime<Utc>>, _>("next_refresh_at")
            .unwrap_or(None),
        governance_notes: row
            .try_get::<Option<String>, _>("governance_notes")
            .unwrap_or(None),
        last_attestation_status: row
            .try_get::<Option<String>, _>("last_attestation_status")
            .unwrap_or(None),
        fallback_launched_at: row
            .try_get::<Option<DateTime<Utc>>, _>("fallback_launched_at")
            .unwrap_or(None),
        remediation_attempts: row.try_get::<i32, _>("remediation_attempts").unwrap_or(0),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn compute_next_refresh(
    provided: Option<DateTime<Utc>>,
    valid_from: DateTime<Utc>,
    cadence_seconds: Option<i64>,
) -> Option<DateTime<Utc>> {
    if provided.is_some() {
        return provided;
    }
    let Some(seconds) = cadence_seconds else {
        return None;
    };
    if seconds <= 0 {
        return None;
    }
    Some(valid_from + Duration::seconds(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_next_refresh_prefers_provided_value() {
        let provided = Utc::now();
        let valid_from = provided - Duration::seconds(30);
        let result = compute_next_refresh(Some(provided), valid_from, Some(600));
        assert_eq!(result, Some(provided));
    }

    #[test]
    fn compute_next_refresh_uses_cadence_when_missing() {
        let valid_from = Utc::now();
        let result = compute_next_refresh(None, valid_from, Some(120));
        assert_eq!(result, Some(valid_from + Duration::seconds(120)));
    }

    #[test]
    fn compute_next_refresh_handles_invalid_values() {
        let valid_from = Utc::now();
        assert_eq!(compute_next_refresh(None, valid_from, Some(0)), None);
        assert_eq!(compute_next_refresh(None, valid_from, Some(-5)), None);
    }
}

pub async fn latest_per_requirement(
    pool: &PgPool,
    manifest_digest: &str,
    tier: &str,
) -> Result<HashMap<String, EvaluationCertification>, sqlx::Error> {
    let mut latest = HashMap::new();
    for certification in list_for_digest_and_tier(pool, manifest_digest, tier).await? {
        latest
            .entry(certification.policy_requirement.clone())
            .or_insert(certification);
    }
    Ok(latest)
}
