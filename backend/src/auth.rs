use crate::extractor::AuthUser;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::{
    extract::Extension,
    http::{HeaderMap, StatusCode},
    Json,
};
use tracing::error;
use crate::error::{AppError, AppResult};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
struct Claims {
    sub: i32,
    role: String,
    exp: usize,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: i32,
    pub email: String,
    pub role: String,
    pub server_quota: i32,
}

pub async fn register_user(
    Extension(pool): Extension<PgPool>,
    Json(payload): Json<RegisterRequest>,
) -> AppResult<StatusCode> {
    if payload.password.len() < 8 {
        return Err(AppError::BadRequest("Password too short".into()));
    }
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(payload.password.as_bytes(), &salt)
        .map_err(|e| AppError::Message(format!("Hashing failed: {}", e)))?;
    let result = sqlx::query("INSERT INTO users (email, password_hash) VALUES ($1, $2)")
        .bind(&payload.email)
        .bind(hash.to_string())
        .execute(&pool)
        .await;
    match result {
        Ok(_) => Ok(StatusCode::CREATED),
        Err(e) => {
            if let sqlx::Error::Database(db_err) = &e {
                if db_err.constraint() == Some("users_email_key") {
                    return Err(AppError::BadRequest("Email already registered".into()));
                }
            }
            Err(AppError::Db(e))
        }
    }
}

pub async fn login_user(
    Extension(pool): Extension<PgPool>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<(HeaderMap, &'static str)> {
    let rec = sqlx::query("SELECT id, password_hash, role FROM users WHERE email = $1")
        .bind(&payload.email)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while fetching user");
            AppError::Db(e)
        })?;
    let rec = rec.ok_or(AppError::Unauthorized)?;
    let id: i32 = rec.get("id");
    let pass_hash: String = rec.get("password_hash");
    let role: String = rec.get("role");
    let parsed = PasswordHash::new(&pass_hash)
        .map_err(|e| {
            error!(?e, "Hash parse error");
            AppError::Message(format!("Hash error: {}", e))
        })?;
    if Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed)
        .is_err()
    {
        return Err(AppError::Unauthorized);
    }
    let exp = Utc::now()
        .checked_add_signed(Duration::hours(24))
        .expect("valid timestamp")
        .timestamp() as usize;
    let claims = Claims { sub: id, role: role.clone(), exp };
    let secret = crate::config::JWT_SECRET.as_str();
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| {
        error!(?e, "Token encoding error");
        AppError::Message("Token error".into())
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        format!("auth_token={token}; HttpOnly; Secure; SameSite=Strict; Path=/")
            .parse()
            .expect("valid header value"),
    );
    Ok((headers, "Login successful"))
}

pub async fn logout_user() -> (HeaderMap, &'static str) {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        "auth_token=deleted; HttpOnly; Path=/; Max-Age=0"
            .parse()
            .expect("valid header value"),
    );
    (headers, "Logged out")
}

pub async fn current_user(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id, role }: AuthUser,
) -> AppResult<Json<UserInfo>> {
    let rec = sqlx::query("SELECT email, server_quota FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            error!(?e, "DB error while fetching user email");
            AppError::Db(e)
        })?;
    let Some(row) = rec else {
        return Err(AppError::NotFound);
    };
    let email: String = row.get("email");
    let quota: i32 = row.get("server_quota");
    Ok(Json(UserInfo { id: user_id, email, role, server_quota: quota }))
}
