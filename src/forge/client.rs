use std::path::Path;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use tokio::io::AsyncWriteExt;

use super::models::*;

const DEFAULT_BASE_URL: &str = "https://forge.sp-tarkov.com/api/v0";

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

        Ok(Self { client, base_url })
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

    /// Resolve the dependency tree for a set of (mod_id, version_string) pairs.
    pub async fn get_dependencies(&self, mods: &[(i64, &str)]) -> Result<Vec<DependencyNode>> {
        let url = format!("{}/mods/dependencies", self.base_url);
        let mods_param: String = mods
            .iter()
            .map(|(id, ver)| format!("{id}:{ver}"))
            .collect::<Vec<_>>()
            .join(",");

        let resp = self
            .client
            .get(&url)
            .query(&[("mods", &mods_param)])
            .send()
            .await
            .context("get_dependencies request failed")?
            .error_for_status()
            .context("get_dependencies returned error status")?;

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

        let resp = self
            .client
            .get(&url)
            .query(&[
                ("mods", &mods_param),
                ("spt_version", &spt_version.to_string()),
            ])
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

#[cfg(test)]
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
}
