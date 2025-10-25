use crate::config::{REGISTRY_ARCH_TARGETS, REGISTRY_AUTH_DOCKERCONFIG};
use crate::servers::{add_metric, set_status, SetStatusError};
use crate::telemetry::MetricError;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use bollard::body_full;
use bollard::image::BuildImageOptions;
use bollard::models::PushImageInfo;
use bollard::query_parameters::{PushImageOptionsBuilder, TagImageOptionsBuilder};
use bollard::Docker;
use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;
use serde_json::Value;
use sqlx::PgPool;
use std::fmt;
use std::fs as stdfs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tar::Builder as TarBuilder;
use tempfile::tempdir;
use thiserror::Error;
use tokio::fs;
use tokio::time::{sleep, Duration as TokioDuration};
use url::Url;

#[derive(Clone, Copy)]
enum LangBuilder {
    Node,
    Python,
    Rust,
}

struct RegistryReference {
    repository: String,
    tag: String,
}

impl RegistryReference {
    fn display_name(&self) -> String {
        format!("{}:{}", self.repository, self.tag)
    }
}

fn build_registry_reference(registry: &str, image_tag: &str, tag: &str) -> RegistryReference {
    let registry = registry.trim_end_matches('/');
    RegistryReference {
        repository: format!("{registry}/{image_tag}"),
        tag: tag.to_string(),
    }
}

#[derive(Debug)]
enum RegistryPushError {
    Tag(bollard::errors::Error),
    Push(bollard::errors::Error),
    Remote(String),
    AuthExpired(String),
}

impl fmt::Display for RegistryPushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryPushError::Tag(err) => write!(f, "docker tag failed: {err}"),
            RegistryPushError::Push(err) => write!(f, "docker push failed: {err}"),
            RegistryPushError::Remote(msg) => write!(f, "registry rejected image: {msg}"),
            RegistryPushError::AuthExpired(msg) => {
                write!(f, "registry authentication expired: {msg}")
            }
        }
    }
}

#[derive(Debug, Error)]
enum ManifestPublishError {
    #[error("manifest publishing requires registry credentials for {0}")]
    MissingCredentials(String),
    #[error("failed to parse registry url: {0}")]
    InvalidRegistryUrl(String),
    #[error("http error while publishing manifest: {0}")]
    Http(String),
    #[error("registry rejected manifest publish: {0}")]
    Remote(String),
}

#[derive(Debug, Clone)]
struct PlatformTarget {
    spec: String,
    slug: String,
    os: String,
    architecture: String,
    variant: Option<String>,
}

impl PlatformTarget {
    fn parse(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split('/').collect();
        if parts.len() < 2 {
            return None;
        }
        let os = parts[0].trim();
        let arch = parts[1].trim();
        if os.is_empty() || arch.is_empty() {
            return None;
        }
        let variant = parts.get(2).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let slug = spec
            .chars()
            .map(|c| match c {
                '/' | ':' | '\\' => '_',
                other => other,
            })
            .collect::<String>();
        Some(Self {
            spec: spec.to_string(),
            slug,
            os: os.to_string(),
            architecture: arch.to_string(),
            variant,
        })
    }

    fn manifest_platform(&self) -> serde_json::Value {
        let mut platform = json!({
            "os": self.os,
            "architecture": self.architecture,
        });
        if let Some(variant) = &self.variant {
            platform
                .as_object_mut()
                .expect("platform json is object")
                .insert("variant".into(), json!(variant));
        }
        platform
    }
}

fn desired_platform_targets() -> Vec<PlatformTarget> {
    REGISTRY_ARCH_TARGETS
        .iter()
        .filter_map(|spec| {
            let spec = spec.trim();
            if spec.is_empty() {
                return None;
            }
            match PlatformTarget::parse(spec) {
                Some(target) => Some(target),
                None => {
                    tracing::warn!(%spec, "ignoring invalid registry architecture target");
                    None
                }
            }
        })
        .collect()
}

fn load_registry_auth_header(registry_host: &str) -> Option<String> {
    let config_path = std::env::var("REGISTRY_AUTH_DOCKERCONFIG")
        .ok()
        .or_else(|| REGISTRY_AUTH_DOCKERCONFIG.as_ref().cloned())?;
    let contents = stdfs::read_to_string(config_path).ok()?;
    let json: Value = serde_json::from_str(&contents).ok()?;
    let auths = json.get("auths")?.as_object()?;
    let candidate_keys = [
        registry_host.to_string(),
        format!("https://{registry_host}"),
        format!("http://{registry_host}"),
    ];
    for key in candidate_keys {
        if let Some(entry) = auths.get(&key) {
            if let Some(auth) = entry.get("auth").and_then(|v| v.as_str()) {
                let trimmed = auth.trim();
                if trimmed.is_empty() {
                    continue;
                }
                return Some(format!("Basic {trimmed}"));
            }
            if let (Some(user), Some(pass)) = (
                entry.get("username").and_then(|v| v.as_str()),
                entry.get("password").and_then(|v| v.as_str()),
            ) {
                let encoded = Base64Engine.encode(format!("{user}:{pass}"));
                return Some(format!("Basic {encoded}"));
            }
        }
    }
    None
}

struct RegistryLocation {
    base: Url,
    host: String,
    auth_host: String,
    repository: String,
}

fn registry_location(
    registry: &str,
    image_tag: &str,
) -> Result<RegistryLocation, ManifestPublishError> {
    let trimmed = registry.trim().trim_end_matches('/');
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let base = Url::parse(&candidate)
        .map_err(|err| ManifestPublishError::InvalidRegistryUrl(err.to_string()))?;
    let host = base
        .host_str()
        .ok_or_else(|| ManifestPublishError::InvalidRegistryUrl(registry.to_string()))?
        .to_string();
    let auth_host = if let Some(port) = base.port() {
        format!("{host}:{port}")
    } else {
        host.clone()
    };
    let path = base.path().trim_matches('/');
    let repository = if path.is_empty() {
        image_tag.to_string()
    } else {
        format!("{path}/{image_tag}")
    };
    Ok(RegistryLocation {
        base,
        host,
        auth_host,
        repository,
    })
}

async fn publish_manifest_list<L: BuildLogSink + ?Sized, M: MetricRecorder + ?Sized>(
    logger: &L,
    metrics: &M,
    server_id: i32,
    registry: &str,
    image_tag: &str,
    manifest_tag: &str,
    entries: &[(PlatformTarget, String)],
) -> Result<String, ManifestPublishError> {
    if entries.is_empty() {
        return Err(ManifestPublishError::Remote(
            "no architecture digests available for manifest publish".to_string(),
        ));
    }

    let location = registry_location(registry, image_tag)?;
    let auth = load_registry_auth_header(&location.auth_host)
        .ok_or_else(|| ManifestPublishError::MissingCredentials(location.host.clone()))?;

    let mut manifest_url = location.base.clone();
    manifest_url.set_path(&format!(
        "/v2/{}/manifests/{}",
        location.repository, manifest_tag
    ));

    let manifests = entries
        .iter()
        .map(|(platform, digest)| {
            json!({
                "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                "digest": digest,
                "platform": platform.manifest_platform(),
            })
        })
        .collect::<Vec<_>>();

    let payload = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
        "manifests": manifests,
    });

    let body = serde_json::to_string(&payload)
        .map_err(|err| ManifestPublishError::Http(err.to_string()))?;

    logger
        .log(
            server_id,
            &format!(
                "Publishing manifest list for {}:{}",
                location.repository, manifest_tag
            ),
        )
        .await;
    tracing::info!(
        target: "registry.push",
        %server_id,
        repository = %location.repository,
        %manifest_tag,
        "publishing manifest list",
    );

    let response = MANIFEST_HTTP_CLIENT
        .put(manifest_url.clone())
        .header(AUTHORIZATION, auth)
        .header(
            CONTENT_TYPE,
            "application/vnd.docker.distribution.manifest.list.v2+json",
        )
        .header(
            ACCEPT,
            "application/vnd.docker.distribution.manifest.list.v2+json",
        )
        .body(body)
        .send()
        .await
        .map_err(|err| ManifestPublishError::Http(err.to_string()))?;

    let status = response.status();
    let headers = response.headers().clone();
    let response_text = response
        .text()
        .await
        .unwrap_or_else(|_| "<unavailable>".to_string());

    if !status.is_success() {
        tracing::error!(
            target: "registry.push",
            %server_id,
            repository = %location.repository,
            %manifest_tag,
            status = %status,
            body = %response_text,
            "manifest publish failed",
        );
        return Err(ManifestPublishError::Remote(format!(
            "{status}: {response_text}"
        )));
    }

    let digest = headers
        .get("Docker-Content-Digest")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_default();

    metrics
        .record(
            "manifest_published",
            Some(json!({
                "registry_endpoint": format!(
                    "{}/{}",
                    registry.trim_end_matches('/'),
                    image_tag
                ),
                "tag": manifest_tag,
                "digest": digest,
                "architectures": entries
                    .iter()
                    .map(|(platform, _)| platform.spec.clone())
                    .collect::<Vec<_>>(),
            })),
        )
        .await;

    logger
        .log(
            server_id,
            &format!(
                "Manifest list published for {}:{}",
                location.repository, manifest_tag
            ),
        )
        .await;

    Ok(digest)
}

const DEFAULT_REGISTRY_PUSH_RETRIES: usize = 3;

fn registry_push_retry_limit() -> usize {
    std::env::var("REGISTRY_PUSH_RETRIES")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|&value| value > 0)
        .unwrap_or(DEFAULT_REGISTRY_PUSH_RETRIES)
}

static MANIFEST_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent("mcp-host-manifest-publisher/1.0")
        .build()
        .expect("failed to construct manifest HTTP client")
});

fn registry_scopes(repository: &str) -> Vec<String> {
    vec![
        format!("repository:{repository}:push"),
        format!("repository:{repository}:pull"),
    ]
}

const DEFAULT_CREDENTIAL_MAX_AGE_SECS: u64 = 86_400;
const DEFAULT_CREDENTIAL_ROTATE_LEAD_SECS: u64 = 3_600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialHealthStatus {
    Healthy,
    ExpiringSoon,
    Expired,
    Unknown,
}

impl CredentialHealthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CredentialHealthStatus::Healthy => "healthy",
            CredentialHealthStatus::ExpiringSoon => "expiring_soon",
            CredentialHealthStatus::Expired => "expired",
            CredentialHealthStatus::Unknown => "unknown",
        }
    }

    fn severity(&self) -> u8 {
        match self {
            CredentialHealthStatus::Expired => 3,
            CredentialHealthStatus::ExpiringSoon => 2,
            CredentialHealthStatus::Healthy => 1,
            CredentialHealthStatus::Unknown => 0,
        }
    }

    pub fn combine(self, other: CredentialHealthStatus) -> CredentialHealthStatus {
        if self.severity() >= other.severity() {
            self
        } else {
            other
        }
    }

    fn requires_rotation(&self) -> bool {
        matches!(
            self,
            CredentialHealthStatus::Expired | CredentialHealthStatus::ExpiringSoon
        )
    }
}

#[derive(Debug, Clone)]
struct CredentialHealthSnapshot {
    status: CredentialHealthStatus,
    observed_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    source_path: Option<String>,
    message: Option<String>,
}

impl CredentialHealthSnapshot {
    fn new(
        status: CredentialHealthStatus,
        observed_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
        source_path: Option<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            status,
            observed_at,
            expires_at,
            source_path,
            message,
        }
    }

    fn seconds_until_expiry(&self) -> Option<i64> {
        self.expires_at
            .map(|expiry| (expiry - self.observed_at).num_seconds())
    }

    fn requires_rotation(&self) -> bool {
        self.status.requires_rotation()
    }
}

fn configured_dockerconfig_path() -> Option<String> {
    std::env::var("REGISTRY_AUTH_DOCKERCONFIG")
        .ok()
        .or_else(|| REGISTRY_AUTH_DOCKERCONFIG.as_ref().cloned())
        .filter(|value| !value.trim().is_empty())
}

fn registry_auth_host(registry: &str) -> Option<String> {
    let trimmed = registry.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let mut parts = without_scheme.split('/');
    parts
        .next()
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
}

fn credential_max_age() -> std::time::Duration {
    std::env::var("REGISTRY_AUTH_MAX_AGE_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(DEFAULT_CREDENTIAL_MAX_AGE_SECS))
}

fn credential_rotation_lead_time() -> std::time::Duration {
    std::env::var("REGISTRY_AUTH_ROTATE_LEAD_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(DEFAULT_CREDENTIAL_ROTATE_LEAD_SECS))
}

fn chrono_from_std(duration: std::time::Duration) -> ChronoDuration {
    ChronoDuration::from_std(duration)
        .unwrap_or_else(|_| ChronoDuration::seconds(duration.as_secs() as i64))
}

fn compute_credential_health(registry: &str) -> CredentialHealthSnapshot {
    // key: registry-credential-health-snapshot
    let observed_at = Utc::now();
    let path = match configured_dockerconfig_path() {
        Some(path) => path,
        None => {
            return CredentialHealthSnapshot::new(
                CredentialHealthStatus::Expired,
                observed_at,
                None,
                None,
                Some("REGISTRY_AUTH_DOCKERCONFIG not configured".to_string()),
            );
        }
    };

    let host = match registry_auth_host(registry) {
        Some(host) => host,
        None => {
            return CredentialHealthSnapshot::new(
                CredentialHealthStatus::Unknown,
                observed_at,
                None,
                Some(path),
                Some("Unable to derive registry host from REGISTRY value".to_string()),
            );
        }
    };

    match stdfs::metadata(&path) {
        Ok(metadata) => {
            let auth_present = load_registry_auth_header(&host).is_some();
            if !auth_present {
                return CredentialHealthSnapshot::new(
                    CredentialHealthStatus::Expired,
                    observed_at,
                    None,
                    Some(path),
                    Some(format!("No auth entry for {host} in dockerconfig")),
                );
            }

            let expires_at = metadata.modified().ok().map(|modified| {
                let modified: DateTime<Utc> = modified.into();
                let expiry_window = chrono_from_std(credential_max_age());
                modified + expiry_window
            });

            if let Some(expiry) = expires_at {
                if expiry <= observed_at {
                    return CredentialHealthSnapshot::new(
                        CredentialHealthStatus::Expired,
                        observed_at,
                        Some(expiry),
                        Some(path),
                        Some("Credentials exceeded configured max age".to_string()),
                    );
                }
                let lead = chrono_from_std(credential_rotation_lead_time());
                if expiry - observed_at <= lead {
                    return CredentialHealthSnapshot::new(
                        CredentialHealthStatus::ExpiringSoon,
                        observed_at,
                        Some(expiry),
                        Some(path),
                        Some("Credentials approaching rotation window".to_string()),
                    );
                }
                return CredentialHealthSnapshot::new(
                    CredentialHealthStatus::Healthy,
                    observed_at,
                    Some(expiry),
                    Some(path),
                    Some("Credentials present and within rotation window".to_string()),
                );
            }

            CredentialHealthSnapshot::new(
                CredentialHealthStatus::Healthy,
                observed_at,
                None,
                Some(path),
                Some("Credentials present (no modification timestamp)".to_string()),
            )
        }
        Err(err) => CredentialHealthSnapshot::new(
            CredentialHealthStatus::Expired,
            observed_at,
            None,
            Some(path),
            Some(format!("Failed to stat dockerconfig: {err}")),
        ),
    }
}

async fn record_credential_health_and_rotate<L, M>(
    logger: &L,
    metrics: &M,
    server_id: i32,
    registry: &str,
    repository: &str,
    platform: &str,
    mut refresher: Option<&mut dyn RegistryAuthRefresher>,
) -> (CredentialHealthSnapshot, bool, bool)
where
    L: BuildLogSink + ?Sized,
    M: MetricRecorder + ?Sized,
{
    let snapshot = compute_credential_health(registry);
    let rotation_configured = refresher.is_some();
    let rotation_recommended = snapshot.requires_rotation();
    let seconds_until_expiry = snapshot.seconds_until_expiry();

    metrics
        .record(
            "auth_health_reported",
            Some(json!({
                "registry_endpoint": repository,
                "registry": registry,
                "platform": platform,
                "status": snapshot.status.as_str(),
                "observed_at": snapshot.observed_at.to_rfc3339(),
                "expires_at": snapshot.expires_at.map(|dt| dt.to_rfc3339()),
                "seconds_until_expiry": seconds_until_expiry,
                "rotation_recommended": rotation_recommended,
                "rotation_configured": rotation_configured,
                "source_path": snapshot.source_path.clone(),
                "message": snapshot.message.clone(),
            })),
        )
        .await;

    let mut rotation_attempted = false;
    let mut rotation_succeeded = false;

    if rotation_recommended {
        let log_message = format!(
            "Registry credentials reported status '{}'{}",
            snapshot.status.as_str(),
            snapshot
                .message
                .as_ref()
                .map(|msg| format!(": {msg}"))
                .unwrap_or_default()
        );
        insert_log(logger, server_id, &log_message).await;
        tracing::warn!(
            target: "registry.push",
            %repository,
            %server_id,
            %platform,
            status = snapshot.status.as_str(),
            rotation_configured,
            seconds_until_expiry,
            "registry credentials outside healthy window",
        );

        if let Some(refresher) = refresher.as_mut() {
            rotation_attempted = true;
            metrics
                .record(
                    "auth_rotation_started",
                    Some(json!({
                        "registry_endpoint": repository,
                        "registry": registry,
                        "platform": platform,
                        "status": snapshot.status.as_str(),
                        "observed_at": snapshot.observed_at.to_rfc3339(),
                    })),
                )
                .await;
            match refresher.refresh().await {
                Ok(()) => {
                    rotation_succeeded = true;
                    metrics
                        .record(
                            "auth_rotation_succeeded",
                            Some(json!({
                                "registry_endpoint": repository,
                                "registry": registry,
                                "platform": platform,
                                "status": snapshot.status.as_str(),
                                "observed_at": Utc::now().to_rfc3339(),
                            })),
                        )
                        .await;
                    insert_log(
                        logger,
                        server_id,
                        "Proactively rotated registry credentials",
                    )
                    .await;
                    tracing::info!(
                        target: "registry.push",
                        %repository,
                        %server_id,
                        %platform,
                        "proactive registry credential rotation succeeded",
                    );
                }
                Err(err) => {
                    metrics
                        .record(
                            "auth_rotation_failed",
                            Some(json!({
                                "registry_endpoint": repository,
                                "registry": registry,
                                "platform": platform,
                                "status": snapshot.status.as_str(),
                                "error": err,
                            })),
                        )
                        .await;
                    let failure_message =
                        format!("Proactive registry credential rotation failed: {err}");
                    insert_log(logger, server_id, &failure_message).await;
                    tracing::error!(
                        target: "registry.push",
                        %repository,
                        %server_id,
                        %platform,
                        error = %err,
                        "proactive registry credential rotation failed",
                    );
                }
            }
        } else {
            metrics
                .record(
                    "auth_rotation_skipped",
                    Some(json!({
                        "registry_endpoint": repository,
                        "registry": registry,
                        "platform": platform,
                        "status": snapshot.status.as_str(),
                        "reason": "refresher_unavailable",
                    })),
                )
                .await;
            insert_log(
                logger,
                server_id,
                "Registry credentials flagged for rotation but no refresher is configured",
            )
            .await;
            tracing::warn!(
                target: "registry.push",
                %repository,
                %server_id,
                %platform,
                "skipping proactive credential rotation: refresher unavailable",
            );
        }
    }

    (snapshot, rotation_attempted, rotation_succeeded)
}

fn is_retryable_push_error(err: &bollard::errors::Error) -> bool {
    use bollard::errors::Error;
    matches!(
        err,
        Error::IOError { .. }
            | Error::HyperResponseError { .. }
            | Error::HttpClientError { .. }
            | Error::RequestTimeoutError
    )
}

fn should_refresh_auth(detail_code: Option<i64>, message: Option<&str>) -> bool {
    matches!(detail_code, Some(401) | Some(403))
        || message
            .map(|msg| msg.contains("authentication required") || msg.contains("token has expired"))
            .unwrap_or(false)
}

fn classify_registry_push_error(err: &RegistryPushError) -> (&'static str, bool) {
    match err {
        RegistryPushError::AuthExpired(_) => ("auth_expired", true),
        RegistryPushError::Remote(_) => ("remote", false),
        RegistryPushError::Tag(_) => ("tag", false),
        RegistryPushError::Push(_) => ("push", false),
    }
}

fn extract_digest(line: &str) -> Option<String> {
    line.split("digest:")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .map(|digest| digest.trim_matches(','))
        .filter(|digest| digest.starts_with("sha256:"))
        .map(ToString::to_string)
}

fn dockerfile_exposes_8080(content: &str) -> bool {
    let re = Regex::new(r"(?i)^\s*EXPOSE\s+(\d+)").expect("invalid regex");
    for line in content.lines() {
        if let Some(cap) = re.captures(line) {
            if let Some(port) = cap.get(1) {
                if port.as_str() == "8080" {
                    return true;
                }
            }
        }
    }
    false
}

async fn detect_builder(path: &Path) -> Option<LangBuilder> {
    if fs::metadata(path.join("package.json")).await.is_ok() {
        return Some(LangBuilder::Node);
    }
    if fs::metadata(path.join("requirements.txt")).await.is_ok()
        || fs::metadata(path.join("pyproject.toml")).await.is_ok()
    {
        return Some(LangBuilder::Python);
    }
    if fs::metadata(path.join("Cargo.toml")).await.is_ok() {
        return Some(LangBuilder::Rust);
    }
    None
}

async fn generate_dockerfile(path: &Path, builder: LangBuilder) -> std::io::Result<()> {
    let contents = match builder {
        LangBuilder::Node => {
            "FROM node:18\nWORKDIR /app\nCOPY . .\nRUN npm install\nEXPOSE 8080\nCMD [\"npm\", \"start\"]".to_string()
        }
        LangBuilder::Python => {
            "FROM python:3.11\nWORKDIR /app\nCOPY . .\nRUN pip install -r requirements.txt || true\nEXPOSE 8080\nCMD [\"python\", \"main.py\"]".to_string()
        }
        LangBuilder::Rust => {
            "FROM rust:1.75 AS build\nWORKDIR /app\nCOPY . .\nRUN cargo install --path .\nFROM debian:buster-slim\nCOPY --from=build /usr/local/cargo/bin/* /app/\nEXPOSE 8080\nCMD [\"/app/mcp-server\"]".to_string()
        }
    };
    fs::write(path.join("Dockerfile"), contents).await
}

#[async_trait]
trait BuildLogSink: Send + Sync {
    async fn log(&self, server_id: i32, text: &str);
}

#[async_trait]
impl BuildLogSink for PgPool {
    async fn log(&self, server_id: i32, text: &str) {
        let _ = sqlx::query("INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)")
            .bind(server_id)
            .bind(text)
            .execute(self)
            .await;
    }
}

#[async_trait]
trait MetricRecorder: Send + Sync {
    async fn record(&self, event_type: &str, details: Option<Value>);
}

type CredentialRefreshFuture = Pin<Box<dyn Future<Output = Result<Docker, String>> + Send>>;
type CredentialRefreshFn = Box<dyn FnMut() -> CredentialRefreshFuture + Send>;

#[async_trait]
trait RegistryAuthRefresher: Send {
    async fn refresh(&mut self) -> Result<(), String>;
}

struct SharedDockerRefresher {
    shared: Arc<Mutex<Docker>>,
    refresh_fn: CredentialRefreshFn,
}

impl SharedDockerRefresher {
    fn new(shared: Arc<Mutex<Docker>>, refresh_fn: CredentialRefreshFn) -> Self {
        Self { shared, refresh_fn }
    }
}

#[async_trait]
impl RegistryAuthRefresher for SharedDockerRefresher {
    async fn refresh(&mut self) -> Result<(), String> {
        let new_docker = (self.refresh_fn)().await?;
        let mut guard = self
            .shared
            .lock()
            .expect("docker client mutex poisoned during auth refresh");
        *guard = new_docker;
        Ok(())
    }
}

struct UsageMetricRecorder<'a> {
    pool: &'a PgPool,
    server_id: i32,
}

#[async_trait]
impl<'a> MetricRecorder for UsageMetricRecorder<'a> {
    async fn record(&self, event_type: &str, details: Option<Value>) {
        let owned_details = details;
        if let Err(err) = add_metric(
            self.pool,
            self.server_id,
            event_type,
            owned_details.as_ref(),
        )
        .await
        {
            match err {
                MetricError::Database(db_err) => {
                    tracing::warn!(
                        target: "registry.push.metrics",
                        event_type = %event_type,
                        server_id = self.server_id,
                        error = %db_err,
                        "failed to persist registry metric"
                    );
                }
                MetricError::Validation(validation) => {
                    tracing::error!(
                        target: "registry.push.metrics",
                        event_type = %event_type,
                        server_id = self.server_id,
                        error = %validation,
                        "registry metric validation failed"
                    );
                }
            }
        }
    }
}

async fn record_push_failure<M: MetricRecorder + ?Sized>(
    metrics: &M,
    registry_endpoint: &str,
    attempt: usize,
    retry_limit: usize,
    error_kind: &str,
    error_message: &str,
    auth_expired: bool,
    platform: &str,
) {
    // telemetry: registry_push_failure
    metrics
        .record(
            "push_failed",
            Some(json!({
                "attempt": attempt,
                "retry_limit": retry_limit,
                "registry_endpoint": registry_endpoint,
                "error": error_message,
                "error_kind": error_kind,
                "auth_expired": auth_expired,
                "platform": platform,
            })),
        )
        .await;
}

async fn stream_push_progress<L, S>(
    logger: &L,
    server_id: i32,
    registry_endpoint: &str,
    scopes: &[String],
    mut stream: S,
) -> Result<Option<String>, RegistryPushError>
where
    L: BuildLogSink + ?Sized,
    S: futures_util::Stream<Item = Result<PushImageInfo, bollard::errors::Error>> + Unpin,
{
    let mut last_digest: Option<String> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(info) => {
                if let Some(detail) = info.error_detail {
                    let detail_code = detail.code;
                    let detail_message = detail.message.clone();
                    let base_message = detail_message
                        .as_deref()
                        .unwrap_or("Unknown registry error")
                        .to_string();
                    if should_refresh_auth(detail_code, detail_message.as_deref()) {
                        return Err(RegistryPushError::AuthExpired(base_message));
                    }
                    let message = if let Some(code) = detail_code {
                        format!("{base_message} (code {code})")
                    } else {
                        base_message
                    };
                    tracing::error!(
                        target: "registry.push",
                        %registry_endpoint,
                        %server_id,
                        scopes = ?scopes,
                        %message,
                        "registry error detail",
                    );
                    return Err(RegistryPushError::Remote(message));
                }
                if let Some(error) = info.error {
                    tracing::error!(
                        target: "registry.push",
                        %registry_endpoint,
                        %server_id,
                        scopes = ?scopes,
                        %error,
                        "registry returned error",
                    );
                    return Err(RegistryPushError::Remote(error));
                }
                if let Some(status) = info.status {
                    let mut line = status.trim().to_string();
                    if let Some(progress) = info.progress {
                        let progress = progress.trim();
                        if !progress.is_empty() {
                            if !line.is_empty() {
                                line.push(' ');
                            }
                            line.push_str(progress);
                        }
                    }
                    if !line.is_empty() {
                        if let Some(digest) = extract_digest(&line) {
                            let digest_message = format!("Manifest published with digest {digest}");
                            tracing::info!(
                                target: "registry.push",
                                %registry_endpoint,
                                %server_id,
                                scopes = ?scopes,
                                %digest,
                                "registry reported digest",
                            );
                            last_digest = Some(digest);
                            insert_log(logger, server_id, &digest_message).await;
                        }
                        tracing::info!(
                            target: "registry.push",
                            %registry_endpoint,
                            %server_id,
                            scopes = ?scopes,
                            status = %line,
                            "registry push status",
                        );
                        insert_log(logger, server_id, &line).await;
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    target: "registry.push",
                    %registry_endpoint,
                    %server_id,
                    scopes = ?scopes,
                    error = %err,
                    "registry push stream error",
                );
                return Err(RegistryPushError::Push(err));
            }
        }
    }

    Ok(last_digest)
}

#[derive(Debug)]
struct PushStreamOutcome {
    auth_refresh_attempted: bool,
    auth_refresh_succeeded: bool,
}

#[derive(Debug)]
struct PushStreamResult {
    outcome: PushStreamOutcome,
    digest: Option<String>,
}

async fn push_stream_with_retry<L, M, F, S>(
    logger: &L,
    metrics: &M,
    server_id: i32,
    registry_endpoint: &str,
    scopes: &[String],
    mut make_stream: F,
    retry_limit: usize,
    platform: &str,
    mut auth_refresher: Option<&mut dyn RegistryAuthRefresher>,
) -> Result<PushStreamResult, RegistryPushError>
where
    L: BuildLogSink + ?Sized,
    M: MetricRecorder + ?Sized,
    F: FnMut() -> S,
    S: futures_util::Stream<Item = Result<PushImageInfo, bollard::errors::Error>> + Unpin,
{
    let mut refresh_attempted = false;
    let mut refresh_succeeded = false;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if attempt == 1 {
            metrics
                .record(
                    "push_started",
                    Some(json!({
                        "attempt": attempt,
                        "retry_limit": retry_limit,
                        "registry_endpoint": registry_endpoint,
                        "platform": platform,
                    })),
                )
                .await;
        }
        tracing::info!(
            target: "registry.push",
            %registry_endpoint,
            %server_id,
            scopes = ?scopes,
            attempt,
            retry_limit,
            platform = %platform,
            "starting registry push attempt",
        );
        insert_log(
            logger,
            server_id,
            &format!("Registry push attempt {attempt}/{retry_limit} for {registry_endpoint}"),
        )
        .await;

        match stream_push_progress(logger, server_id, registry_endpoint, scopes, make_stream())
            .await
        {
            Ok(digest) => {
                metrics
                    .record(
                        "push_succeeded",
                        Some(json!({
                            "attempt": attempt,
                            "retry_limit": retry_limit,
                            "registry_endpoint": registry_endpoint,
                            "platform": platform,
                        })),
                    )
                    .await;
                tracing::info!(
                    target: "registry.push",
                    %registry_endpoint,
                    %server_id,
                    scopes = ?scopes,
                    attempt,
                    retry_limit,
                    platform = %platform,
                    "registry push succeeded",
                );
                insert_log(
                    logger,
                    server_id,
                    &format!(
                        "Registry push succeeded after {attempt} attempt(s) for {registry_endpoint}"
                    ),
                )
                .await;
                return Ok(PushStreamOutcome {
                    auth_refresh_attempted: refresh_attempted,
                    auth_refresh_succeeded: refresh_succeeded,
                }
                .into_with_digest(digest));
            }
            Err(err) => {
                let (error_kind, auth_expired) = classify_registry_push_error(&err);
                let error_message = err.to_string();
                let retryable = matches!(
                    &err,
                    RegistryPushError::Push(inner) | RegistryPushError::Tag(inner)
                        if is_retryable_push_error(inner)
                );
                record_push_failure(
                    metrics,
                    registry_endpoint,
                    attempt,
                    retry_limit,
                    error_kind,
                    &error_message,
                    auth_expired,
                    platform,
                )
                .await;
                tracing::error!(
                    target: "registry.push",
                    %registry_endpoint,
                    %server_id,
                    scopes = ?scopes,
                    attempt,
                    retry_limit,
                    platform = %platform,
                    error = %error_message,
                    "registry push failed",
                );

                if auth_expired {
                    refresh_attempted = true;
                    if let Some(refresher) = auth_refresher.as_mut() {
                        insert_log(
                            logger,
                            server_id,
                            "Refreshing registry credentials after authentication expiry",
                        )
                        .await;
                        tracing::warn!(
                            target: "registry.push",
                            %registry_endpoint,
                            %server_id,
                            scopes = ?scopes,
                            attempt,
                            retry_limit,
                            platform = %platform,
                            error = %error_message,
                            "registry authentication expired; attempting credential refresh",
                        );

                        metrics
                            .record(
                                "auth_refresh_started",
                                Some(json!({
                                    "attempt": attempt,
                                    "retry_limit": retry_limit,
                                    "registry_endpoint": registry_endpoint,
                                    "platform": platform,
                                })),
                            )
                            .await;

                        match refresher.refresh().await {
                            Ok(()) => {
                                refresh_succeeded = true;
                                metrics
                                    .record(
                                        "auth_refresh_succeeded",
                                        Some(json!({
                                            "attempt": attempt,
                                            "retry_limit": retry_limit,
                                            "registry_endpoint": registry_endpoint,
                                            "platform": platform,
                                        })),
                                    )
                                    .await;
                                insert_log(
                                    logger,
                                    server_id,
                                    "Registry credentials refreshed; retrying push",
                                )
                                .await;
                                tracing::info!(
                                    target: "registry.push",
                                    %registry_endpoint,
                                    %server_id,
                                    scopes = ?scopes,
                                    attempt,
                                    retry_limit,
                                    platform = %platform,
                                    "registry auth refresh succeeded; retrying push",
                                );
                                metrics
                                    .record(
                                        "push_retry",
                                        Some(json!({
                                            "attempt": attempt,
                                            "retry_limit": retry_limit,
                                            "registry_endpoint": registry_endpoint,
                                            "reason": "auth_refresh",
                                            "error": error_message,
                                            "platform": platform,
                                        })),
                                    )
                                    .await;
                                let backoff = TokioDuration::from_millis(100 * attempt as u64);
                                sleep(backoff).await;
                                continue;
                            }
                            Err(refresh_err) => {
                                metrics
                                    .record(
                                        "auth_refresh_failed",
                                        Some(json!({
                                            "attempt": attempt,
                                            "retry_limit": retry_limit,
                                            "registry_endpoint": registry_endpoint,
                                            "error": refresh_err,
                                            "platform": platform,
                                        })),
                                    )
                                    .await;
                                insert_log(
                                    logger,
                                    server_id,
                                    "Registry auth refresh failed; aborting push",
                                )
                                .await;
                                tracing::error!(
                                    target: "registry.push",
                                    %registry_endpoint,
                                    %server_id,
                                    scopes = ?scopes,
                                    attempt,
                                    retry_limit,
                                    platform = %platform,
                                    error = %refresh_err,
                                    "registry auth refresh failed",
                                );
                                let err = RegistryPushError::AuthExpired(format!(
                                    "{error_message}; auth refresh failed: {refresh_err}"
                                ));
                                let (error_kind, auth_expired) = classify_registry_push_error(&err);
                                let error_message = err.to_string();
                                record_push_failure(
                                    metrics,
                                    registry_endpoint,
                                    attempt,
                                    retry_limit,
                                    error_kind,
                                    &error_message,
                                    auth_expired,
                                    platform,
                                )
                                .await;
                                tracing::error!(
                                    target: "registry.push",
                                    %registry_endpoint,
                                    %server_id,
                                    scopes = ?scopes,
                                    attempt,
                                    retry_limit,
                                    platform = %platform,
                                    error = %error_message,
                                    "registry push failed",
                                );
                                return Err(err);
                            }
                        }
                    } else {
                        let err = RegistryPushError::AuthExpired(error_message);
                        let (error_kind, auth_expired) = classify_registry_push_error(&err);
                        let error_message = err.to_string();
                        record_push_failure(
                            metrics,
                            registry_endpoint,
                            attempt,
                            retry_limit,
                            error_kind,
                            &error_message,
                            auth_expired,
                            platform,
                        )
                        .await;
                        tracing::error!(
                            target: "registry.push",
                            %registry_endpoint,
                            %server_id,
                            scopes = ?scopes,
                            attempt,
                            retry_limit,
                            platform = %platform,
                            error = %error_message,
                            "registry push failed",
                        );
                        return Err(err);
                    }
                } else {
                    if attempt < retry_limit && retryable {
                        metrics
                            .record(
                                "push_retry",
                                Some(json!({
                                    "attempt": attempt,
                                    "retry_limit": retry_limit,
                                    "registry_endpoint": registry_endpoint,
                                    "reason": "retryable_error",
                                    "error": error_message,
                                    "platform": platform,
                                })),
                            )
                            .await;
                        let backoff = TokioDuration::from_millis(100 * attempt as u64);
                        sleep(backoff).await;
                        continue;
                    }

                    return Err(err);
                }
            }
        }
    }
}

impl PushStreamOutcome {
    fn into_with_digest(self, digest: Option<String>) -> PushStreamResult {
        PushStreamResult {
            outcome: self,
            digest,
        }
    }
}

pub struct RegistryPushResult {
    pub image: String,
    pub remote_tag: String,
    pub digest: Option<String>,
    pub platform: String,
    pub auth_refresh_attempted: bool,
    pub auth_refresh_succeeded: bool,
    pub auth_rotation_attempted: bool,
    pub auth_rotation_succeeded: bool,
    pub credential_health_status: CredentialHealthStatus,
}

async fn push_image_to_registry<L: BuildLogSink + ?Sized>(
    pool: &PgPool,
    logger: &L,
    docker: &Docker,
    server_id: i32,
    local_image: &str,
    registry: &str,
    remote_image_name: &str,
    remote_tag: &str,
    platform: &str,
    credential_refresher: Option<CredentialRefreshFn>,
) -> Result<RegistryPushResult, RegistryPushError> {
    let reference = build_registry_reference(registry, remote_image_name, remote_tag);
    let scopes = registry_scopes(&reference.repository);
    insert_log(
        logger,
        server_id,
        &format!("Tagging image as {}", reference.display_name()),
    )
    .await;
    tracing::info!(
        target: "registry.push",
        registry_endpoint = %reference.repository,
        %server_id,
        scopes = ?scopes,
        tag = %reference.tag,
        "tagging image for registry push"
    );

    let retry_limit = registry_push_retry_limit();
    let usage_metrics = UsageMetricRecorder { pool, server_id };

    usage_metrics
        .record(
            "tag_started",
            Some(json!({
                "registry_endpoint": &reference.repository,
                "tag": &reference.tag,
                "platform": platform,
            })),
        )
        .await;

    let tag_opts = TagImageOptionsBuilder::new()
        .repo(&reference.repository)
        .tag(&reference.tag)
        .build();
    if let Err(err) = docker.tag_image(local_image, Some(tag_opts)).await {
        let error_message = err.to_string();
        record_push_failure(
            &usage_metrics,
            &reference.repository,
            0,
            retry_limit,
            "tag",
            &error_message,
            false,
            platform,
        )
        .await;
        tracing::error!(
            target: "registry.push",
            registry_endpoint = %reference.repository,
            %server_id,
            scopes = ?scopes,
            tag = %reference.tag,
            error = %error_message,
            "failed to tag image for registry push",
        );
        insert_log(
            logger,
            server_id,
            &format!("Failed to tag image for registry push: {error_message}"),
        )
        .await;
        return Err(RegistryPushError::Tag(err));
    }

    usage_metrics
        .record(
            "tag_succeeded",
            Some(json!({
                "registry_endpoint": &reference.repository,
                "tag": &reference.tag,
                "platform": platform,
            })),
        )
        .await;

    insert_log(logger, server_id, "Pushing image to registry").await;
    tracing::info!(
        target: "registry.push",
        registry_endpoint = %reference.repository,
        %server_id,
        scopes = ?scopes,
        tag = %reference.tag,
        "starting registry push"
    );

    let shared_docker = Arc::new(Mutex::new(docker.clone()));
    let repository_for_stream = reference.repository.clone();
    let tag_for_stream = reference.tag.clone();
    let mut refresh_context = credential_refresher
        .map(|refresh_fn| SharedDockerRefresher::new(Arc::clone(&shared_docker), refresh_fn));

    let (health_snapshot, proactive_rotation_attempted, proactive_rotation_succeeded) =
        record_credential_health_and_rotate(
            logger,
            &usage_metrics,
            server_id,
            registry,
            &reference.repository,
            platform,
            refresh_context
                .as_mut()
                .map(|context| context as &mut dyn RegistryAuthRefresher),
        )
        .await;
    let credential_health_status = health_snapshot.status;

    let push_result = push_stream_with_retry(
        logger,
        &usage_metrics,
        server_id,
        &reference.repository,
        &scopes,
        {
            let shared = Arc::clone(&shared_docker);
            move || {
                let client = {
                    let guard = shared
                        .lock()
                        .expect("docker client mutex poisoned during push stream creation");
                    guard.clone()
                };
                let push_opts = PushImageOptionsBuilder::new().tag(&tag_for_stream).build();
                client.push_image(&repository_for_stream, Some(push_opts), None)
            }
        },
        retry_limit,
        platform,
        refresh_context
            .as_mut()
            .map(|context| context as &mut dyn RegistryAuthRefresher),
    )
    .await?;
    let outcome = push_result.outcome;
    let digest = push_result.digest;
    insert_log(logger, server_id, "Image pushed to registry").await;
    tracing::info!(
        target: "registry.push",
        registry_endpoint = %reference.repository,
        %server_id,
        scopes = ?scopes,
        platform = %platform,
        digest = ?digest,
        "registry push completed",
    );

    Ok(RegistryPushResult {
        image: reference.display_name(),
        remote_tag: reference.tag,
        digest,
        platform: platform.to_string(),
        auth_refresh_attempted: outcome.auth_refresh_attempted,
        auth_refresh_succeeded: outcome.auth_refresh_succeeded,
        auth_rotation_attempted: proactive_rotation_attempted,
        auth_rotation_succeeded: proactive_rotation_succeeded,
        credential_health_status,
    })
}

async fn insert_log<L: BuildLogSink + ?Sized>(logger: &L, server_id: i32, text: &str) {
    logger.log(server_id, text).await;
}

async fn set_status_or_log(
    pool: &PgPool,
    server_id: i32,
    status: &str,
) -> Result<(), SetStatusError> {
    match set_status(pool, server_id, status).await {
        Ok(()) => Ok(()),
        Err(err) => {
            tracing::error!(
                ?err,
                %server_id,
                status = %status,
                "failed to update server status after build operation"
            );
            Err(err)
        }
    }
}

pub struct BuildArtifacts {
    pub local_image: String,
    pub registry_image: Option<String>,
    pub auth_refresh_attempted: bool,
    pub auth_refresh_succeeded: bool,
    pub auth_rotation_attempted: bool,
    pub auth_rotation_succeeded: bool,
    pub credential_health_status: CredentialHealthStatus,
}

/// Clone a git repository and build a Docker image.
/// Returns the build artifacts on success.
pub async fn build_from_git(
    pool: &PgPool,
    server_id: i32,
    repo_url: &str,
    branch: Option<&str>,
) -> Result<Option<BuildArtifacts>, SetStatusError> {
    insert_log(pool, server_id, "Cloning repository").await;
    let tmp = match tempdir() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(?e, "tempdir failed");
            set_status_or_log(pool, server_id, "error").await?;
            insert_log(pool, server_id, "Failed to create build dir").await;
            return Ok(None);
        }
    };

    let repo = repo_url.to_string();
    let br_opt = branch.map(|s| s.to_string());
    let clone_path = tmp.path().to_path_buf();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let mut builder = git2::build::RepoBuilder::new();
        if let Some(ref br) = br_opt {
            builder.branch(br);
        }
        builder.clone(&repo, &clone_path).map(|_| ())
    })
    .await
    .unwrap_or_else(|e| Err(git2::Error::from_str(&e.to_string())))
    {
        tracing::error!(?e, "git clone failed");
        insert_log(pool, server_id, "Git clone failed").await;
        set_status_or_log(pool, server_id, "error").await?;
        return Ok(None);
    }

    // Generate a Dockerfile when none exists using a simple language-specific template
    let dockerfile = tmp.path().join("Dockerfile");
    if fs::metadata(&dockerfile).await.is_err() {
        if let Some(builder) = detect_builder(tmp.path()).await {
            insert_log(pool, server_id, "No Dockerfile found, generating one").await;
            if let Err(e) = generate_dockerfile(tmp.path(), builder).await {
                tracing::error!(?e, "failed to write Dockerfile");
            }
        }
    }

    insert_log(pool, server_id, "Building image").await;
    let base_name = format!("mcp-custom-{server_id}");
    let manifest_tag = "latest";
    let platform_targets = desired_platform_targets();
    let registry_env = std::env::var("REGISTRY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut registry_image = None;
    let mut auth_refresh_attempted = false;
    let mut auth_refresh_succeeded = false;
    let mut auth_rotation_attempted = false;
    let mut auth_rotation_succeeded = false;
    let mut credential_health_status = CredentialHealthStatus::Unknown;
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(?e, "Failed to connect to Docker");
            insert_log(pool, server_id, "Docker connection failed").await;
            set_status_or_log(pool, server_id, "error").await?;
            return Ok(None);
        }
    };

    let ctx_path = tmp.path().to_path_buf();
    let tar_res = tokio::task::spawn_blocking(move || {
        let mut builder = TarBuilder::new(Vec::new());
        builder
            .append_dir_all(".", &ctx_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        builder.into_inner().map(Bytes::from)
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::new(std::io::ErrorKind::Other, e)));

    let tar_data = match tar_res {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(?e, "Failed to create tar");
            insert_log(pool, server_id, "Failed to create build context").await;
            set_status_or_log(pool, server_id, "error").await?;
            return Ok(None);
        }
    };

    let mut platform_pushes: Vec<(PlatformTarget, RegistryPushResult)> = Vec::new();
    let mut local_alias_created = false;

    for target in &platform_targets {
        let local_tag = format!("{base_name}-{}", target.slug);
        let build_options = BuildImageOptions::<String> {
            dockerfile: "Dockerfile".into(),
            t: local_tag.clone(),
            pull: true,
            nocache: true,
            rm: true,
            forcerm: true,
            platform: target.spec.clone(),
            ..Default::default()
        };

        let mut build_stream =
            docker.build_image(build_options, None, Some(body_full(tar_data.clone())));
        while let Some(item) = build_stream.next().await {
            match item {
                Ok(output) => {
                    if let Some(msg) = output.stream {
                        insert_log(pool, server_id, msg.trim()).await;
                    }
                }
                Err(e) => {
                    tracing::error!(?e, platform = %target.spec, "docker build error");
                    insert_log(pool, server_id, "Image build failed").await;
                    set_status_or_log(pool, server_id, "error").await?;
                    return Ok(None);
                }
            }
        }

        if !local_alias_created {
            let alias_opts = TagImageOptionsBuilder::new()
                .repo(&base_name)
                .tag("latest")
                .build();
            if let Err(err) = docker.tag_image(&local_tag, Some(alias_opts)).await {
                tracing::error!(?err, %server_id, platform = %target.spec, "failed to tag local image alias");
                insert_log(pool, server_id, "Failed to tag local image").await;
                set_status_or_log(pool, server_id, "error").await?;
                return Ok(None);
            }
            local_alias_created = true;
        }

        if let Some(registry) = registry_env.as_ref() {
            let remote_tag = if platform_targets.len() > 1 {
                format!("{manifest_tag}-{}", target.slug)
            } else {
                manifest_tag.to_string()
            };
            match push_image_to_registry(
                pool,
                pool,
                &docker,
                server_id,
                &local_tag,
                registry,
                &base_name,
                &remote_tag,
                &target.spec,
                None,
            )
            .await
            {
                Ok(result) => {
                    auth_refresh_attempted |= result.auth_refresh_attempted;
                    auth_refresh_succeeded |= result.auth_refresh_succeeded;
                    auth_rotation_attempted |= result.auth_rotation_attempted;
                    auth_rotation_succeeded |= result.auth_rotation_succeeded;
                    credential_health_status =
                        credential_health_status.combine(result.credential_health_status);
                    platform_pushes.push((target.clone(), result));
                }
                Err(err) => {
                    tracing::error!(
                        ?err,
                        registry = %registry,
                        platform = %target.spec,
                        %server_id,
                        "registry push failed"
                    );
                    insert_log(pool, server_id, &format!("Registry push failed: {err}")).await;
                    set_status_or_log(pool, server_id, "error").await?;
                    return Ok(None);
                }
            }
        }
    }

    if let Some(registry) = registry_env.as_ref() {
        if platform_pushes.len() > 1 {
            let mut manifest_inputs = Vec::new();
            for (target, result) in &platform_pushes {
                if let Some(digest) = result.digest.clone() {
                    manifest_inputs.push((target.clone(), digest));
                } else {
                    tracing::error!(platform = %target.spec, %server_id, "missing digest for manifest publish");
                    insert_log(pool, server_id, "Missing digest for manifest publish").await;
                    set_status_or_log(pool, server_id, "error").await?;
                    return Ok(None);
                }
            }
            let manifest_metrics = UsageMetricRecorder { pool, server_id };
            if let Err(err) = publish_manifest_list(
                pool,
                &manifest_metrics,
                server_id,
                registry,
                &base_name,
                manifest_tag,
                &manifest_inputs,
            )
            .await
            {
                tracing::error!(?err, registry = %registry, %server_id, "manifest publish failed");
                insert_log(pool, server_id, &format!("Manifest publish failed: {err}")).await;
                set_status_or_log(pool, server_id, "error").await?;
                return Ok(None);
            }
            registry_image = Some(format!(
                "{}/{}:{}",
                registry.trim_end_matches('/'),
                base_name,
                manifest_tag
            ));
        } else if let Some((_, result)) = platform_pushes.first() {
            registry_image = Some(result.image.clone());
        }
    }
    // Parse Dockerfile for EXPOSE instructions
    let dockerfile = tmp.path().join("Dockerfile");
    if let Ok(content) = tokio::fs::read_to_string(&dockerfile).await {
        if !dockerfile_exposes_8080(&content) {
            insert_log(pool, server_id, "Warning: no EXPOSE 8080 found").await;
        }
    }

    insert_log(pool, server_id, "Image built").await;

    insert_log(pool, server_id, "Cleaning up").await;
    Ok(Some(BuildArtifacts {
        local_image: base_name,
        registry_image,
        auth_refresh_attempted,
        auth_refresh_succeeded,
        auth_rotation_attempted,
        auth_rotation_succeeded,
        credential_health_status,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as Base64Standard;
    use base64::Engine;
    use bollard::models::PushImageInfo;
    use futures_util::stream;
    use httpmock::prelude::*;
    use tempfile::{tempdir, NamedTempFile};
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct RecordingLog {
        entries: Mutex<Vec<String>>,
    }

    impl RecordingLog {
        async fn messages(&self) -> Vec<String> {
            self.entries.lock().await.clone()
        }
    }

    #[derive(Default)]
    struct RecordingMetrics {
        entries: Mutex<Vec<(String, Option<Value>)>>,
    }

    impl RecordingMetrics {
        async fn events(&self) -> Vec<(String, Option<Value>)> {
            self.entries.lock().await.clone()
        }
    }

    struct TestRefresher {
        succeed: bool,
        attempts: usize,
        failure_message: String,
    }

    impl TestRefresher {
        fn succeed() -> Self {
            Self {
                succeed: true,
                attempts: 0,
                failure_message: String::new(),
            }
        }

        fn fail(message: &str) -> Self {
            Self {
                succeed: false,
                attempts: 0,
                failure_message: message.to_string(),
            }
        }

        fn attempts(&self) -> usize {
            self.attempts
        }
    }

    #[async_trait]
    impl RegistryAuthRefresher for TestRefresher {
        async fn refresh(&mut self) -> Result<(), String> {
            self.attempts += 1;
            if self.succeed {
                Ok(())
            } else {
                Err(self.failure_message.clone())
            }
        }
    }

    #[async_trait]
    impl BuildLogSink for RecordingLog {
        async fn log(&self, _server_id: i32, text: &str) {
            self.entries.lock().await.push(text.to_string());
        }
    }

    #[async_trait]
    impl MetricRecorder for RecordingMetrics {
        async fn record(&self, event_type: &str, details: Option<Value>) {
            self.entries
                .lock()
                .await
                .push((event_type.to_string(), details));
        }
    }

    #[tokio::test]
    async fn detect_builder_works() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}")
            .await
            .unwrap();
        assert!(matches!(
            detect_builder(dir.path()).await,
            Some(LangBuilder::Node)
        ));
    }

    #[tokio::test]
    async fn generates_dockerfile() {
        let dir = tempdir().unwrap();
        generate_dockerfile(dir.path(), LangBuilder::Python)
            .await
            .unwrap();
        let contents = fs::read_to_string(dir.path().join("Dockerfile"))
            .await
            .unwrap();
        assert!(contents.contains("python"));
    }

    #[test]
    fn exposes_check() {
        let dockerfile = "FROM scratch\nEXPOSE 8080";
        assert!(dockerfile_exposes_8080(dockerfile));
        let other = "FROM scratch\nEXPOSE 5000";
        assert!(!dockerfile_exposes_8080(other));
    }

    #[test]
    fn registry_reference_formats_path() {
        let reference = build_registry_reference("example.com/org/", "app", "latest");
        assert_eq!(reference.repository, "example.com/org/app");
        assert_eq!(reference.tag, "latest");
        assert_eq!(reference.display_name(), "example.com/org/app:latest");
    }

    #[tokio::test]
    async fn stream_progress_logs_status_updates() {
        let logger = RecordingLog::default();
        let entries = vec![
            Ok(PushImageInfo {
                status: Some("Preparing".to_string()),
                progress: Some("1/2".to_string()),
                ..Default::default()
            }),
            Ok(PushImageInfo {
                status: Some("Done".to_string()),
                ..Default::default()
            }),
        ];

        stream_push_progress(
            &logger,
            7,
            "test/example",
            &["scope".to_string()],
            stream::iter(entries),
        )
        .await
        .expect("stream should complete");

        let messages = logger.messages().await;
        assert!(messages.iter().any(|m| m.contains("Preparing 1/2")));
        assert!(messages.iter().any(|m| m.contains("Done")));
    }

    #[tokio::test]
    async fn stream_progress_reports_remote_error() {
        let logger = RecordingLog::default();
        let entries = vec![Ok(PushImageInfo {
            error: Some("denied".to_string()),
            ..Default::default()
        })];

        let err = stream_push_progress(
            &logger,
            7,
            "test/example",
            &["scope".to_string()],
            stream::iter(entries),
        )
        .await
        .expect_err("expected remote error");

        match err {
            RegistryPushError::Remote(msg) => assert!(msg.contains("denied")),
            other => panic!("expected remote error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn stream_progress_reports_error_detail_code() {
        let logger = RecordingLog::default();
        let mut info = PushImageInfo::default();
        info.error_detail = Some(bollard::models::ErrorDetail {
            code: Some(401),
            message: Some("authentication required".to_string()),
        });
        let err = stream_push_progress(
            &logger,
            7,
            "test/example",
            &["scope".to_string()],
            stream::iter(vec![Ok(info)]),
        )
        .await
        .expect_err("expected detail error");

        match err {
            RegistryPushError::AuthExpired(msg) => assert!(msg.contains("authentication required")),
            other => panic!("expected remote error detail, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn stream_progress_logs_digest_from_status() {
        let logger = RecordingLog::default();
        stream_push_progress(
            &logger,
            42,
            "registry.test/example",
            &["scope".to_string()],
            stream::iter(vec![Ok(PushImageInfo {
                status: Some("latest: digest: sha256:abc123 size: 123".to_string()),
                ..Default::default()
            })]),
        )
        .await
        .expect("digest status should succeed");

        let messages = logger.messages().await;
        assert!(messages
            .iter()
            .any(|m| m.contains("Manifest published with digest sha256:abc123")));
    }

    #[tokio::test]
    async fn retryable_push_errors_are_retried() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let scopes = vec!["scope".to_string()];

        let outcome = push_stream_with_retry(
            &logger,
            &metrics,
            99,
            "registry.test/example",
            &scopes,
            move || {
                let attempt = counter_clone.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    stream::iter(vec![Err(bollard::errors::Error::IOError {
                        err: std::io::Error::new(std::io::ErrorKind::Interrupted, "network hiccup"),
                    })])
                } else {
                    stream::iter(vec![Ok(PushImageInfo {
                        status: Some("Done".to_string()),
                        ..Default::default()
                    })])
                }
            },
            3,
            "linux/amd64",
            None,
        )
        .await
        .expect("retry should eventually succeed");

        assert!(!outcome.outcome.auth_refresh_attempted);
        assert!(!outcome.outcome.auth_refresh_succeeded);

        assert_eq!(counter.load(Ordering::SeqCst), 2);
        let events = metrics.events().await;
        assert!(events.iter().any(|(event, _)| event == "push_started"));
        let retry_events: Vec<_> = events
            .iter()
            .filter(|(event, _)| event == "push_retry")
            .collect();
        assert_eq!(retry_events.len(), 1);
        assert!(events.iter().any(|(event, _)| event == "push_succeeded"));
        let success_details = events
            .iter()
            .find(|(event, _)| event == "push_succeeded")
            .and_then(|(_, details)| details.as_ref());
        assert_eq!(
            success_details
                .and_then(|value| value.get("attempt"))
                .and_then(Value::as_u64),
            Some(2)
        );
    }

    #[tokio::test]
    async fn non_retryable_push_errors_bubble() {
        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let scopes = vec!["scope".to_string()];
        let err = push_stream_with_retry(
            &logger,
            &metrics,
            100,
            "registry.test/example",
            &scopes,
            || {
                stream::iter(vec![Ok(PushImageInfo {
                    error: Some("denied".to_string()),
                    ..Default::default()
                })])
            },
            2,
            "linux/amd64",
            None,
        )
        .await
        .expect_err("remote error should bubble");

        assert!(matches!(err, RegistryPushError::Remote(_)));
        let events = metrics.events().await;
        assert!(events.iter().any(|(event, _)| event == "push_started"));
        let failure_details = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed event should include details");
        assert_eq!(
            failure_details.get("error_kind").and_then(Value::as_str),
            Some("remote")
        );
        assert_eq!(
            failure_details.get("auth_expired").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[tokio::test]
    async fn auth_expired_records_failed_metric() {
        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let scopes = vec!["scope".to_string()];
        let err = push_stream_with_retry(
            &logger,
            &metrics,
            7,
            "registry.test/example",
            &scopes,
            || {
                let mut info = PushImageInfo::default();
                info.error_detail = Some(bollard::models::ErrorDetail {
                    code: Some(401),
                    message: Some("authentication required".to_string()),
                });
                stream::iter(vec![Ok(info)])
            },
            2,
            "linux/amd64",
            None,
        )
        .await
        .expect_err("auth expired should bubble");

        assert!(matches!(err, RegistryPushError::AuthExpired(_)));
        let events = metrics.events().await;
        let failed_details = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed details present");
        assert_eq!(
            failed_details.get("auth_expired").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn auth_expired_refresh_retries_and_succeeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let scopes = vec!["scope".to_string()];
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let mut refresher = TestRefresher::succeed();

        let outcome = push_stream_with_retry(
            &logger,
            &metrics,
            21,
            "registry.test/example",
            &scopes,
            move || {
                let attempt = counter_clone.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    let mut info = PushImageInfo::default();
                    info.error_detail = Some(bollard::models::ErrorDetail {
                        code: Some(401),
                        message: Some("authentication required".to_string()),
                    });
                    stream::iter(vec![Ok(info)])
                } else {
                    stream::iter(vec![Ok(PushImageInfo {
                        status: Some("Done".to_string()),
                        ..Default::default()
                    })])
                }
            },
            3,
            "linux/amd64",
            Some(&mut refresher as &mut dyn RegistryAuthRefresher),
        )
        .await
        .expect("refresh should allow push to succeed");

        assert!(outcome.outcome.auth_refresh_attempted);
        assert!(outcome.outcome.auth_refresh_succeeded);

        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert_eq!(refresher.attempts(), 1);

        let events = metrics.events().await;
        let started = events
            .iter()
            .find(|(event, _)| event == "auth_refresh_started")
            .and_then(|(_, details)| details.as_ref())
            .expect("auth_refresh_started recorded");
        assert_eq!(started.get("attempt").and_then(Value::as_u64), Some(1));
        let succeeded = events
            .iter()
            .find(|(event, _)| event == "auth_refresh_succeeded")
            .and_then(|(_, details)| details.as_ref())
            .expect("auth_refresh_succeeded recorded");
        assert_eq!(succeeded.get("attempt").and_then(Value::as_u64), Some(1));
        let retry_details = events
            .iter()
            .find(|(event, _)| event == "push_retry")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_retry recorded");
        assert_eq!(
            retry_details.get("reason").and_then(Value::as_str),
            Some("auth_refresh")
        );
        assert_eq!(
            retry_details.get("attempt").and_then(Value::as_u64),
            Some(1)
        );
        let success_details = events
            .iter()
            .find(|(event, _)| event == "push_succeeded")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_succeeded recorded");
        assert_eq!(
            success_details
                .get("attempt")
                .and_then(Value::as_u64)
                .map(|value| value as usize),
            Some(2),
        );
    }

    #[tokio::test]
    async fn auth_refresh_failure_records_metrics() {
        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let scopes = vec!["scope".to_string()];
        let mut refresher = TestRefresher::fail("token refresh failure");

        let err = push_stream_with_retry(
            &logger,
            &metrics,
            22,
            "registry.test/example",
            &scopes,
            || {
                let mut info = PushImageInfo::default();
                info.error_detail = Some(bollard::models::ErrorDetail {
                    code: Some(401),
                    message: Some("authentication required".to_string()),
                });
                stream::iter(vec![Ok(info)])
            },
            2,
            "linux/amd64",
            Some(&mut refresher as &mut dyn RegistryAuthRefresher),
        )
        .await
        .expect_err("refresh failure should bubble");

        assert!(
            matches!(err, RegistryPushError::AuthExpired(message) if message.contains("token refresh failure"))
        );
        assert_eq!(refresher.attempts(), 1);

        let events = metrics.events().await;
        assert!(events
            .iter()
            .any(|(event, _)| event == "auth_refresh_started"));
        let failed_details = events
            .iter()
            .find(|(event, _)| event == "auth_refresh_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("auth_refresh_failed recorded");
        assert_eq!(
            failed_details.get("error").and_then(Value::as_str),
            Some("token refresh failure")
        );
        let failure_entry = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed recorded");
        assert_eq!(
            failure_entry.get("auth_expired").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            failure_entry.get("attempt").and_then(Value::as_u64),
            Some(1)
        );
    }

    #[tokio::test]
    async fn record_push_failure_helper_serializes_context() {
        let metrics = RecordingMetrics::default();

        record_push_failure(
            &metrics,
            "registry.test/example",
            0,
            5,
            "tag",
            "simulated failure",
            false,
            "linux/amd64",
        )
        .await;

        let events = metrics.events().await;
        let failure_entry = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed metric recorded");

        assert_eq!(
            failure_entry
                .get("registry_endpoint")
                .and_then(Value::as_str),
            Some("registry.test/example")
        );
        assert_eq!(
            failure_entry.get("attempt").and_then(Value::as_u64),
            Some(0)
        );
        assert_eq!(
            failure_entry.get("error_kind").and_then(Value::as_str),
            Some("tag")
        );
        assert_eq!(
            failure_entry.get("retry_limit").and_then(Value::as_u64),
            Some(5)
        );
        assert_eq!(
            failure_entry.get("auth_expired").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            failure_entry.get("platform").and_then(Value::as_str),
            Some("linux/amd64")
        );
    }

    #[tokio::test]
    async fn publish_manifest_list_pushes_payload() {
        let server = MockServer::start_async().await;
        let registry = format!("http://{}/demo", server.address());
        let auth_value = Base64Standard.encode("user:pass");
        let config = NamedTempFile::new().expect("temp docker config");
        std::fs::write(
            config.path(),
            format!(
                r#"{{"auths": {{"http://{}": {{"auth": "{}"}}}}}}"#,
                server.address(),
                auth_value
            ),
        )
        .expect("write docker config");
        std::env::set_var("REGISTRY_AUTH_DOCKERCONFIG", config.path());

        let manifest_path = "/v2/demo/example/manifests/latest";
        let mock = server
            .mock_async(|when, then| {
                when.method("PUT")
                    .path(manifest_path)
                    .header("authorization", format!("Basic {}", auth_value))
                    .header(
                        "content-type",
                        "application/vnd.docker.distribution.manifest.list.v2+json",
                    );
                then.status(201)
                    .header("Docker-Content-Digest", "sha256:manifest123");
            })
            .await;

        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let target = PlatformTarget::parse("linux/amd64").expect("valid platform");

        let digest = publish_manifest_list(
            &logger,
            &metrics,
            5,
            &registry,
            "example",
            "latest",
            &[(target.clone(), "sha256:deadbeef".to_string())],
        )
        .await
        .expect("manifest publish succeeds");

        assert_eq!(digest, "sha256:manifest123");
        mock.assert_async().await;

        let events = metrics.events().await;
        let manifest_event = events
            .iter()
            .find(|(event, _)| event == "manifest_published")
            .expect("manifest event emitted");
        let details = manifest_event.1.as_ref().expect("manifest details");
        assert_eq!(
            details
                .get("architectures")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(Value::as_str),
            Some("linux/amd64")
        );
    }
    #[tokio::test]
    async fn record_push_failure_respects_auth_flag() {
        let cases = vec![(true, "auth_expired"), (false, "remote")];

        for (auth_expired, error_kind) in cases {
            let metrics = RecordingMetrics::default();

            record_push_failure(
                &metrics,
                "registry.test/example",
                2,
                4,
                error_kind,
                "failure",
                auth_expired,
                "linux/amd64",
            )
            .await;

            let events = metrics.events().await;
            let failure_entry = events
                .iter()
                .find(|(event, _)| event == "push_failed")
                .and_then(|(_, details)| details.as_ref())
                .expect("push_failed metric recorded");

            assert_eq!(
                failure_entry.get("auth_expired").and_then(Value::as_bool),
                Some(auth_expired)
            );
            assert_eq!(
                failure_entry.get("retry_limit").and_then(Value::as_u64),
                Some(4)
            );
            assert_eq!(
                failure_entry.get("error_kind").and_then(Value::as_str),
                Some(error_kind)
            );
            assert_eq!(
                failure_entry.get("platform").and_then(Value::as_str),
                Some("linux/amd64")
            );
        }
    }

    #[tokio::test]
    async fn record_push_failure_captures_all_error_variants() {
        use bollard::errors::Error as BollardError;

        let cases: Vec<(RegistryPushError, usize, &'static str, bool)> = vec![
            (
                RegistryPushError::Remote("denied".to_string()),
                1,
                "remote",
                false,
            ),
            (
                RegistryPushError::AuthExpired("expired".to_string()),
                2,
                "auth_expired",
                true,
            ),
            (
                RegistryPushError::Tag(BollardError::DockerResponseServerError {
                    status_code: 500,
                    message: "tag failure".to_string(),
                }),
                0,
                "tag",
                false,
            ),
            (
                RegistryPushError::Push(BollardError::DockerResponseServerError {
                    status_code: 502,
                    message: "push failure".to_string(),
                }),
                3,
                "push",
                false,
            ),
        ];

        for (error, attempt, expected_kind, expected_auth) in cases {
            let metrics = RecordingMetrics::default();
            let (error_kind, auth_flag) = classify_registry_push_error(&error);
            let error_message = error.to_string();

            record_push_failure(
                &metrics,
                "registry.test/example",
                attempt,
                5,
                error_kind,
                &error_message,
                auth_flag,
                "linux/amd64",
            )
            .await;

            let events = metrics.events().await;
            let failure_entry = events
                .iter()
                .find(|(event, _)| event == "push_failed")
                .and_then(|(_, details)| details.as_ref())
                .expect("push_failed metric recorded");

            assert_eq!(
                failure_entry
                    .get("attempt")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
                Some(attempt),
            );
            assert_eq!(
                failure_entry
                    .get("retry_limit")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
                Some(5),
            );
            assert_eq!(
                failure_entry.get("error_kind").and_then(Value::as_str),
                Some(expected_kind),
            );
            assert_eq!(
                failure_entry.get("auth_expired").and_then(Value::as_bool),
                Some(expected_auth),
            );
            assert_eq!(
                failure_entry
                    .get("error")
                    .and_then(Value::as_str)
                    .map(|value| value.to_owned()),
                Some(error_message.clone()),
            );
            assert_eq!(
                failure_entry.get("platform").and_then(Value::as_str),
                Some("linux/amd64")
            );
        }
    }

    #[tokio::test]
    async fn record_push_failure_handles_zero_retry_limit() {
        let metrics = RecordingMetrics::default();

        record_push_failure(
            &metrics,
            "registry.test/example",
            1,
            0,
            "remote",
            "simulated error",
            false,
            "linux/amd64",
        )
        .await;

        let events = metrics.events().await;
        let failure_entry = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed metric recorded");

        assert_eq!(
            failure_entry.get("retry_limit").and_then(Value::as_u64),
            Some(0),
        );
        assert_eq!(
            failure_entry.get("attempt").and_then(Value::as_u64),
            Some(1),
        );
    }

    #[tokio::test]
    async fn proactive_rotation_skipped_without_refresher() {
        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let config = NamedTempFile::new().expect("temp docker config");
        std::fs::write(
            config.path(),
            r#"{"auths": {"example.com": {"auth": "dXNlcjpwYXNz"}}}"#,
        )
        .expect("write docker config");

        std::env::set_var("REGISTRY_AUTH_DOCKERCONFIG", config.path());
        std::env::set_var("REGISTRY_AUTH_MAX_AGE_SECONDS", "0");
        std::env::set_var("REGISTRY_AUTH_ROTATE_LEAD_SECONDS", "0");

        let (snapshot, attempted, succeeded) = record_credential_health_and_rotate(
            &logger,
            &metrics,
            404,
            "https://example.com",
            "example/repo",
            "linux/amd64",
            None,
        )
        .await;

        assert!(snapshot.status.requires_rotation());
        assert!(!attempted);
        assert!(!succeeded);

        let events = metrics.events().await;
        assert!(events
            .iter()
            .any(|(event, _)| event == "auth_health_reported"));
        let skipped_details = events
            .iter()
            .find(|(event, _)| event == "auth_rotation_skipped")
            .and_then(|(_, details)| details.as_ref())
            .expect("auth_rotation_skipped recorded");
        assert_eq!(
            skipped_details.get("reason").and_then(Value::as_str),
            Some("refresher_unavailable")
        );

        std::env::remove_var("REGISTRY_AUTH_MAX_AGE_SECONDS");
        std::env::remove_var("REGISTRY_AUTH_ROTATE_LEAD_SECONDS");
        std::env::remove_var("REGISTRY_AUTH_DOCKERCONFIG");
    }

    #[tokio::test]
    async fn proactive_rotation_succeeds_with_refresher() {
        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let config = NamedTempFile::new().expect("temp docker config");
        std::fs::write(
            config.path(),
            r#"{"auths": {"example.com": {"auth": "dXNlcjpwYXNz"}}}"#,
        )
        .expect("write docker config");

        std::env::set_var("REGISTRY_AUTH_DOCKERCONFIG", config.path());
        std::env::set_var("REGISTRY_AUTH_MAX_AGE_SECONDS", "0");
        std::env::set_var("REGISTRY_AUTH_ROTATE_LEAD_SECONDS", "0");

        let mut refresher = TestRefresher::succeed();

        let (_, attempted, succeeded) = record_credential_health_and_rotate(
            &logger,
            &metrics,
            405,
            "https://example.com",
            "example/repo",
            "linux/amd64",
            Some(&mut refresher as &mut dyn RegistryAuthRefresher),
        )
        .await;

        assert!(attempted);
        assert!(succeeded);
        assert_eq!(refresher.attempts(), 1);

        let events = metrics.events().await;
        assert!(events
            .iter()
            .any(|(event, _)| event == "auth_rotation_started"));
        assert!(events
            .iter()
            .any(|(event, _)| event == "auth_rotation_succeeded"));

        std::env::remove_var("REGISTRY_AUTH_MAX_AGE_SECONDS");
        std::env::remove_var("REGISTRY_AUTH_ROTATE_LEAD_SECONDS");
        std::env::remove_var("REGISTRY_AUTH_DOCKERCONFIG");
    }

    #[tokio::test]
    async fn remote_error_without_detail_message_records_failure() {
        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let scopes = vec!["scope".to_string()];

        let err = push_stream_with_retry(
            &logger,
            &metrics,
            11,
            "registry.test/example",
            &scopes,
            || {
                let mut info = PushImageInfo::default();
                info.error_detail = Some(bollard::models::ErrorDetail {
                    code: Some(418),
                    message: None,
                });
                stream::iter(vec![Ok(info)])
            },
            2,
            "linux/amd64",
            None,
        )
        .await
        .expect_err("error details without message should bubble");

        match err {
            RegistryPushError::Remote(msg) => assert!(msg.contains("Unknown registry error")),
            other => panic!("expected remote error detail, got {:?}", other),
        }

        let events = metrics.events().await;
        let failure_entry = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed metric recorded");

        assert_eq!(
            failure_entry.get("error_kind").and_then(Value::as_str),
            Some("remote"),
        );
        assert_eq!(
            failure_entry.get("auth_expired").and_then(Value::as_bool),
            Some(false),
        );
        assert_eq!(
            failure_entry.get("attempt").and_then(Value::as_u64),
            Some(1),
        );
        assert_eq!(
            failure_entry.get("retry_limit").and_then(Value::as_u64),
            Some(2),
        );
    }

    #[test]
    fn classify_registry_push_error_covers_variants() {
        use bollard::errors::Error as BollardError;

        let cases = vec![
            (
                RegistryPushError::AuthExpired("expired".to_string()),
                ("auth_expired", true),
            ),
            (
                RegistryPushError::Remote("denied".to_string()),
                ("remote", false),
            ),
            (
                RegistryPushError::Tag(BollardError::DockerResponseServerError {
                    status_code: 500,
                    message: "tag failed".to_string(),
                }),
                ("tag", false),
            ),
            (
                RegistryPushError::Push(BollardError::DockerResponseServerError {
                    status_code: 502,
                    message: "push failed".to_string(),
                }),
                ("push", false),
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(classify_registry_push_error(&error), expected);
        }
    }

    #[tokio::test]
    async fn push_errors_record_retry_metadata() {
        use bollard::errors::Error as BollardError;

        let logger = RecordingLog::default();
        let metrics = RecordingMetrics::default();
        let scopes = vec!["scope".to_string()];

        let err = push_stream_with_retry(
            &logger,
            &metrics,
            8,
            "registry.test/example",
            &scopes,
            || {
                stream::iter(vec![Err(BollardError::DockerResponseServerError {
                    status_code: 500,
                    message: "boom".to_string(),
                })])
            },
            1,
            "linux/amd64",
            None,
        )
        .await
        .expect_err("push error should bubble");

        assert!(matches!(err, RegistryPushError::Push(_)));
        let events = metrics.events().await;
        let failure_details = events
            .iter()
            .find(|(event, _)| event == "push_failed")
            .and_then(|(_, details)| details.as_ref())
            .expect("push_failed details present");

        assert_eq!(
            failure_details.get("attempt").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            failure_details.get("retry_limit").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            failure_details.get("error_kind").and_then(Value::as_str),
            Some("push")
        );
        assert_eq!(
            failure_details.get("auth_expired").and_then(Value::as_bool),
            Some(false)
        );
    }
}
