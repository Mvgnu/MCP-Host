use sqlx::{PgPool, Row};
use std::path::PathBuf;
use std::time::Duration;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use acme2::{AccountBuilder, DirectoryBuilder, OrderBuilder, Csr, gen_rsa_private_key};

pub fn conf_dir() -> PathBuf {
    std::env::var("PROXY_CONF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./proxy_conf"))
}

pub async fn ensure_tls(domain: &str) {
    let cert_path = format!("/etc/letsencrypt/live/{domain}/fullchain.pem");
    if tokio::fs::metadata(&cert_path).await.is_ok() {
        return;
    }
    let email = match std::env::var("CERTBOT_EMAIL") {
        Ok(e) => e,
        Err(_) => return,
    };
    if let Err(e) = obtain_cert(domain.to_string(), email).await {
        tracing::error!(?e, "certificate request failed");
    }
}

async fn obtain_cert(domain: String, email: String) -> Result<(), Box<dyn std::error::Error>> {
    const LETS_ENCRYPT_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";
    let dir = DirectoryBuilder::new(LETS_ENCRYPT_URL.to_string()).build().await?;
    let account = AccountBuilder::new(dir.clone())
        .contact(vec![format!("mailto:{email}")])
        .terms_of_service_agreed(true)
        .build()
        .await?;

    let mut order = OrderBuilder::new(account)
        .add_dns_identifier(domain.clone())
        .build()
        .await?;

    for auth in order.authorizations().await? {
        if let Some(mut challenge) = auth.get_challenge("http-01") {
            let token = match &challenge.token {
                Some(t) => t.clone(),
                None => continue,
            };
            let key = match challenge.key_authorization()? {
                Some(k) => k,
                None => continue,
            };
            let path = conf_dir().join("acme").join(&token);
            if let Some(p) = path.parent() {
                tokio::fs::create_dir_all(p).await?;
            }
            tokio::fs::write(&path, key).await?;
            challenge = challenge.validate().await?;
            challenge.wait_done(Duration::from_secs(5), 15).await?;
        }
    }

    order = order.wait_ready(Duration::from_secs(5), 15).await?;
    let pkey = gen_rsa_private_key(2048)?;
    order = order.finalize(Csr::Automatic(pkey.clone())).await?;
    order = order.wait_done(Duration::from_secs(5), 15).await?;
    let certs = order
        .certificate()
        .await?
        .ok_or("certificate missing")?;
    let mut pem_chain = Vec::new();
    for c in &certs {
        pem_chain.extend(c.to_pem()?);
    }
    let cert_dir = format!("/etc/letsencrypt/live/{domain}");
    tokio::fs::create_dir_all(&cert_dir).await?;
    tokio::fs::write(format!("{cert_dir}/fullchain.pem"), pem_chain).await?;
    tokio::fs::write(
        format!("{cert_dir}/privkey.pem"),
        pkey.private_key_to_pem_pkcs8()?,
    )
    .await?;
    Ok(())
}

async fn write_config(server_id: i32, domains: &[String]) -> std::io::Result<()> {
    let dir = conf_dir();
    tokio::fs::create_dir_all(&dir).await.ok();
    let path = dir.join(format!("server_{}.conf", server_id));
    if domains.is_empty() {
        let _ = tokio::fs::remove_file(&path).await;
        return Ok(());
    }
    let mut content = String::new();
    for d in domains {
        content.push_str(&format!(
"server {{
    listen 80;
    server_name {};
    location / {{
        proxy_pass http://mcp-server-{}:8080;
    }}
}}
",
            d, server_id
        ));
    }
    tokio::fs::write(&path, content).await?;
    Ok(())
}

pub async fn reload() -> std::io::Result<()> {
    let pid_str = tokio::fs::read_to_string("/run/nginx.pid").await?;
    if let Ok(pid) = pid_str.trim().parse::<i32>() {
        if let Err(e) = kill(Pid::from_raw(pid), Signal::SIGHUP) {
            tracing::error!(?e, "failed to reload nginx");
        }
    }
    Ok(())
}

pub async fn rebuild_for_server(pool: &PgPool, server_id: i32) {
    match sqlx::query("SELECT domain FROM custom_domains WHERE server_id = $1")
        .bind(server_id)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => {
            let domains: Vec<String> = rows.into_iter().map(|r| r.get("domain")).collect();
            if let Err(e) = write_config(server_id, &domains).await {
                tracing::error!(?e, "failed to write proxy config");
            }
        }
        Err(e) => tracing::error!(?e, "proxy DB error"),
    }
}
