use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{routing::get, Router};
use axum_prometheus::PrometheusMetricLayer;
use tower::ServiceExt;

#[tokio::test]
async fn metrics_returns_ok() {
    let (layer, handle) = PrometheusMetricLayer::pair();
    let app = Router::new()
        .route("/metrics", get(move || async move { handle.render() }))
        .layer(layer);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
