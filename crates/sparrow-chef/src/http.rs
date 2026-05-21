use eyre::{Result, eyre};
use reqwest::Client;

pub struct SparrowClient {
    base_url: String,
    client: Client,
}

impl SparrowClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// POST to /diagnostics to check if the database is reachable.
    pub async fn check_health(&self) -> Result<()> {
        let url = format!("{}/diagnostics", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .map_err(|e| eyre!("health check failed: {e}"))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(eyre!("database returned status {}", resp.status()))
        }
    }

    /// POST a v1/query-compatible JSON body to /v1/query.
    pub async fn post_v1_query(&self, body: &str) -> Result<serde_json::Value> {
        let url = format!("{}/v1/query", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| eyre!("POST /v1/query failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(eyre!("seed request failed with {status}: {text}"));
        }

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| eyre!("failed to parse seed response: {e}"))
    }
}
