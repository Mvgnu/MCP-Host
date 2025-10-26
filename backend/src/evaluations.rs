use std::collections::HashMap;

use chrono::{DateTime, Utc};
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
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
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
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}

pub async fn upsert_certification(
    pool: &PgPool,
    payload: CertificationUpsert,
) -> Result<EvaluationCertification, sqlx::Error> {
    let row = sqlx::query(
        r#"
        INSERT INTO evaluation_certifications (
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            valid_from,
            valid_until
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (manifest_digest, tier, policy_requirement)
        DO UPDATE SET
            build_artifact_run_id = EXCLUDED.build_artifact_run_id,
            status = EXCLUDED.status,
            evidence = EXCLUDED.evidence,
            valid_from = EXCLUDED.valid_from,
            valid_until = EXCLUDED.valid_until,
            updated_at = NOW()
        RETURNING
            id,
            build_artifact_run_id,
            manifest_digest,
            tier,
            policy_requirement,
            status,
            evidence,
            valid_from,
            valid_until,
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
    .bind(payload.valid_from)
    .bind(payload.valid_until)
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
            valid_from,
            valid_until,
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
            valid_from,
            valid_until,
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
    let row = sqlx::query(
        r#"
        UPDATE evaluation_certifications
        SET
            status = 'pending',
            valid_from = NOW(),
            valid_until = NULL,
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
            valid_from,
            valid_until,
            created_at,
            updated_at
        "#,
    )
    .bind(certification_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_certification))
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
        valid_from: row.get("valid_from"),
        valid_until: row.get("valid_until"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
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
