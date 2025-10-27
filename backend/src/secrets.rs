use crate::extractor::AuthUser;
use crate::vault::VaultClient;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tracing::error;

#[derive(Serialize)]
pub struct SecretInfo {
    pub id: i32,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize, Serialize)]
pub struct CreateSecret {
    pub name: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct UpdateSecret {
    pub value: String,
}

pub fn encryption_key() -> String {
    std::env::var("SECRET_KEY").unwrap_or_else(|_| "secret".into())
}

pub async fn list_secrets(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
) -> Result<Json<Vec<SecretInfo>>, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let rows = sqlx::query(
        "SELECT id, name, created_at FROM server_secrets WHERE server_id = $1 ORDER BY id",
    )
    .bind(server_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        error!(?e, "DB error listing secrets");
        (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
    })?;
    let secrets = rows
        .into_iter()
        .map(|r| SecretInfo {
            id: r.get("id"),
            name: r.get("name"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(secrets))
}

pub async fn create_secret(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path(server_id): Path<i32>,
    Json(payload): Json<CreateSecret>,
) -> Result<StatusCode, (StatusCode, String)> {
    if payload.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name required".into()));
    }
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    if let Some(vault) = VaultClient::from_env() {
        let path = format!("servers/{}/{}", server_id, payload.name);
        vault
            .store_secret(&path, &payload.value)
            .await
            .map_err(|e| {
                error!(?e, "Vault error storing secret");
                (StatusCode::INTERNAL_SERVER_ERROR, "Vault error".into())
            })?;
        sqlx::query("INSERT INTO server_secrets (server_id, name, value) VALUES ($1, $2, $3)")
            .bind(server_id)
            .bind(&payload.name)
            .bind(format!("vault:{}", path))
            .execute(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error inserting secret path");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
            })?;
    } else {
        let key = encryption_key();
        sqlx::query(
            "INSERT INTO server_secrets (server_id, name, value) VALUES ($1, $2, pgp_sym_encrypt($3, $4))",
        )
        .bind(server_id)
        .bind(&payload.name)
        .bind(&payload.value)
        .bind(&key)
        .execute(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error inserting secret");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    }
    Ok(StatusCode::CREATED)
}

pub async fn get_secret(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((server_id, secret_id)): Path<(i32, i32)>,
) -> Result<Json<CreateSecret>, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let row =
        sqlx::query("SELECT name, value FROM server_secrets WHERE id = $1 AND server_id = $2")
            .bind(secret_id)
            .bind(server_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| {
                error!(?e, "DB error fetching secret");
                (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
            })?;
    if let Some(r) = row {
        let name: String = r.get("name");
        let value: String = r.get("value");
        if let Some(path) = value.strip_prefix("vault:") {
            if let Some(vault) = VaultClient::from_env() {
                let val = vault.read_secret(path).await.map_err(|e| {
                    error!(?e, "Vault error reading secret");
                    (StatusCode::INTERNAL_SERVER_ERROR, "Vault error".into())
                })?;
                Ok(Json(CreateSecret { name, value: val }))
            } else {
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Vault not configured".into(),
                ))
            }
        } else {
            let key = encryption_key();
            let row = sqlx::query("SELECT pgp_sym_decrypt($1::bytea, $2) as value")
                .bind(value)
                .bind(&key)
                .fetch_one(&pool)
                .await
                .map_err(|e| {
                    error!(?e, "DB error decrypting secret");
                    (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
                })?;
            let val: String = row.get("value");
            Ok(Json(CreateSecret { name, value: val }))
        }
    } else {
        Err((StatusCode::NOT_FOUND, "Secret not found".into()))
    }
}

pub async fn update_secret(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((server_id, secret_id)): Path<(i32, i32)>,
    Json(payload): Json<UpdateSecret>,
) -> Result<StatusCode, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let row = sqlx::query("SELECT value FROM server_secrets WHERE id = $1 AND server_id = $2")
        .bind(secret_id)
        .bind(server_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching secret");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    let Some(r) = row else {
        return Err((StatusCode::NOT_FOUND, "Secret not found".into()));
    };
    let stored: String = r.get("value");
    if let Some(path) = stored.strip_prefix("vault:") {
        if let Some(vault) = VaultClient::from_env() {
            vault
                .store_secret(path, &payload.value)
                .await
                .map_err(|e| {
                    error!(?e, "Vault error updating secret");
                    (StatusCode::INTERNAL_SERVER_ERROR, "Vault error".into())
                })?;
        } else {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Vault not configured".into(),
            ));
        }
    } else {
        let key = encryption_key();
        let result = sqlx::query(
            "UPDATE server_secrets SET value = pgp_sym_encrypt($1, $2) WHERE id = $3 AND server_id = $4",
        )
        .bind(&payload.value)
        .bind(&key)
        .bind(secret_id)
        .bind(server_id)
        .execute(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error updating secret");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
        if result.rows_affected() == 0 {
            return Err((StatusCode::NOT_FOUND, "Secret not found".into()));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_secret(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, .. }: AuthUser,
    Path((server_id, secret_id)): Path<(i32, i32)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let rec = sqlx::query("SELECT id FROM mcp_servers WHERE id = $1 AND owner_id = $2")
        .bind(server_id)
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while verifying server owner");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if rec.is_none() {
        return Err((StatusCode::NOT_FOUND, "Server not found".into()));
    }
    let row = sqlx::query("SELECT value FROM server_secrets WHERE id = $1 AND server_id = $2")
        .bind(secret_id)
        .bind(server_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error fetching secret");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    let Some(r) = row else {
        return Err((StatusCode::NOT_FOUND, "Secret not found".into()));
    };
    let stored: String = r.get("value");
    if let Some(path) = stored.strip_prefix("vault:") {
        if let Some(vault) = VaultClient::from_env() {
            vault.delete_secret(path).await.map_err(|e| {
                error!(?e, "Vault error deleting secret");
                (StatusCode::INTERNAL_SERVER_ERROR, "Vault error".into())
            })?;
        } else {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Vault not configured".into(),
            ));
        }
    }
    let result = sqlx::query("DELETE FROM server_secrets WHERE id = $1 AND server_id = $2")
        .bind(secret_id)
        .bind(server_id)
        .execute(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error deleting secret");
            (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into())
        })?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Secret not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
