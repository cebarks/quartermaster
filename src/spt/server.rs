use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};

pub struct PingResult {
    pub ok: bool,
    pub latency_ms: u64,
}

pub struct SptClient {
    client: reqwest::Client,
    base_url: String,
}

impl SptClient {
    pub fn new(host: &str, port: u16) -> Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .context("failed to build SPT HTTP client")?;

        Ok(Self {
            client,
            base_url: format!("https://{}:{}", host, port),
        })
    }

    pub async fn ping(&self) -> Result<PingResult> {
        let start = Instant::now();
        let resp = self
            .client
            .get(format!("{}/launcher/ping", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => Ok(PingResult {
                ok: true,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
            Ok(_) => Ok(PingResult {
                ok: false,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
            Err(_) => Ok(PingResult {
                ok: false,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }

    pub async fn server_version(&self) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/launcher/server/version", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach SPT server for version")?
            .error_for_status()
            .context("SPT server version endpoint returned error")?;

        let body = resp
            .text()
            .await
            .context("failed to read version response")?;
        // Response is a JSON string like "\"4.0.13\"" — strip outer quotes
        let version = body.trim().trim_matches('"').to_string();
        Ok(version)
    }

    pub async fn loaded_server_mods(&self) -> Result<HashMap<String, serde_json::Value>> {
        let resp = self
            .client
            .get(format!(
                "{}/launcher/server/loadedServerMods",
                self.base_url
            ))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach SPT server for loaded mods")?
            .error_for_status()
            .context("SPT loaded mods endpoint returned error")?;

        let mods: HashMap<String, serde_json::Value> = resp
            .json()
            .await
            .context("failed to parse loaded mods response")?;
        Ok(mods)
    }

    pub async fn headless_clients(&self) -> Result<crate::spt::headless::GetHeadlessesResponse> {
        let resp = self
            .client
            .get(format!("{}/fika/headless/get", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach Fika headless endpoint")?
            .error_for_status()
            .context("Fika headless endpoint returned error")?;
        resp.json()
            .await
            .context("failed to parse headless clients response")
    }

    pub async fn available_headless_clients(
        &self,
    ) -> Result<Vec<crate::spt::headless::HeadlessAvailableClient>> {
        let resp = self
            .client
            .get(format!("{}/fika/headless/available", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach Fika headless available endpoint")?
            .error_for_status()
            .context("Fika headless available endpoint returned error")?;
        resp.json()
            .await
            .context("failed to parse available headless clients response")
    }

    pub async fn headless_restart_config(
        &self,
    ) -> Result<crate::spt::headless::HeadlessRestartConfig> {
        let resp = self
            .client
            .get(format!(
                "{}/fika/headless/restartafterraidamount",
                self.base_url
            ))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach Fika headless restart config endpoint")?
            .error_for_status()
            .context("Fika headless restart config endpoint returned error")?;
        resp.json()
            .await
            .context("failed to parse headless restart config response")
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spt_client_constructs_base_url() {
        let client = SptClient::new("192.168.1.10", 6969).unwrap();
        assert_eq!(client.base_url(), "https://192.168.1.10:6969");
    }

    #[test]
    fn spt_client_localhost() {
        let client = SptClient::new("127.0.0.1", 6969).unwrap();
        assert_eq!(client.base_url(), "https://127.0.0.1:6969");
    }

    #[test]
    fn spt_client_custom_port() {
        let client = SptClient::new("10.0.0.1", 7070).unwrap();
        assert_eq!(client.base_url(), "https://10.0.0.1:7070");
    }
}
