use regex::Regex;
use sqlx::PgPool;
use crate::servers::set_status;
use std::path::Path;
use tempfile::tempdir;
use tokio::fs;
use bollard::body_full;
use bollard::image::BuildImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use bytes::Bytes;
use tar::Builder as TarBuilder;

#[derive(Clone, Copy)]
enum LangBuilder {
    Node,
    Python,
    Rust,
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

async fn insert_log(pool: &PgPool, server_id: i32, text: &str) {
    let _ = sqlx::query("INSERT INTO server_logs (server_id, log_text) VALUES ($1, $2)")
        .bind(server_id)
        .bind(text)
        .execute(pool)
        .await;
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
            set_status(pool, server_id, "error").await;
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
        set_status(pool, server_id, "error").await;
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
            set_status(pool, server_id, "error").await;
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
    .unwrap_or_else(|e| Err(std::io::Error::new(std::io::ErrorKind::Other, e))) ;

    let tar_data = match tar_res {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(?e, "Failed to create tar");
            insert_log(pool, server_id, "Failed to create build context").await;
            set_status(pool, server_id, "error").await;
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
                set_status(pool, server_id, "error").await;
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
        let full_tag = format!("{}/{}", registry, tag);
        insert_log(pool, server_id, "Pushing image to registry").await;
        let _ = tokio::process::Command::new("docker")
            .arg("tag")
            .arg(&tag)
            .arg(&full_tag)
            .status()
            .await;
        let _ = tokio::process::Command::new("docker")
            .arg("push")
            .arg(&full_tag)
            .status()
            .await;
    }
    insert_log(pool, server_id, "Cleaning up").await;
    Some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn detect_builder_works() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").await.unwrap();
        assert!(matches!(detect_builder(dir.path()).await, Some(LangBuilder::Node)));
    }

    #[tokio::test]
    async fn generates_dockerfile() {
        let dir = tempdir().unwrap();
        generate_dockerfile(dir.path(), LangBuilder::Python).await.unwrap();
        let contents = fs::read_to_string(dir.path().join("Dockerfile")).await.unwrap();
        assert!(contents.contains("python"));
    }

    #[test]
    fn exposes_check() {
        let dockerfile = "FROM scratch\nEXPOSE 8080";
        assert!(dockerfile_exposes_8080(dockerfile));
        let other = "FROM scratch\nEXPOSE 5000";
        assert!(!dockerfile_exposes_8080(other));
    }
}
