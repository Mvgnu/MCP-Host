use crate::docker;
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
pub struct VectorDb {
    pub id: i32,
    pub name: String,
    pub db_type: String,
    pub url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateVectorDb {
    pub name: String,
    #[serde(default = "default_db_type")]
    pub db_type: String,
}

fn default_db_type() -> String {
    "chroma".into()
}

pub async fn list_vector_dbs(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
) -> Result<Json<Vec<VectorDb>>, (StatusCode, String)> {
    let rows = sqlx::query(
        "SELECT id, name, db_type, url, created_at FROM vector_dbs WHERE owner_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error listing vector dbs");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let list = rows
        .into_iter()
        .map(|r| VectorDb {
            id: r.get("id"),
            name: r.get("name"),
            db_type: r.get("db_type"),
            url: r.try_get("url").ok(),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(list))
}

pub async fn create_vector_db(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Json(payload): Json<CreateVectorDb>,
) -> Result<Json<VectorDb>, (StatusCode, String)> {
    let rec = sqlx::query(
        "INSERT INTO vector_dbs (owner_id, name, db_type) VALUES ($1,$2,$3) RETURNING id, created_at"
    )
    .bind(user_id)
    .bind(&payload.name)
    .bind(&payload.db_type)
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error creating vector db");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let id: i32 = rec.get("id");
    let created_at: chrono::DateTime<chrono::Utc> = rec.get("created_at");
    docker::spawn_vector_db_task(id, payload.db_type.clone(), pool.clone());
    Ok(Json(VectorDb {
        id,
        name: payload.name,
        db_type: payload.db_type,
        url: None,
        created_at,
    }))
}

pub async fn delete_vector_db(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, String)> {
    let rec = sqlx::query("SELECT owner_id FROM vector_dbs WHERE id = $1")
        .bind(id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching vector db");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    match rec {
        Some(r) if r.get::<i32, _>("owner_id") == user_id => {}
        _ => return Err((StatusCode::NOT_FOUND, "Vector DB not found".into())),
    }
    docker::delete_vector_db_task(id, pool.clone());
    Ok(StatusCode::NO_CONTENT)
}
