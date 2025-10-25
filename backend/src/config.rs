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

/// Address the HTTP server should bind to. Defaults to `0.0.0.0`.
pub static BIND_ADDRESS: Lazy<String> =
    Lazy::new(|| std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_string()));

/// Port the HTTP server should listen on. Defaults to `3000`.
pub static BIND_PORT: Lazy<u16> = Lazy::new(|| {
    std::env::var("BIND_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000)
});

/// When set to a truthy value, allows the application to continue running even if database
/// migrations fail. Defaults to `false`.
pub static ALLOW_MIGRATION_FAILURE: Lazy<bool> = Lazy::new(|| {
    std::env::var("ALLOW_MIGRATION_FAILURE")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes")
        })
        .unwrap_or(false)
});
