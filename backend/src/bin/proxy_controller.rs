use backend::proxy::{conf_dir, ensure_tls, reload};
use dotenvy::dotenv;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use tokio::fs;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let dir = conf_dir();
    fs::create_dir_all(&dir).await?;
    let mut mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
    let domain_re = Regex::new(r"server_name\s+([^;]+);")?;
    loop {
        let mut changed = false;
        let mut domains = Vec::new();
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => {
                sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("conf") {
                continue;
            }
            let meta = entry.metadata().await?;
            let mtime = meta.modified()?;
            if mtimes.get(&path) != Some(&mtime) {
                mtimes.insert(path.clone(), mtime);
                changed = true;
            }
            let content = fs::read_to_string(&path).await?;
            if let Some(caps) = domain_re.captures(&content) {
                for d in caps[1].split_whitespace() {
                    domains.push(d.to_string());
                }
            }
        }
        if changed {
            for d in &domains {
                ensure_tls(d).await;
            }
            if let Err(e) = reload().await {
                eprintln!("failed to reload nginx: {e:?}");
            }
        }
        sleep(Duration::from_secs(10)).await;
    }
}
