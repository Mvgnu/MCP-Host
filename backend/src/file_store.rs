use axum::{extract::{Multipart, Path, Extension}, http::{StatusCode, HeaderMap, header}, Json, response::IntoResponse};
use serde::Serialize;
use sqlx::{PgPool, Row};
use tokio::{fs, io::AsyncWriteExt};
use tracing::error;

#[derive(Serialize)]
pub struct FileInfo {
    pub id: i32,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn list_files(
    Path(server_id): Path<i32>,
    Extension(pool): Extension<PgPool>,
) -> Result<Json<Vec<FileInfo>>, (StatusCode, String)> {
    let rows = sqlx::query("SELECT id, name, created_at FROM server_files WHERE server_id = $1 ORDER BY id DESC")
        .bind(server_id)
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error listing files");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    let files = rows
        .into_iter()
        .map(|r| FileInfo {
            id: r.get("id"),
            name: r.get("name"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(files))
}

pub async fn upload_file(
    Path(server_id): Path<i32>,
    Extension(pool): Extension<PgPool>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let dir = format!("storage/{server_id}");
    if fs::create_dir_all(&dir).await.is_err() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Failed to create dir".into()));
    }
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let file_name = field.file_name().map(|s| s.to_string()).unwrap_or_else(|| "file.bin".into());
        let data = field.bytes().await.map_err(|e| {
            error!(?e, "Failed reading upload field");
            (StatusCode::BAD_REQUEST, "Read error".into())
        })?;
        let path = format!("{}/{}", dir, file_name);
        let mut f = fs::File::create(&path).await.map_err(|e| {
            error!(?e, "Failed creating file");
            (StatusCode::INTERNAL_SERVER_ERROR, "Write error".into())
        })?;
        f.write_all(&data).await.map_err(|e| {
            error!(?e, "Failed writing file");
            (StatusCode::INTERNAL_SERVER_ERROR, "Write error".into())
        })?;
        let rec = sqlx::query("INSERT INTO server_files (server_id, name, path) VALUES ($1,$2,$3) RETURNING id, created_at")
            .bind(server_id)
            .bind(&file_name)
            .bind(&path)
            .fetch_one(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error inserting file record");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
            })?;
        return Ok((StatusCode::CREATED, Json(FileInfo {
            id: rec.get("id"),
            name: file_name,
            created_at: rec.get("created_at"),
        })));
    }
    Err((StatusCode::BAD_REQUEST, "No file".into()))
}

pub async fn download_file(
    Path((server_id, file_id)): Path<(i32, i32)>,
    Extension(pool): Extension<PgPool>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let row = sqlx::query("SELECT name, path FROM server_files WHERE id = $1 AND server_id = $2")
        .bind(file_id)
        .bind(server_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching file metadata");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    let Some(r) = row else { return Err((StatusCode::NOT_FOUND, "File not found".into())); };
    let name: String = r.get("name");
    let path: String = r.get("path");
    let data = fs::read(&path).await.map_err(|e| {
        error!(?e, "File read error");
        (StatusCode::INTERNAL_SERVER_ERROR, "Read error".into())
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/octet-stream"));
    let disposition = format!("attachment; filename=\"{}\"", name);
    if let Ok(val) = header::HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, val);
    }
    Ok((headers, data))
}

pub async fn delete_file(
    Path((server_id, file_id)): Path<(i32, i32)>,
    Extension(pool): Extension<PgPool>,
) -> Result<StatusCode, (StatusCode, String)> {
    let row = sqlx::query("SELECT path FROM server_files WHERE id = $1 AND server_id = $2")
        .bind(file_id)
        .bind(server_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching file for deletion");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    let Some(r) = row else { return Err((StatusCode::NOT_FOUND, "File not found".into())); };
    let path: String = r.get("path");
    let _ = fs::remove_file(&path).await;
    sqlx::query("DELETE FROM server_files WHERE id = $1")
        .bind(file_id)
        .execute(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error deleting file record");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    Ok(StatusCode::NO_CONTENT)
}
