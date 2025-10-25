use backend::telemetry::{validate_metric_details, Metric, MetricValidationError};
use chrono::Utc;
use serde_json::json;

#[test]
fn rest_metrics_payload_preserves_registry_schema() {
    let metric = Metric {
        id: 42,
        timestamp: Utc::now(),
        event_type: "push_failed".to_string(),
        details: Some(json!({
            "attempt": 1,
            "retry_limit": 3,
            "registry_endpoint": "registry.test/example",
            "error_kind": "auth_expired",
            "auth_expired": true,
        })),
    };

    let payload = serde_json::to_value(vec![metric.clone()]).expect("metrics should serialize");
    let array = payload.as_array().expect("response should be an array");
    let first = array.first().expect("at least one metric");
    let details = first
        .get("details")
        .and_then(|value| value.as_object())
        .expect("details should be an object");

    for key in [
        "attempt",
        "retry_limit",
        "registry_endpoint",
        "error_kind",
        "auth_expired",
    ] {
        assert!(
            details.contains_key(key),
            "missing key {key} in REST payload"
        );
    }

    assert_eq!(
        first.get("event_type").and_then(|v| v.as_str()),
        Some("push_failed")
    );
}

#[test]
fn sse_metrics_payload_preserves_registry_schema() {
    let metric = Metric {
        id: 43,
        timestamp: Utc::now(),
        event_type: "push_retry".to_string(),
        details: Some(json!({
            "attempt": 2,
            "retry_limit": 3,
            "registry_endpoint": "registry.test/example",
            "reason": "auth_refresh",
        })),
    };

    let serialized = serde_json::to_string(&metric).expect("metric serializes to JSON");
    let value: serde_json::Value = serde_json::from_str(&serialized).expect("SSE payload parses");
    let details = value
        .get("details")
        .and_then(|value| value.as_object())
        .expect("details should be an object");

    for key in ["attempt", "retry_limit", "registry_endpoint", "reason"] {
        assert!(
            details.contains_key(key),
            "missing key {key} in SSE payload"
        );
    }
    assert_eq!(
        value.get("event_type").and_then(|v| v.as_str()),
        Some("push_retry")
    );
}

#[test]
fn missing_registry_fields_surface_validation_error() {
    let err = validate_metric_details(
        "push_failed",
        Some(&json!({
            "attempt": 1,
            "retry_limit": 3,
        })),
    )
    .expect_err("missing registry fields should error");

    assert!(matches!(
        err,
        MetricValidationError::MissingField {
            field: "registry_endpoint",
            ..
        }
    ));
}
