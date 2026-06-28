use std::path::Path;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use tokio::io::AsyncWriteExt;

use super::models::*;

const DEFAULT_BASE_URL: &str = "https://forge.sp-tarkov.com/api/v0";
const MAX_RETRIES: u32 = 2;

#[derive(Clone)]
pub struct ForgeClient {
    client: reqwest::Client,
    base_url: String,
}

impl ForgeClient {
    /// Create a new client pointing at the production Forge API.
    /// If `token` is provided it is sent as a Bearer token on every request.
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::build(DEFAULT_BASE_URL.to_string(), token)
    }

    #[allow(dead_code)] // used by integration tests
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
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { client, base_url })
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

    /// Search mods by query string.
    pub async fn search_mods(&self, query: &str) -> Result<Vec<ForgeMod>> {
        let url = format!("{}/mods", self.base_url);
        let req = self.client.get(&url).query(&[("query", query)]);

        let resp = self
            .send_with_retry(req)
            .await
            .context("search_mods request failed")?;

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

        let resp = self
            .send_with_retry(req)
            .await
            .context("get_mod request failed")?;

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

        let resp = self
            .send_with_retry(req)
            .await
            .context("get_versions request failed")?;

        let body: ForgeVersionsResponse = resp
            .json()
            .await
            .context("get_versions: failed to parse response")?;
        Ok(body.data)
    }

    /// Resolve the dependency tree for a set of (mod_id, version_string) pairs.
    pub async fn get_dependencies(&self, mods: &[(i64, &str)]) -> Result<Vec<DependencyNode>> {
        let url = format!("{}/mods/dependencies", self.base_url);
        let mods_param: String = mods
            .iter()
            .map(|(id, ver)| format!("{id}:{ver}"))
            .collect::<Vec<_>>()
            .join(",");

        let req = self.client.get(&url).query(&[("mods", &mods_param)]);

        let resp = self
            .send_with_retry(req)
            .await
            .context("get_dependencies request failed")?;

        let body: DependencyResponse = resp
            .json()
            .await
            .context("get_dependencies: failed to parse response")?;
        Ok(body.data)
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

        let timeout = std::time::Duration::from_secs(600);
        let resp = if self.is_forge_url(url) {
            self.client.get(url).timeout(timeout).send().await
        } else {
            // External host (GitLab, GitHub, etc.) — don't send Forge auth token
            reqwest::Client::new()
                .get(url)
                .timeout(timeout)
                .send()
                .await
        }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_client(server: &MockServer) -> ForgeClient {
        ForgeClient::with_base_url(server.uri(), None).unwrap()
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
        assert_eq!(mods[1].fika_compatibility, Some(FikaCompat::Incompatible));
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
        let deps = client.get_dependencies(&[(42, "1.2.0")]).await.unwrap();

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
            .expect(1)
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
    async fn download_file_external_url_omits_auth() {
        let forge_server = MockServer::start().await;
        let external_server = MockServer::start().await;
        let file_content = b"external file content";

        Mock::given(method("GET"))
            .and(path("/files/mod.zip"))
            .and(wiremock::matchers::header_exists("Authorization"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&external_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/files/mod.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(file_content.to_vec()))
            .mount(&external_server)
            .await;

        let client =
            ForgeClient::with_base_url(forge_server.uri(), Some("secret-token".into())).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("mod.zip");

        let url = format!("{}/files/mod.zip", external_server.uri());
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
            ForgeClient::with_base_url("https://forge.sp-tarkov.com/api/v0".into(), None).unwrap();

        // Unparseable URL should return false (don't leak auth token)
        assert!(!client.is_forge_url("not a url at all"));

        // Valid URL on different host should return false
        assert!(!client.is_forge_url("https://github.com/some/file.zip"));

        // Valid URL on same host should return true
        assert!(client.is_forge_url("https://forge.sp-tarkov.com/api/v0/files/download.zip"));
    }

    #[tokio::test]
    async fn auth_token_sent_in_header() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"data": []});

        Mock::given(method("GET"))
            .and(path("/mods"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer test-token-123",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client =
            ForgeClient::with_base_url(server.uri(), Some("test-token-123".to_string())).unwrap();

        let mods = client.search_mods("test").await.unwrap();
        assert!(mods.is_empty());
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
}
