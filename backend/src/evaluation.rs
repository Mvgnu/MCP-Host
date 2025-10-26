use crate::error::{AppError, AppResult};
use crate::evaluations::{CertificationStatus, CertificationUpsert, EvaluationCertification};
use crate::extractor::AuthUser;
use axum::{
    extract::{Extension, Path},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use strsim::jaro_winkler;

#[derive(Serialize)]
pub struct EvaluationTest {
    pub id: i32,
    pub question: String,
    pub expected_answer: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateTest {
    pub question: String,
    pub expected_answer: String,
}

#[derive(Serialize)]
pub struct EvaluationResult {
    pub id: i32,
    pub test_id: i32,
    pub response: String,
    pub score: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct SubmitCertification {
    pub manifest_digest: Option<String>,
    pub tier: String,
    pub policy_requirement: String,
    pub status: CertificationStatus,
    #[serde(default)]
    pub evidence: Option<Value>,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
}

pub async fn list_tests(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> AppResult<Json<Vec<EvaluationTest>>> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id=$1 AND owner_id=$2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await?;
    if rec.is_none() {
        return Err(AppError::NotFound);
    }
    let rows = sqlx::query(
        "SELECT id, question, expected_answer, created_at FROM evaluation_tests WHERE server_id=$1 ORDER BY id"
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await?;
    let list = rows
        .into_iter()
        .map(|r| EvaluationTest {
            id: r.get("id"),
            question: r.get("question"),
            expected_answer: r.get("expected_answer"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(list))
}

pub async fn create_test(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
    Json(payload): Json<CreateTest>,
) -> AppResult<Json<EvaluationTest>> {
    let rec = sqlx::query(
        "INSERT INTO evaluation_tests (server_id, question, expected_answer) \
         SELECT id, $2, $3 FROM mcp_servers WHERE id=$1 AND owner_id=$4 RETURNING id, created_at",
    )
    .bind(server_id)
    .bind(&payload.question)
    .bind(&payload.expected_answer)
    .bind(user_id)
    .fetch_optional(&pool)
    .await?;
    let rec = rec.ok_or(AppError::NotFound)?;
    Ok(Json(EvaluationTest {
        id: rec.get("id"),
        question: payload.question,
        expected_answer: payload.expected_answer,
        created_at: rec.get("created_at"),
    }))
}

pub async fn list_results(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> AppResult<Json<Vec<EvaluationResult>>> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id=$1 AND owner_id=$2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await?;
    if rec.is_none() {
        return Err(AppError::NotFound);
    }
    let rows = sqlx::query(
        "SELECT r.id, r.test_id, r.response, r.score, r.created_at FROM evaluation_results r JOIN evaluation_tests t ON r.test_id=t.id WHERE t.server_id=$1 ORDER BY r.id DESC LIMIT 50"
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await?;
    let list = rows
        .into_iter()
        .map(|r| EvaluationResult {
            id: r.get("id"),
            test_id: r.get("test_id"),
            response: r.get("response"),
            score: r.get("score"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(list))
}

#[derive(Serialize)]
pub struct RunSummary {
    pub results: Vec<EvaluationResult>,
}

pub async fn run_tests(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> AppResult<Json<RunSummary>> {
    let row = sqlx::query("SELECT api_key FROM mcp_servers WHERE id=$1 AND owner_id=$2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await?;
    let Some(row) = row else {
        return Err(AppError::NotFound);
    };
    let api_key: String = row.get("api_key");
    let tests = sqlx::query(
        "SELECT id, question, expected_answer FROM evaluation_tests WHERE server_id=$1",
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await?;
    let mut results = Vec::new();
    for row in tests {
        let test_id: i32 = row.get("id");
        let question: String = row.get("question");
        let expected: String = row.get("expected_answer");
        let input = serde_json::json!({"question": question});
        let client = reqwest::Client::new();
        let resp_text = match client
            .post(format!("http://mcp-server-{server_id}:8080/invoke"))
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&input)
            .send()
            .await
        {
            Ok(resp) => resp.text().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        let score = jaro_winkler(&expected, &resp_text);
        let rec = sqlx::query(
            "INSERT INTO evaluation_results (test_id, response, score) VALUES ($1,$2,$3) RETURNING id, created_at"
        )
        .bind(test_id)
        .bind(&resp_text)
        .bind(score)
        .fetch_one(&pool)
        .await?;
        results.push(EvaluationResult {
            id: rec.get("id"),
            test_id,
            response: resp_text,
            score,
            created_at: rec.get("created_at"),
        });
    }
    Ok(Json(RunSummary { results }))
}

pub async fn list_all_results(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> AppResult<Json<Vec<(String, String, f64, chrono::DateTime<chrono::Utc>)>>> {
    let rows = sqlx::query(
        "SELECT s.name, t.question, r.score, r.created_at \
         FROM evaluation_results r \
         JOIN evaluation_tests t ON r.test_id=t.id \
         JOIN mcp_servers s ON t.server_id=s.id \
         WHERE s.owner_id=$1 ORDER BY r.created_at DESC LIMIT 50",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;
    let list = rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("name"),
                r.get::<String, _>("question"),
                r.get::<f64, _>("score"),
                r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            )
        })
        .collect();
    Ok(Json(list))
}

pub async fn list_certifications(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(run_id): Path<i32>,
) -> AppResult<Json<Vec<EvaluationCertification>>> {
    ensure_run_access(&pool, run_id, user_id).await?;
    let certifications = crate::evaluations::list_for_run(&pool, run_id).await?;
    Ok(Json(certifications))
}

pub async fn submit_certification(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(run_id): Path<i32>,
    Json(payload): Json<SubmitCertification>,
) -> AppResult<Json<EvaluationCertification>> {
    let RunAccess {
        manifest_digest: stored_digest,
    } = ensure_run_access(&pool, run_id, user_id).await?;

    let SubmitCertification {
        manifest_digest,
        tier,
        policy_requirement,
        status,
        evidence,
        valid_from,
        valid_until,
    } = payload;

    if tier.trim().is_empty() {
        return Err(AppError::BadRequest(
            "tier must be provided for certification".to_string(),
        ));
    }
    if policy_requirement.trim().is_empty() {
        return Err(AppError::BadRequest(
            "policy_requirement must be provided".to_string(),
        ));
    }

    let digest = manifest_digest.or(stored_digest).ok_or_else(|| {
        AppError::BadRequest("manifest digest required for certification".to_string())
    })?;

    let effective_from = valid_from.unwrap_or_else(Utc::now);
    if let Some(ref until) = valid_until {
        if *until <= effective_from {
            return Err(AppError::BadRequest(
                "valid_until must be later than valid_from".to_string(),
            ));
        }
    }

    let certification = crate::evaluations::upsert_certification(
        &pool,
        CertificationUpsert {
            build_artifact_run_id: run_id,
            manifest_digest: digest,
            tier,
            policy_requirement,
            status,
            evidence,
            valid_from: effective_from,
            valid_until,
        },
    )
    .await?;

    Ok(Json(certification))
}

pub async fn retry_certification(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(certification_id): Path<i32>,
) -> AppResult<Json<EvaluationCertification>> {
    let row = sqlx::query(
        r#"
        SELECT runs.id
        FROM evaluation_certifications cert
        JOIN build_artifact_runs runs ON cert.build_artifact_run_id = runs.id
        JOIN mcp_servers servers ON runs.server_id = servers.id
        WHERE cert.id = $1 AND servers.owner_id = $2
        "#,
    )
    .bind(certification_id)
    .bind(user_id)
    .fetch_optional(&pool)
    .await?;

    if row.is_none() {
        return Err(AppError::NotFound);
    }

    let certification = crate::evaluations::retry_certification(&pool, certification_id)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(certification))
}

struct RunAccess {
    manifest_digest: Option<String>,
}

async fn ensure_run_access(pool: &PgPool, run_id: i32, user_id: i32) -> AppResult<RunAccess> {
    let row = sqlx::query(
        r#"
        SELECT runs.manifest_digest
        FROM build_artifact_runs runs
        JOIN mcp_servers servers ON runs.server_id = servers.id
        WHERE runs.id = $1 AND servers.owner_id = $2
        "#,
    )
    .bind(run_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Err(AppError::NotFound);
    };
    Ok(RunAccess {
        manifest_digest: row.get("manifest_digest"),
    })
}

#[derive(Serialize)]
pub struct ServerScore {
    pub server: String,
    pub average_score: f64,
    pub runs: i64,
}

pub async fn scores_summary(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> AppResult<Json<Vec<ServerScore>>> {
    let rows = sqlx::query(
        "SELECT s.name, AVG(r.score) AS avg_score, COUNT(r.id) AS runs \
         FROM evaluation_results r \
         JOIN evaluation_tests t ON r.test_id=t.id \
         JOIN mcp_servers s ON t.server_id=s.id \
         WHERE s.owner_id=$1 GROUP BY s.name ORDER BY avg_score DESC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;
    let list = rows
        .into_iter()
        .map(|r| ServerScore {
            server: r.get("name"),
            average_score: r.get("avg_score"),
            runs: r.get("runs"),
        })
        .collect();
    Ok(Json(list))
}
