use once_cell::sync::Lazy;

/// Secret used for JWT signing. Must be set via the `JWT_SECRET` env variable.
pub static JWT_SECRET: Lazy<String> =
    Lazy::new(|| std::env::var("JWT_SECRET").expect("JWT_SECRET must be set"));

/// Container runtime backend. Defaults to `docker`.
pub static CONTAINER_RUNTIME: Lazy<String> =
    Lazy::new(|| std::env::var("CONTAINER_RUNTIME").unwrap_or_else(|_| "docker".to_string()));

/// Namespace used by the Kubernetes runtime. Defaults to `default`.
pub static K8S_NAMESPACE: Lazy<String> =
    Lazy::new(|| std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "default".to_string()));

/// Service account name used by the Kubernetes runtime. Defaults to `default`.
pub static K8S_SERVICE_ACCOUNT: Lazy<String> =
    Lazy::new(|| std::env::var("K8S_SERVICE_ACCOUNT").unwrap_or_else(|_| "default".to_string()));
