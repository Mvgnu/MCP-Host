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

/// Optional image pull secret used by the Kubernetes runtime when refreshing registry credentials.
pub static K8S_REGISTRY_SECRET_NAME: Lazy<Option<String>> = Lazy::new(|| {
    std::env::var("K8S_REGISTRY_SECRET_NAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
});

/// Path to a dockerconfigjson file containing registry credentials.
pub static REGISTRY_AUTH_DOCKERCONFIG: Lazy<Option<String>> = Lazy::new(|| {
    std::env::var("REGISTRY_AUTH_DOCKERCONFIG")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
});

/// Architectures that should be targeted when building/publishing images. Provide a
/// comma-separated list such as `linux/amd64,linux/arm64` via `REGISTRY_ARCH_TARGETS`.
/// Defaults to just `linux/amd64` so existing single-arch builds continue to function.
pub static REGISTRY_ARCH_TARGETS: Lazy<Vec<String>> = Lazy::new(|| {
    std::env::var("REGISTRY_ARCH_TARGETS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|raw| {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect::<Vec<_>>()
        })
        .filter(|targets| !targets.is_empty())
        .unwrap_or_else(|| vec!["linux/amd64".to_string()])
});

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
