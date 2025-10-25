use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Serialize, Clone)]
pub struct Metric {
    pub id: i32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub event_type: String,
    pub details: Option<Value>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MetricValidationError {
    #[error("metric `{event_type}` missing registry detail payload")]
    MissingDetails { event_type: String },
    #[error("metric `{event_type}` missing required registry detail `{field}`")]
    MissingField {
        event_type: String,
        field: &'static str,
    },
}

#[derive(Debug, Error)]
pub enum MetricError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Validation(#[from] MetricValidationError),
}

pub fn validate_metric_details(
    event_type: &str,
    details: Option<&Value>,
) -> Result<(), MetricValidationError> {
    match event_type {
        ty if ty.starts_with("push_") => {
            let payload = details.ok_or_else(|| MetricValidationError::MissingDetails {
                event_type: event_type.to_string(),
            })?;
            require_field(payload, event_type, "attempt")?;
            require_field(payload, event_type, "retry_limit")?;
            require_field(payload, event_type, "registry_endpoint")?;
            require_field(payload, event_type, "platform")?;
            if event_type == "push_failed" {
                require_field(payload, event_type, "error_kind")?;
                require_field(payload, event_type, "auth_expired")?;
            }
            if event_type == "push_retry" {
                require_field(payload, event_type, "reason")?;
            }
        }
        ty if ty.starts_with("tag_") => {
            let payload = details.ok_or_else(|| MetricValidationError::MissingDetails {
                event_type: event_type.to_string(),
            })?;
            require_field(payload, event_type, "registry_endpoint")?;
            require_field(payload, event_type, "tag")?;
            require_field(payload, event_type, "platform")?;
        }
        "manifest_published" => {
            let payload = details.ok_or_else(|| MetricValidationError::MissingDetails {
                event_type: event_type.to_string(),
            })?;
            require_field(payload, event_type, "registry_endpoint")?;
            require_field(payload, event_type, "tag")?;
            require_field(payload, event_type, "digest")?;
            require_field(payload, event_type, "architectures")?;
        }
        _ => {}
    }
    Ok(())
}

fn require_field<'a>(
    payload: &'a Value,
    event_type: &str,
    field: &'static str,
) -> Result<&'a Value, MetricValidationError> {
    payload
        .get(field)
        .ok_or_else(|| MetricValidationError::MissingField {
            event_type: event_type.to_string(),
            field,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_metrics_require_expected_fields() {
        let payload = json!({
            "attempt": 1,
            "retry_limit": 3,
            "registry_endpoint": "registry.test/example",
            "error_kind": "remote",
            "auth_expired": false,
            "platform": "linux/amd64",
        });

        assert!(validate_metric_details("push_failed", Some(&payload)).is_ok());
    }

    #[test]
    fn missing_registry_field_is_reported() {
        let payload = json!({
            "attempt": 1,
            "retry_limit": 3,
            "platform": "linux/amd64",
        });

        let err = validate_metric_details("push_failed", Some(&payload))
            .expect_err("missing registry_endpoint should error");
        assert!(matches!(
            err,
            MetricValidationError::MissingField {
                field: "registry_endpoint",
                ..
            }
        ));
    }

    #[test]
    fn manifest_metrics_require_expected_fields() {
        let payload = json!({
            "registry_endpoint": "registry.test/example",
            "tag": "latest",
            "digest": "sha256:123",
            "architectures": ["linux/amd64"],
        });

        assert!(validate_metric_details("manifest_published", Some(&payload)).is_ok());
    }
}
