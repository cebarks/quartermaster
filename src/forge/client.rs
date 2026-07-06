use std::path::Path;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use tokio::io::AsyncWriteExt;

use super::cache::ForgeResponseCache;
use super::models::*;

const DEFAULT_BASE_URL: &str = "https://forge.sp-tarkov.com/api/v0";
const MAX_RETRIES: u32 = 2;
const MAX_DOWNLOAD_SIZE: u64 = 500 * 1024 * 1024; // 500 MB

#[cfg(test)]
const EXTERNAL_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(not(test))]
const EXTERNAL_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(serde::Deserialize)]
struct DataWrapper<T> {
    data: T,
}

fn format_id_version_pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(id, ver)| format!("{id}:{ver}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone)]
pub struct ForgeClient {
    client: reqwest::Client,
    base_url: String,
    cache: ForgeResponseCache,
}

impl ForgeClient {
    pub fn new() -> Result<Self> {
        Self::build(DEFAULT_BASE_URL.to_string())
    }

    #[allow(dead_code)] // used by integration tests
    pub fn with_base_url(base_url: String) -> Result<Self> {
        Self::build(base_url)
    }

    fn build(base_url: String) -> Result<Self> {
        let mut headers = HeaderMap::new();

        let ua = format!("quartermaster/{}", env!("CARGO_PKG_VERSION"));
        headers.insert(USER_AGENT, HeaderValue::from_str(&ua)?);

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url,
            cache: ForgeResponseCache::new(256, 300),
        })
    }

    async fn send_with_retry(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let mut last_error = None;

        for attempt in 0..=MAX_RETRIES {
            let req = request
                .try_clone()
                .ok_or_else(|| anyhow::anyhow!("request not cloneable (has streaming body)"))?
                .send()
                .await;

            match req {
                Ok(resp) if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                    if attempt == MAX_RETRIES {
                        anyhow::bail!(
                            "Forge API rate limit exceeded after {} retries (HTTP 429)",
                            MAX_RETRIES
                        );
                    }
                    let retry_after = resp
                        .headers()
                        .get("Retry-After")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(5);
                    let wait = retry_after.min(60);
                    tracing::warn!(
                        attempt = attempt + 1,
                        retry_after = wait,
                        "Forge API rate limited, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                }
                Ok(resp) if resp.status().is_server_error() => {
                    if attempt == MAX_RETRIES {
                        return resp
                            .error_for_status()
                            .context("Forge API returned server error after retries");
                    }
                    let status = resp.status();
                    tracing::warn!(
                        attempt = attempt + 1,
                        status = status.as_u16(),
                        "Forge API returned server error, retrying immediately"
                    );
                }
                Ok(resp) => {
                    return resp
                        .error_for_status()
                        .context("Forge API returned error status");
                }
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        tracing::warn!(
                            attempt = attempt + 1,
                            error = %e,
                            "Forge API request failed, retrying immediately"
                        );
                    }
                    last_error = Some(e);
                    if attempt == MAX_RETRIES {
                        break;
                    }
                }
            }
        }

        Err(last_error
            .map(|e| anyhow::anyhow!(e))
            .unwrap_or_else(|| anyhow::anyhow!("request failed after retries"))
            .context("Forge API request failed"))
    }

    async fn send_cached(&self, request: reqwest::RequestBuilder) -> Result<Vec<u8>> {
        let built = request
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("request not cloneable"))?
            .build()
            .context("failed to build request")?;
        let cache_key = built.url().to_string();

        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(cached);
        }

        let resp = self.send_with_retry(request).await?;
        let body = resp.bytes().await.context("failed to read response body")?;
        let bytes = body.to_vec();
        // Only cache if the response is valid JSON — avoids poisoning the
        // cache with garbled 200 responses (CDN errors, partial responses).
        if serde_json::from_slice::<serde_json::Value>(&bytes).is_ok() {
            self.cache.insert(cache_key, bytes.clone());
        }
        Ok(bytes)
    }

    async fn fetch_and_parse<T: serde::de::DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
        context: &str,
    ) -> Result<T> {
        let body = self
            .send_cached(req)
            .await
            .with_context(|| format!("{context} request failed"))?;
        let wrapper: DataWrapper<T> = serde_json::from_slice(&body)
            .with_context(|| format!("{context}: failed to parse response"))?;
        Ok(wrapper.data)
    }

    /// Search mods by query string.
    pub async fn search_mods(&self, query: &str) -> Result<Vec<ForgeMod>> {
        let url = format!("{}/mods", self.base_url);
        let req = self.client.get(&url).query(&[("query", query)]);
        self.fetch_and_parse(req, "search_mods").await
    }

    /// Fetch a single mod by ID, optionally including its versions.
    pub async fn get_mod(&self, id: i64, include_versions: bool) -> Result<ForgeMod> {
        let url = format!("{}/mod/{}", self.base_url, id);
        let mut req = self.client.get(&url);
        if include_versions {
            req = req.query(&[("include", "versions")]);
        }
        self.fetch_and_parse(req, "get_mod").await
    }

    /// List versions for a mod, optionally filtered to a specific SPT version.
    pub async fn get_versions(
        &self,
        mod_id: i64,
        spt_version: Option<&str>,
    ) -> Result<Vec<ForgeVersion>> {
        let url = format!("{}/mod/{}/versions", self.base_url, mod_id);
        let mut req = self.client.get(&url).query(&[("per_page", "100")]);
        if let Some(v) = spt_version {
            req = req.query(&[("filter[spt_version]", v)]);
        }
        self.fetch_and_parse(req, "get_versions").await
    }

    /// Resolve the dependency tree for a set of (identifier, version) pairs.
    /// The identifier can be a numeric mod ID or a GUID string.
    pub async fn get_dependencies(&self, mods: &[(&str, &str)]) -> Result<Vec<DependencyNode>> {
        let url = format!("{}/mods/dependencies", self.base_url);
        let mods_param = format_id_version_pairs(mods);
        let req = self.client.get(&url).query(&[("mods", &mods_param)]);
        self.fetch_and_parse(req, "get_dependencies").await
    }

    /// Check for available updates for the given mods.
    /// Each entry in `mods` is (mod_id, current_version_string).
    pub async fn check_updates(
        &self,
        mods: &[(i64, String)],
        spt_version: &str,
    ) -> Result<UpdatesResponseData> {
        let url = format!("{}/mods/updates", self.base_url);
        let mods_param: String = mods
            .iter()
            .map(|(id, ver)| format!("{id}:{ver}"))
            .collect::<Vec<_>>()
            .join(",");

        let req = self.client.get(&url).query(&[
            ("mods", &mods_param),
            ("spt_version", &spt_version.to_string()),
        ]);

        let resp = self
            .send_with_retry(req)
            .await
            .context("check_updates request failed")?;

        let body: UpdatesResponse = resp
            .json()
            .await
            .context("check_updates: failed to parse response")?;
        Ok(body.data)
    }

    #[allow(dead_code)] // used in Task 5
    pub async fn search_addons(&self, query: &str) -> Result<Vec<ForgeAddon>> {
        let url = format!("{}/addons", self.base_url);
        let req = self.client.get(&url).query(&[("query", query)]);
        self.fetch_and_parse(req, "search_addons").await
    }

    #[allow(dead_code)] // used in Task 5
    pub async fn get_addon(&self, id: i64, include_versions: bool) -> Result<ForgeAddon> {
        let url = format!("{}/addon/{}", self.base_url, id);
        let mut req = self.client.get(&url);
        if include_versions {
            req = req.query(&[("include", "versions")]);
        }
        self.fetch_and_parse(req, "get_addon").await
    }

    #[allow(dead_code)] // used in Task 5
    pub async fn get_addon_versions(&self, addon_id: i64) -> Result<Vec<ForgeAddonVersion>> {
        let url = format!("{}/addon/{}/versions", self.base_url, addon_id);
        let req = self.client.get(&url);
        self.fetch_and_parse(req, "get_addon_versions").await
    }

    #[allow(dead_code)] // used in Task 4
    pub async fn get_addon_dependencies(
        &self,
        addons: &[(&str, &str)],
    ) -> Result<Vec<DependencyNode>> {
        let url = format!("{}/addons/dependencies", self.base_url);
        let addons_param = format_id_version_pairs(addons);
        let req = self.client.get(&url).query(&[("addons", &addons_param)]);
        self.fetch_and_parse(req, "get_addon_dependencies").await
    }

    /// Get all known SPT versions with mod counts and release links.
    #[allow(dead_code)] // plumbing for future UI
    pub async fn get_spt_versions(&self) -> Result<Vec<SptVersion>> {
        let url = format!("{}/spt/versions", self.base_url);
        let req = self.client.get(&url);
        self.fetch_and_parse(req, "get_spt_versions").await
    }

    fn is_forge_url(&self, url: &str) -> bool {
        let origin = |s: &str| {
            let u = reqwest::Url::parse(s).ok()?;
            Some((u.scheme().to_string(), u.host_str()?.to_string(), u.port()))
        };
        match (origin(url), origin(&self.base_url)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Download a file from `url` and write it to `dest`.
    pub async fn download_file(&self, url: &str, dest: &Path) -> Result<()> {
        use futures_util::StreamExt;

        let resp = if self.is_forge_url(url) {
            self.client
                .get(url)
                .timeout(std::time::Duration::from_secs(600))
                .send()
                .await
        } else {
            // External host (GitLab, GitHub, etc.) — different timeout config
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .read_timeout(EXTERNAL_READ_TIMEOUT)
                .build()
                .context("failed to build download client")?
                .get(url)
                .send()
                .await
        }
        .context("download_file request failed")?
        .error_for_status()
        .context("download_file returned error status")?;

        let total_size = resp.content_length();
        if let Some(size) = total_size {
            anyhow::ensure!(
                size <= MAX_DOWNLOAD_SIZE,
                "download too large ({:.0} MB, limit {:.0} MB)",
                size as f64 / (1024.0 * 1024.0),
                MAX_DOWNLOAD_SIZE as f64 / (1024.0 * 1024.0)
            );
        }
        let mut stream = resp.bytes_stream();
        let mut file = tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("failed to create file: {}", dest.display()))?;

        let mut downloaded: u64 = 0;
        let mut last_log = std::time::Instant::now();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk
                .context("error reading download stream (possible stall — no data received)")?;
            file.write_all(&bytes)
                .await
                .context("error writing to file")?;
            downloaded += bytes.len() as u64;
            anyhow::ensure!(
                downloaded <= MAX_DOWNLOAD_SIZE,
                "download exceeded size limit ({:.0} MB)",
                MAX_DOWNLOAD_SIZE as f64 / (1024.0 * 1024.0)
            );

            if last_log.elapsed() >= std::time::Duration::from_secs(10) {
                last_log = std::time::Instant::now();
                if let Some(total) = total_size {
                    tracing::info!(
                        downloaded_mb = downloaded / (1024 * 1024),
                        total_mb = total / (1024 * 1024),
                        pct = downloaded * 100 / total.max(1),
                        "download progress"
                    );
                } else {
                    tracing::info!(
                        downloaded_mb = downloaded / (1024 * 1024),
                        "download progress"
                    );
                }
            }
        }

        file.flush().await.context("error flushing file")?;

        tracing::debug!(
            url,
            size_mb = downloaded / (1024 * 1024),
            "download complete"
        );
        Ok(())
    }

    /// Make a GET request to a non-Forge URL, stripping the Authorization header.
    ///
    /// Reuses the client's connection pool and User-Agent but does not send
    /// the Forge Bearer token to external services.
    pub async fn get_external_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        use reqwest::header::AUTHORIZATION;
        self.client
            .get(url)
            .header(AUTHORIZATION, "") // override default Bearer token
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?
            .error_for_status()
            .with_context(|| format!("GET {url} returned error status"))?
            .json()
            .await
            .with_context(|| format!("failed to parse JSON from {url}"))
    }

    /// Invalidate all cached responses.
    #[allow(dead_code)] // public API for cache management, used by external consumers
    pub fn invalidate_cache(&self) {
        self.cache.invalidate_all();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_client(server: &MockServer) -> ForgeClient {
        ForgeClient::with_base_url(server.uri()).unwrap()
    }

    #[tokio::test]
    async fn search_mods_returns_results() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 42,
                    "name": "Big Brain",
                    "slug": "big-brain",
                    "description": "AI overhaul",
                    "fika_compatibility": true
                },
                {
                    "id": 99,
                    "name": "SAIN",
                    "slug": "sain",
                    "fika_compatibility": false
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mods"))
            .and(query_param("query", "brain"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let mods = client.search_mods("brain").await.unwrap();

        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].id, 42);
        assert_eq!(mods[0].name, "Big Brain");
        assert_eq!(mods[0].slug.as_deref(), Some("big-brain"));
        assert_eq!(mods[0].fika_compatibility, Some(FikaCompat::Compatible));
        assert_eq!(mods[1].id, 99);
        assert_eq!(mods[1].fika_compatibility, Some(FikaCompat::Unknown));
    }

    #[tokio::test]
    async fn get_mod_without_versions() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Big Brain",
                "slug": "big-brain",
                "description": "AI overhaul",
                "fika_compatibility": true
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, false).await.unwrap();

        assert_eq!(m.id, 42);
        assert_eq!(m.name, "Big Brain");
        assert!(m.versions.is_none());
    }

    #[tokio::test]
    async fn get_mod_with_versions() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Big Brain",
                "fika_compatibility": true,
                "versions": [
                    {
                        "id": 100,
                        "version": "1.2.0",
                        "spt_version": "3.9.0",
                        "link": "https://example.com/download",
                        "content_length": 1048576,
                        "fika_compatibility": "compatible",
                        "dependencies": []
                    },
                    {
                        "id": 101,
                        "version": "1.1.0",
                        "spt_version": "3.8.0"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, true).await.unwrap();

        let versions = m.versions.expect("should have versions");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, 100);
        assert_eq!(versions[0].version, "1.2.0");
        assert_eq!(
            versions[0].link.as_deref(),
            Some("https://example.com/download")
        );
        assert_eq!(versions[1].id, 101);
        assert!(versions[1].link.is_none());
    }

    #[tokio::test]
    async fn get_versions_with_spt_filter() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 100,
                    "version": "1.2.0",
                    "spt_version": "3.10.0",
                    "fika_compatibility": "compatible"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mod/42/versions"))
            .and(query_param("per_page", "100"))
            .and(query_param("filter[spt_version]", "3.10.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let versions = client.get_versions(42, Some("3.10.0")).await.unwrap();

        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].spt_version.as_deref(), Some("3.10.0"));
    }

    #[tokio::test]
    async fn get_versions_no_spt_filter_omits_param() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {"id": 100, "version": "1.2.0"},
                {"id": 101, "version": "1.1.0"}
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mod/42/versions"))
            .and(query_param("per_page", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let versions = client.get_versions(42, None).await.unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn check_updates_parses_response() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "spt_version": "3.10.0",
                "updates": [
                    {
                        "current_version": {
                            "id": 100,
                            "mod_id": 42,
                            "name": "Big Brain",
                            "slug": "big-brain",
                            "version": "1.1.0"
                        },
                        "recommended_version": {
                            "id": 200,
                            "version": "1.2.0",
                            "link": "https://example.com/dl",
                            "content_length": 2048,
                            "fika_compatibility": "compatible"
                        },
                        "update_reason": "newer version available"
                    }
                ],
                "blocked_updates": [],
                "up_to_date": [],
                "incompatible_with_spt": []
            }
        });

        Mock::given(method("GET"))
            .and(path("/mods/updates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client
            .check_updates(&[(42, "1.1.0".to_string())], "3.10.0")
            .await
            .unwrap();

        assert_eq!(result.spt_version, "3.10.0");
        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.updates[0].current_version.name, "Big Brain");
        assert_eq!(result.updates[0].recommended_version.version, "1.2.0");
        assert_eq!(result.updates[0].update_reason, "newer version available");
        assert!(result.blocked_updates.is_empty());
        assert!(result.incompatible_with_spt.is_empty());
    }

    #[tokio::test]
    async fn get_dependencies_parses_tree() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 42,
                    "name": "Big Brain",
                    "slug": "big-brain",
                    "latest_compatible_version": {
                        "id": 100,
                        "version": "1.2.0",
                        "spt_version_constraint": "~3.10.0",
                        "link": "https://example.com/dl",
                        "content_length": 2048,
                        "fika_compatibility": "compatible"
                    },
                    "dependencies": [
                        {
                            "id": 10,
                            "name": "CoreLib",
                            "slug": "corelib",
                            "latest_compatible_version": {
                                "id": 50,
                                "version": "0.5.0"
                            },
                            "dependencies": [],
                            "conflict": false
                        }
                    ],
                    "conflict": false
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mods/dependencies"))
            .and(query_param("mods", "42:1.2.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let deps = client.get_dependencies(&[("42", "1.2.0")]).await.unwrap();

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].id, 42);
        assert_eq!(deps[0].name, "Big Brain");
        assert!(!deps[0].conflict);

        let version = deps[0].latest_compatible_version.as_ref().unwrap();
        assert_eq!(version.version, "1.2.0");
        assert_eq!(version.spt_version.as_deref(), Some("~3.10.0"));

        assert_eq!(deps[0].dependencies.len(), 1);
        assert_eq!(deps[0].dependencies[0].name, "CoreLib");
        assert!(!deps[0].dependencies[0].conflict);
    }

    #[tokio::test]
    async fn get_dependencies_with_guid() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 42,
                    "guid": "com.example.big-brain",
                    "name": "Big Brain",
                    "slug": "big-brain",
                    "latest_compatible_version": {
                        "id": 100,
                        "version": "1.2.0"
                    },
                    "dependencies": [],
                    "conflict": false
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mods/dependencies"))
            .and(query_param("mods", "com.example.big-brain:1.2.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let deps = client
            .get_dependencies(&[("com.example.big-brain", "1.2.0")])
            .await
            .unwrap();

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "Big Brain");
        assert_eq!(deps[0].guid.as_deref(), Some("com.example.big-brain"));
    }

    #[tokio::test]
    async fn check_updates_formats_multiple_mods() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "spt_version": "3.10.0",
                "updates": [],
                "blocked_updates": [],
                "up_to_date": [],
                "incompatible_with_spt": []
            }
        });

        Mock::given(method("GET"))
            .and(path("/mods/updates"))
            .and(query_param("mods", "42:1.0.0,99:2.0.0"))
            .and(query_param("spt_version", "3.10.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client
            .check_updates(&[(42, "1.0.0".into()), (99, "2.0.0".into())], "3.10.0")
            .await
            .unwrap();

        assert_eq!(result.spt_version, "3.10.0");
        assert!(result.updates.is_empty());
    }

    #[tokio::test]
    async fn search_mods_404_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("anything").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn search_mods_500_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(500))
            .expect(3) // initial + 2 retries (5xx is now retried)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("anything").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_mod_not_found_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mod/99999"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.get_mod(99999, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_json_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(200).set_body_string("this is not json"))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn download_file_writes_to_disk() {
        let server = MockServer::start().await;
        let file_content = b"fake archive content for testing";

        Mock::given(method("GET"))
            .and(path("/files/test.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(file_content.to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("downloaded.zip");

        let url = format!("{}/files/test.zip", server.uri());
        client.download_file(&url, &dest).await.unwrap();

        let written = std::fs::read(&dest).unwrap();
        assert_eq!(written, file_content);
    }

    #[tokio::test]
    async fn download_file_404_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/files/missing.zip"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("missing.zip");

        let url = format!("{}/files/missing.zip", server.uri());
        let result = client.download_file(&url, &dest).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fika_compat_bool_on_mod_object() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Test Mod",
                "fika_compatibility": true
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, false).await.unwrap();
        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));
    }

    #[tokio::test]
    async fn fika_compat_string_on_version_object() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Test Mod",
                "fika_compatibility": true,
                "versions": [
                    {
                        "id": 100,
                        "version": "1.0.0",
                        "fika_compatibility": "incompatible"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, true).await.unwrap();

        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));
        let v = &m.versions.unwrap()[0];
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Incompatible));
    }

    #[tokio::test]
    async fn abbreviated_versions_missing_optional_fields() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Test Mod",
                "fika_compatibility": true,
                "versions": [
                    {
                        "id": 100,
                        "version": "1.0.0"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, true).await.unwrap();
        let v = &m.versions.unwrap()[0];

        assert!(v.link.is_none());
        assert!(v.content_length.is_none());
        assert!(v.fika_compatibility.is_none());
        assert!(v.spt_version.is_none());
        assert!(v.dependencies.is_none());
    }

    #[test]
    fn is_forge_url_returns_false_on_parse_failure() {
        let client =
            ForgeClient::with_base_url("https://forge.sp-tarkov.com/api/v0".into()).unwrap();

        // Unparseable URL should return false
        assert!(!client.is_forge_url("not a url at all"));

        // Valid URL on different host should return false
        assert!(!client.is_forge_url("https://github.com/some/file.zip"));

        // Valid URL on same host should return true
        assert!(client.is_forge_url("https://forge.sp-tarkov.com/api/v0/files/download.zip"));
    }

    #[tokio::test]
    async fn retries_on_429_with_retry_after() {
        let server = MockServer::start().await;
        let body_429 = serde_json::json!({
            "success": false,
            "code": "RATE_LIMITED",
            "message": "Too many requests."
        });
        let body_ok = serde_json::json!({
            "data": [{"id": 1, "name": "Test Mod"}]
        });

        // Mount 429 mock FIRST (matches first, exhausted after 1 hit)
        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(&body_429)
                    .insert_header("Retry-After", "0"),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        // Mount success mock SECOND (matches after 429 exhausted)
        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body_ok))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let mods = client.search_mods("test").await.unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "Test Mod");
    }

    #[tokio::test]
    async fn gives_up_after_max_retries_on_429() {
        let server = MockServer::start().await;
        let body_429 = serde_json::json!({
            "success": false,
            "code": "RATE_LIMITED",
            "message": "Too many requests."
        });

        // Always returns 429
        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(&body_429)
                    .insert_header("Retry-After", "0"),
            )
            .expect(3) // initial + 2 retries
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("test").await;
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(
            err_msg.contains("rate limit") || err_msg.contains("429"),
            "error should mention rate limiting: {err_msg}"
        );
    }

    #[tokio::test]
    async fn retries_on_5xx_server_error() {
        let server = MockServer::start().await;
        let body_ok = serde_json::json!({
            "data": [{"id": 1, "name": "Test Mod"}]
        });

        // Mount 502 mock FIRST (FIFO — matches first, exhausted after 1 hit)
        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        // Mount success mock SECOND (matches after 502 exhausted)
        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body_ok))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let mods = client.search_mods("test").await.unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "Test Mod");
    }

    #[tokio::test]
    async fn gives_up_after_max_retries_on_5xx() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(500))
            .expect(3) // initial + 2 retries
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_json_200_not_cached() {
        let server = MockServer::start().await;
        let good_body = serde_json::json!({
            "data": {"id": 42, "name": "Good Mod"}
        });

        // Mount garbled response FIRST (FIFO — matches first, exhausted after 1 hit)
        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param_is_missing("include"))
            .respond_with(ResponseTemplate::new(200).set_body_string("this is not json"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        // Mount good response SECOND (matches after garbled exhausted)
        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param_is_missing("include"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&good_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;

        // First call gets garbled response — should fail but NOT cache it
        let result = client.get_mod(42, false).await;
        assert!(result.is_err());

        // Second call should hit the server again (not cached), get good response
        let m = client.get_mod(42, false).await.unwrap();
        assert_eq!(m.id, 42);
        assert_eq!(m.name, "Good Mod");
    }

    #[tokio::test]
    async fn caches_identical_get_requests() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {"id": 42, "name": "Cached Mod"}
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1) // only 1 request should reach the server
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m1 = client.get_mod(42, false).await.unwrap();
        let m2 = client.get_mod(42, false).await.unwrap();

        assert_eq!(m1.id, 42);
        assert_eq!(m2.id, 42);
    }

    #[tokio::test]
    async fn cache_distinguishes_different_params() {
        let server = MockServer::start().await;
        let body_with_ver = serde_json::json!({
            "data": {
                "id": 42,
                "name": "With Versions",
                "versions": [{"id": 1, "version": "1.0.0"}]
            }
        });
        let body_no_ver = serde_json::json!({
            "data": {"id": 42, "name": "No Versions"}
        });

        // Mount with-versions mock (matches requests with ?include=versions)
        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body_with_ver))
            .expect(1)
            .mount(&server)
            .await;

        // Mount without-versions mock (matches requests WITHOUT include param)
        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param_is_missing("include"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body_no_ver))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m1 = client.get_mod(42, false).await.unwrap();
        let m2 = client.get_mod(42, true).await.unwrap();

        assert_eq!(m1.name, "No Versions");
        assert_eq!(m2.name, "With Versions");
    }

    #[tokio::test]
    async fn check_updates_not_cached() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "spt_version": "3.10.0",
                "updates": [],
                "blocked_updates": [],
                "up_to_date": [],
                "incompatible_with_spt": []
            }
        });

        Mock::given(method("GET"))
            .and(path("/mods/updates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(2) // should hit server twice, not cached
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let _ = client
            .check_updates(&[(42, "1.0.0".into())], "3.10.0")
            .await
            .unwrap();
        let _ = client
            .check_updates(&[(42, "1.0.0".into())], "3.10.0")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn search_addons_returns_results() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {"id": 1, "name": "Music Pack", "slug": "music-pack", "mod_id": 5}
            ]
        });

        Mock::given(method("GET"))
            .and(path("/addons"))
            .and(query_param("query", "music"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let addons = client.search_addons("music").await.unwrap();
        assert_eq!(addons.len(), 1);
        assert_eq!(addons[0].name, "Music Pack");
    }

    #[tokio::test]
    async fn get_addon_returns_details() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {"id": 1, "name": "Music Pack", "mod_id": 5}
        });

        Mock::given(method("GET"))
            .and(path("/addon/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let addon = client.get_addon(1, false).await.unwrap();
        assert_eq!(addon.id, 1);
        assert_eq!(addon.name, "Music Pack");
    }

    #[tokio::test]
    async fn get_addon_versions_returns_list() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 1,
                    "version": "1.2.0",
                    "mod_version_constraint": "^2.0.0",
                    "downloads": 523
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/addon/1/versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let versions = client.get_addon_versions(1).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "1.2.0");
    }

    #[tokio::test]
    async fn get_spt_versions_returns_list() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 2,
                    "version": "3.11.3",
                    "version_major": 3,
                    "version_minor": 11,
                    "version_patch": 3,
                    "version_labels": "",
                    "mod_count": 371,
                    "link": "https://github.com/sp-tarkov/build/releases/tag/3.11.3",
                    "color_class": "green",
                    "created_at": "2025-04-08T19:29:40.000000Z",
                    "updated_at": "2025-04-08T19:29:40.000000Z"
                },
                {
                    "id": 3,
                    "version": "3.11.2",
                    "mod_count": 371
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/spt/versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let versions = client.get_spt_versions().await.unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "3.11.3");
        assert_eq!(versions[0].mod_count, Some(371));
        assert_eq!(versions[1].version, "3.11.2");
    }

    #[tokio::test]
    async fn download_file_stall_times_out() {
        let forge_server = MockServer::start().await;
        let external_server = MockServer::start().await;

        // wiremock's set_delay delays the entire response (headers + body).
        // This tests the header-stall path. read_timeout also covers bytes_stream()
        // per reqwest docs, but that path isn't directly testable with wiremock.
        Mock::given(method("GET"))
            .and(path("/files/huge.7z"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0u8; 1024])
                    .set_delay(std::time::Duration::from_secs(10)),
            )
            .mount(&external_server)
            .await;

        let client = ForgeClient::with_base_url(forge_server.uri()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("huge.7z");

        let url = format!("{}/files/huge.7z", external_server.uri());
        let result = client.download_file(&url, &dest).await;
        assert!(result.is_err(), "stalled download should time out");
    }
}
