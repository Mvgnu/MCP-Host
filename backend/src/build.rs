use crate::servers::{add_metric, set_status, SetStatusError};
use async_trait::async_trait;
use bollard::body_full;
use bollard::image::BuildImageOptions;
use bollard::models::PushImageInfo;
use bollard::query_parameters::{PushImageOptionsBuilder, TagImageOptionsBuilder};
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use regex::Regex;
use serde_json::json;
use serde_json::Value;
use sqlx::PgPool;
use std::fmt;
use std::path::Path;
use tar::Builder as TarBuilder;
use tempfile::tempdir;
use tokio::fs;
use tokio::time::{sleep, Duration};

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

fn build_registry_reference(registry: &str, image_tag: &str) -> RegistryReference {
    let registry = registry.trim_end_matches('/');
    RegistryReference {
        repository: format!("{registry}/{image_tag}"),
        tag: "latest".to_string(),
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

const DEFAULT_REGISTRY_PUSH_RETRIES: usize = 3;

fn registry_push_retry_limit() -> usize {
    std::env::var("REGISTRY_PUSH_RETRIES")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|&value| value > 0)
        .unwrap_or(DEFAULT_REGISTRY_PUSH_RETRIES)
}

fn registry_scopes(repository: &str) -> Vec<String> {
    vec![
        format!("repository:{repository}:push"),
        format!("repository:{repository}:pull"),
    ]
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
            tracing::warn!(
                target: "registry.push.metrics",
                event_type = %event_type,
                server_id = self.server_id,
                ?err,
                "failed to persist registry metric"
            );
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
) -> Result<(), RegistryPushError>
where
    L: BuildLogSink + ?Sized,
    S: futures_util::Stream<Item = Result<PushImageInfo, bollard::errors::Error>> + Unpin,
{
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
                        "registry error detail"
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
                        "registry returned error"
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
                                "registry reported digest"
                            );
                            insert_log(logger, server_id, &digest_message).await;
                        }
                        tracing::info!(
                            target: "registry.push",
                            %registry_endpoint,
                            %server_id,
                            scopes = ?scopes,
                            status = %line,
                            "registry push status"
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
                    "registry push stream error"
                );
                return Err(RegistryPushError::Push(err));
            }
        }
    }

    Ok(())
}

async fn push_stream_with_retry<L, M, F, S>(
    logger: &L,
    metrics: &M,
    server_id: i32,
    registry_endpoint: &str,
    scopes: &[String],
    mut make_stream: F,
    retry_limit: usize,
) -> Result<(), RegistryPushError>
where
    L: BuildLogSink + ?Sized,
    M: MetricRecorder + ?Sized,
    F: FnMut() -> S,
    S: futures_util::Stream<Item = Result<PushImageInfo, bollard::errors::Error>> + Unpin,
{
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
            "starting registry push attempt"
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
            Ok(()) => {
                if attempt > 1 {
                    tracing::info!(
                        target: "registry.push",
                        %registry_endpoint,
                        %server_id,
                        scopes = ?scopes,
                        attempt,
                        "registry push succeeded after retry"
                    );
                }
                metrics
                    .record(
                        "push_succeeded",
                        Some(json!({
                            "attempt": attempt,
                            "retry_limit": retry_limit,
                            "registry_endpoint": registry_endpoint,
                        })),
                    )
                    .await;
                return Ok(());
            }
            Err(RegistryPushError::Push(err))
                if attempt < retry_limit && is_retryable_push_error(&err) =>
            {
                tracing::warn!(
                    target: "registry.push",
                    %registry_endpoint,
                    %server_id,
                    scopes = ?scopes,
                    attempt,
                    retry_limit,
                    error = %err,
                    "retryable registry push error"
                );
                metrics
                    .record(
                        "push_retry",
                        Some(json!({
                            "attempt": attempt,
                            "retry_limit": retry_limit,
                            "registry_endpoint": registry_endpoint,
                            "error": err.to_string(),
                        })),
                    )
                    .await;
                let backoff = Duration::from_millis(100 * attempt as u64);
                sleep(backoff).await;
                continue;
            }
            Err(err) => {
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
                )
                .await;
                tracing::error!(
                    target: "registry.push",
                    %registry_endpoint,
                    %server_id,
                    scopes = ?scopes,
                    attempt,
                    retry_limit,
                    error = %error_message,
                    "registry push failed",
                );
                return Err(err);
            }
        }
    }
}

async fn push_image_to_registry<L: BuildLogSink + ?Sized>(
    pool: &PgPool,
    logger: &L,
    docker: &Docker,
    server_id: i32,
    image_tag: &str,
    registry: &str,
) -> Result<(), RegistryPushError> {
    let reference = build_registry_reference(registry, image_tag);
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
            })),
        )
        .await;

    let tag_opts = TagImageOptionsBuilder::new()
        .repo(&reference.repository)
        .tag(&reference.tag)
        .build();
    if let Err(err) = docker.tag_image(image_tag, Some(tag_opts)).await {
        let error_message = err.to_string();
        record_push_failure(
            &usage_metrics,
            &reference.repository,
            0,
            retry_limit,
            "tag",
            &error_message,
            false,
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

    push_stream_with_retry(
        logger,
        &usage_metrics,
        server_id,
        &reference.repository,
        &scopes,
        || {
            let push_opts = PushImageOptionsBuilder::new().tag(&reference.tag).build();
            docker.push_image(&reference.repository, Some(push_opts), None)
        },
        retry_limit,
    )
    .await?;

    insert_log(logger, server_id, "Image pushed to registry").await;
    tracing::info!(
        target: "registry.push",
        registry_endpoint = %reference.repository,
        %server_id,
        scopes = ?scopes,
        "registry push completed"
    );

    Ok(())
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

/// Clone a git repository and build a Docker image.
/// Returns the built image tag on success.
pub async fn build_from_git(
    pool: &PgPool,
    server_id: i32,
    repo_url: &str,
    branch: Option<&str>,
) -> Result<Option<String>, SetStatusError> {
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
    let tag = format!("mcp-custom-{server_id}");
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

    let options = BuildImageOptions::<String> {
        dockerfile: "Dockerfile".into(),
        t: tag.clone(),
        pull: true,
        nocache: true,
        rm: true,
        forcerm: true,
        ..Default::default()
    };

    let mut build_stream = docker.build_image(options, None, Some(body_full(tar_data)));
    while let Some(item) = build_stream.next().await {
        match item {
            Ok(output) => {
                if let Some(msg) = output.stream {
                    insert_log(pool, server_id, msg.trim()).await;
                }
            }
            Err(e) => {
                tracing::error!(?e, "docker build error");
                insert_log(pool, server_id, "Image build failed").await;
                set_status_or_log(pool, server_id, "error").await?;
                return Ok(None);
            }
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

    if let Ok(registry) = std::env::var("REGISTRY") {
        let registry = registry.trim();
        if !registry.is_empty() {
            if let Err(err) =
                push_image_to_registry(pool, pool, &docker, server_id, &tag, registry).await
            {
                tracing::error!(?err, %registry, %server_id, "registry push failed");
                insert_log(pool, server_id, &format!("Registry push failed: {err}")).await;
                set_status_or_log(pool, server_id, "error").await?;
                return Ok(None);
            }
        }
    }
    insert_log(pool, server_id, "Cleaning up").await;
    Ok(Some(tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::PushImageInfo;
    use futures_util::stream;
    use tempfile::tempdir;
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
        let reference = build_registry_reference("example.com/org/", "app");
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

        push_stream_with_retry(
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
        )
        .await
        .expect("retry should eventually succeed");

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
            failure_entry
                .get("error_kind")
                .and_then(Value::as_str),
            Some("tag")
        );
        assert_eq!(
            failure_entry
                .get("retry_limit")
                .and_then(Value::as_u64),
            Some(5)
        );
        assert_eq!(
            failure_entry
                .get("auth_expired")
                .and_then(Value::as_bool),
            Some(false)
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
                    .get("auth_expired")
                    .and_then(Value::as_bool),
                Some(auth_expired)
            );
            assert_eq!(
                failure_entry
                    .get("retry_limit")
                    .and_then(Value::as_u64),
                Some(4)
            );
            assert_eq!(
                failure_entry
                    .get("error_kind")
                    .and_then(Value::as_str),
                Some(error_kind)
            );
        }
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
            failure_details
                .get("retry_limit")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            failure_details
                .get("error_kind")
                .and_then(Value::as_str),
            Some("push")
        );
        assert_eq!(
            failure_details
                .get("auth_expired")
                .and_then(Value::as_bool),
            Some(false)
        );
    }
}
