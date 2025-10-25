use axum::async_trait;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;

#[derive(Deserialize)]
struct Claims {
    sub: i32,
    role: String,
    #[allow(dead_code)]
    exp: usize,
}

pub struct AuthUser {
    pub user_id: i32,
    pub role: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token_opt = if let Some(cookie_header) = parts.headers.get(axum::http::header::COOKIE) {
            let cookies = cookie_header.to_str().unwrap_or("");
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("auth_token=").map(|s| s.to_string())
            })
        } else if let Some(authz) = parts.headers.get(axum::http::header::AUTHORIZATION) {
            authz
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix("Bearer ").map(|s| s.to_string()))
        } else {
            None
        };
        let token = token_opt.ok_or((StatusCode::UNAUTHORIZED, "Missing token".into()))?;
        let secret = crate::config::JWT_SECRET.as_str();
        let decoded = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token".into()))?;
        Ok(AuthUser {
            user_id: decoded.claims.sub,
            role: decoded.claims.role,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::Request, RequestPartsExt};
    use jsonwebtoken::{encode, EncodingKey, Header};

    #[tokio::test]
    async fn token_parsed_from_header() {
        let claims = serde_json::json!({"sub": 7, "role": "user", "exp": 9999999999u64});
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(b"secret"),
        )
        .unwrap();
        std::env::set_var("JWT_SECRET", "secret");
        let request = Request::builder()
            .header("Authorization", format!("Bearer {}", token))
            .body(axum::body::Body::empty())
            .unwrap();
        let mut parts = request.into_parts().0;
        let user = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(user.user_id, 7);
        assert_eq!(user.role, "user");
    }

    #[tokio::test]
    async fn invalid_token_rejected() {
        std::env::set_var("JWT_SECRET", "secret");
        let request = Request::builder()
            .header("Authorization", "Bearer invalid")
            .body(axum::body::Body::empty())
            .unwrap();
        let mut parts = request.into_parts().0;
        let res = AuthUser::from_request_parts(&mut parts, &()).await;
        assert!(res.is_err());
    }
}
