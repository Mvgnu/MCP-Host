use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

pub struct VaultClient {
    base: String,
    token: String,
    client: Client,
}

impl VaultClient {
    pub fn from_env() -> Option<Self> {
        let base = std::env::var("VAULT_ADDR").ok()?;
        let token = std::env::var("VAULT_TOKEN").ok()?;
        Some(Self::new(base, token))
    }

    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            token: token.into(),
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("client build"),
        }
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, reqwest::Error> {
        let url = format!("{}/v1/{}", self.base, path);
        let mut req = self
            .client
            .request(method, &url)
            .header("X-Vault-Token", &self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?.error_for_status()?;
        if resp.status().is_success() && resp.content_length().unwrap_or(0) == 0 {
            return Ok(Value::Null);
        }
        Ok(resp.json().await?)
    }

    pub async fn store_secret(&self, path: &str, value: &str) -> Result<(), reqwest::Error> {
        self.request(
            reqwest::Method::POST,
            path,
            Some(serde_json::json!({"data": {"value": value}})),
        )
        .await?
        .clear();
        Ok(())
    }

    pub async fn read_secret(&self, path: &str) -> Result<String, reqwest::Error> {
        let val = self.request(reqwest::Method::GET, path, None).await?;
        Ok(val["data"]["data"]["value"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    pub async fn delete_secret(&self, path: &str) -> Result<(), reqwest::Error> {
        self.request(reqwest::Method::DELETE, path, None).await?;
        Ok(())
    }
}

trait Clear {
    fn clear(self);
}

impl Clear for Value {
    fn clear(self) {}
}
