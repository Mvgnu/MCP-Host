use crate::servers::set_status;
use async_trait::async_trait;
use bollard::body_full;
use bollard::image::BuildImageOptions;
use bollard::models::PushImageInfo;
use bollard::query_parameters::{PushImageOptionsBuilder, TagImageOptionsBuilder};
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use regex::Regex;
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

async fn push_stream_with_retry<L, F, S>(
    logger: &L,
    server_id: i32,
    registry_endpoint: &str,
    scopes: &[String],
    mut make_stream: F,
    retry_limit: usize,
) -> Result<(), RegistryPushError>
where
    L: BuildLogSink + ?Sized,
    F: FnMut() -> S,
    S: futures_util::Stream<Item = Result<PushImageInfo, bollard::errors::Error>> + Unpin,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
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
                let backoff = Duration::from_millis(100 * attempt as u64);
                sleep(backoff).await;
                continue;
            }
            Err(err) => {
                tracing::error!(
                    target: "registry.push",
                    %registry_endpoint,
                    %server_id,
                    scopes = ?scopes,
                    attempt,
                    retry_limit,
                    error = %err,
                    "registry push failed"
                );
                return Err(err);
            }
        }
    }
}

async fn push_image_to_registry<L: BuildLogSink + ?Sized>(
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

    let tag_opts = TagImageOptionsBuilder::new()
        .repo(&reference.repository)
        .tag(&reference.tag)
        .build();
    docker
        .tag_image(image_tag, Some(tag_opts))
        .await
        .map_err(RegistryPushError::Tag)?;

    insert_log(logger, server_id, "Pushing image to registry").await;
    tracing::info!(
        target: "registry.push",
        registry_endpoint = %reference.repository,
        %server_id,
        scopes = ?scopes,
        tag = %reference.tag,
        "starting registry push"
    );

    let retry_limit = registry_push_retry_limit();
    push_stream_with_retry(
        logger,
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

async fn set_status_or_log(pool: &PgPool, server_id: i32, status: &str) {
    if let Err(err) = set_status(pool, server_id, status).await {
        tracing::error!(
            ?err,
            %server_id,
            status = %status,
            "failed to update server status after build operation"
        );
    }
}

/// Clone a git repository and build a Docker image.
/// Returns the built image tag on success.
pub async fn build_from_git(
    pool: &PgPool,
    server_id: i32,
    repo_url: &str,
    branch: Option<&str>,
) -> Option<String> {
    insert_log(pool, server_id, "Cloning repository").await;
    let tmp = match tempdir() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(?e, "tempdir failed");
            set_status_or_log(pool, server_id, "error").await;
            insert_log(pool, server_id, "Failed to create build dir").await;
            return None;
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
        set_status_or_log(pool, server_id, "error").await;
        return None;
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
            set_status_or_log(pool, server_id, "error").await;
            return None;
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
            set_status_or_log(pool, server_id, "error").await;
            return None;
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
                set_status_or_log(pool, server_id, "error").await;
                return None;
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
            if let Err(err) = push_image_to_registry(pool, &docker, server_id, &tag, registry).await
            {
                tracing::error!(?err, %registry, %server_id, "registry push failed");
                insert_log(pool, server_id, &format!("Registry push failed: {err}")).await;
                set_status_or_log(pool, server_id, "error").await;
                return None;
            }
        }
    }
    insert_log(pool, server_id, "Cleaning up").await;
    Some(tag)
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

    #[async_trait]
    impl BuildLogSink for RecordingLog {
        async fn log(&self, _server_id: i32, text: &str) {
            self.entries.lock().await.push(text.to_string());
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
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let scopes = vec!["scope".to_string()];

        push_stream_with_retry(
            &logger,
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
    }

    #[tokio::test]
    async fn non_retryable_push_errors_bubble() {
        let logger = RecordingLog::default();
        let scopes = vec!["scope".to_string()];
        let err = push_stream_with_retry(
            &logger,
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
    }
}
