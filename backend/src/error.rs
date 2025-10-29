use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("docker error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("vault error: {0}")]
    Vault(#[from] reqwest::Error),
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("bad request")]
    JsonBadRequest(Value),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("bad gateway: {0}")]
    BadGateway(String),
    #[error("{0}")]
    Message(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::JsonBadRequest(payload) => {
                tracing::error!(payload = ?payload, "json bad request");
                (StatusCode::BAD_REQUEST, Json(payload)).into_response()
            }
            other => {
                let status = match &other {
                    AppError::NotFound => StatusCode::NOT_FOUND,
                    AppError::Unauthorized => StatusCode::UNAUTHORIZED,
                    AppError::Forbidden => StatusCode::FORBIDDEN,
                    AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
                    AppError::Conflict(_) => StatusCode::CONFLICT,
                    AppError::BadGateway(_) => StatusCode::BAD_GATEWAY,
                    AppError::Db(_)
                    | AppError::Docker(_)
                    | AppError::Vault(_)
                    | AppError::Message(_)
                    | AppError::JsonBadRequest(_) => StatusCode::INTERNAL_SERVER_ERROR,
                };
                tracing::error!(error = ?other);
                (status, other.to_string()).into_response()
            }
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
