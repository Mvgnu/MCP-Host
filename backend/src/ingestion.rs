use crate::extractor::AuthUser;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tracing::error;

#[derive(Serialize)]
pub struct IngestionJob {
    pub id: i32,
    pub vector_db_id: i32,
    pub source_url: String,
    pub schedule_minutes: i32,
    pub last_run: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateJob {
    pub vector_db_id: i32,
    pub source_url: String,
    #[serde(default)]
    pub schedule_minutes: i32,
}

pub async fn list_jobs(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> Result<Json<Vec<IngestionJob>>, (StatusCode, String)> {
    let rows = sqlx::query(
        "SELECT id, vector_db_id, source_url, schedule_minutes, last_run, created_at \
         FROM ingestion_jobs WHERE owner_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error listing ingestion jobs");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let jobs = rows
        .into_iter()
        .map(|r| IngestionJob {
            id: r.get("id"),
            vector_db_id: r.get("vector_db_id"),
            source_url: r.get("source_url"),
            schedule_minutes: r.get("schedule_minutes"),
            last_run: r.try_get("last_run").ok(),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(jobs))
}

pub async fn create_job(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<CreateJob>,
) -> Result<Json<IngestionJob>, (StatusCode, String)> {
    let rec = sqlx::query(
        "INSERT INTO ingestion_jobs (owner_id, vector_db_id, source_url, schedule_minutes) \
         VALUES ($1,$2,$3,$4) RETURNING id, last_run, created_at",
    )
    .bind(user_id)
    .bind(payload.vector_db_id)
    .bind(&payload.source_url)
    .bind(payload.schedule_minutes)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error creating ingestion job");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    Ok(Json(IngestionJob {
        id: rec.get("id"),
        vector_db_id: payload.vector_db_id,
        source_url: payload.source_url,
        schedule_minutes: payload.schedule_minutes,
        last_run: rec.try_get("last_run").ok(),
        created_at: rec.get("created_at"),
    }))
}

pub async fn delete_job(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    let res = sqlx::query("DELETE FROM ingestion_jobs WHERE id = $1 AND owner_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error deleting job");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if res.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Job not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub fn start_ingestion_worker(pool: PgPool) {
    tokio::spawn(async move {
        loop {
            let rows = sqlx::query(
                "SELECT id, vector_db_id, source_url, schedule_minutes, last_run FROM ingestion_jobs"
            )
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
            let now = chrono::Utc::now();
            for row in rows {
                let id: i32 = row.get("id");
                let vector_db_id: i32 = row.get("vector_db_id");
                let url: String = row.get("source_url");
                let schedule: i32 = row.get("schedule_minutes");
                let last_run: Option<chrono::DateTime<chrono::Utc>> = row.try_get("last_run").ok();
                let due = match last_run {
                    Some(t) => now - t > chrono::Duration::minutes(schedule as i64),
                    None => true,
                };
                if due {
                    if let Ok(resp) = reqwest::get(&url).await {
                        if let Ok(text) = resp.text().await {
                            let target = format!("http://mcp-vectordb-{vector_db_id}:8000/ingest");
                            let _ = reqwest::Client::new().post(&target).body(text).send().await;
                            let _ = sqlx::query(
                                "UPDATE ingestion_jobs SET last_run = NOW() WHERE id = $1",
                            )
                            .bind(id)
                            .execute(&pool)
                            .await;
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}
