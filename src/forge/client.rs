use std::path::Path;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde_json::json;
use tokio::io::AsyncWriteExt;

use super::models::*;

const DEFAULT_BASE_URL: &str = "https://forge.sp-tarkov.com/api/v0";

pub struct ForgeClient {
    client: reqwest::Client,
    base_url: String,
    #[allow(dead_code)]
    token: Option<String>,
}

impl ForgeClient {
    /// Create a new client pointing at the production Forge API.
    /// If `token` is provided it is sent as a Bearer token on every request.
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::build(DEFAULT_BASE_URL.to_string(), token)
    }

    /// Create a client with a custom base URL (for tests against a mock server).
    #[cfg(test)]
    pub fn with_base_url(base_url: String, token: Option<String>) -> Result<Self> {
        Self::build(base_url, token)
    }

    fn build(base_url: String, token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();

        let ua = format!("quartermaster/{}", env!("CARGO_PKG_VERSION"));
        headers.insert(USER_AGENT, HeaderValue::from_str(&ua)?);

        if let Some(ref t) = token {
            let val =
                HeaderValue::from_str(&format!("Bearer {t}")).context("invalid auth token")?;
            headers.insert(AUTHORIZATION, val);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    /// Search mods by query string.
    pub async fn search_mods(&self, query: &str) -> Result<Vec<ForgeMod>> {
        let url = format!("{}/mods", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("query", query)])
            .send()
            .await
            .context("search_mods request failed")?
            .error_for_status()
            .context("search_mods returned error status")?;

        let body: ForgeSearchResponse = resp
            .json()
            .await
            .context("search_mods: failed to parse response")?;
        Ok(body.data)
    }

    /// Fetch a single mod by ID, optionally including its versions.
    pub async fn get_mod(&self, id: i64, include_versions: bool) -> Result<ForgeMod> {
        let url = format!("{}/mod/{}", self.base_url, id);
        let mut req = self.client.get(&url);
        if include_versions {
            req = req.query(&[("include", "versions")]);
        }

        let resp = req
            .send()
            .await
            .context("get_mod request failed")?
            .error_for_status()
            .context("get_mod returned error status")?;

        let body: ForgeModResponse = resp
            .json()
            .await
            .context("get_mod: failed to parse response")?;
        Ok(body.data)
    }

    /// List versions for a mod, optionally filtered to a specific SPT version.
    pub async fn get_versions(
        &self,
        mod_id: i64,
        spt_version: Option<&str>,
    ) -> Result<Vec<ForgeVersion>> {
        let url = format!("{}/mod/{}/versions", self.base_url, mod_id);
        let mut req = self.client.get(&url);
        if let Some(v) = spt_version {
            req = req.query(&[("filter[spt_version]", v)]);
        }

        let resp = req
            .send()
            .await
            .context("get_versions request failed")?
            .error_for_status()
            .context("get_versions returned error status")?;

        let body: ForgeVersionsResponse = resp
            .json()
            .await
            .context("get_versions: failed to parse response")?;
        Ok(body.data)
    }

    /// Resolve the dependency tree for a set of (mod_id, version_id) pairs.
    pub async fn get_dependencies(&self, mods: &[(i64, i64)]) -> Result<Vec<DependencyNode>> {
        let url = format!("{}/mods/dependencies", self.base_url);

        let payload: Vec<_> = mods
            .iter()
            .map(|(mod_id, version_id)| json!({ "mod_id": mod_id, "version_id": version_id }))
            .collect();

        let resp = self
            .client
            .request(reqwest::Method::GET, &url)
            .json(&payload)
            .send()
            .await
            .context("get_dependencies request failed")?
            .error_for_status()
            .context("get_dependencies returned error status")?;

        let nodes: Vec<DependencyNode> = resp
            .json()
            .await
            .context("get_dependencies: failed to parse response")?;
        Ok(nodes)
    }

    /// Check for available updates for the given mods.
    /// Each entry in `mods` is (mod_id, current_version_string).
    pub async fn check_updates(
        &self,
        mods: &[(i64, String)],
        spt_version: &str,
    ) -> Result<Vec<UpdateCheckResult>> {
        let url = format!("{}/mods/updates", self.base_url);

        let payload = json!({
            "spt_version": spt_version,
            "mods": mods.iter().map(|(id, ver)| {
                json!({ "mod_id": id, "current_version": ver })
            }).collect::<Vec<_>>(),
        });

        let resp = self
            .client
            .request(reqwest::Method::GET, &url)
            .json(&payload)
            .send()
            .await
            .context("check_updates request failed")?
            .error_for_status()
            .context("check_updates returned error status")?;

        let body: UpdatesResponse = resp
            .json()
            .await
            .context("check_updates: failed to parse response")?;
        Ok(body.data)
    }

    /// Download a file from `url` and write it to `dest`.
    pub async fn download_file(&self, url: &str, dest: &Path) -> Result<()> {
        use futures_util::StreamExt;

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("download_file request failed")?
            .error_for_status()
            .context("download_file returned error status")?;

        let mut stream = resp.bytes_stream();
        let mut file = tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("failed to create file: {}", dest.display()))?;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("error reading download stream")?;
            file.write_all(&bytes)
                .await
                .context("error writing to file")?;
        }

        file.flush().await.context("error flushing file")?;
        Ok(())
    }
}
