use axum::{Router, routing::get};
use axum::http::{Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt; // for `oneshot`

async fn root() -> &'static str { "MCP Host API" }

#[tokio::test]
async fn root_responds_ok() {
    let app = Router::new().route("/", get(root));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    assert_eq!(body, "MCP Host API".as_bytes());
}
