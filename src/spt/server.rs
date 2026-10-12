use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};

pub struct PingResult {
    pub ok: bool,
    pub latency_ms: u64,
}

#[derive(serde::Deserialize)]
struct LauncherResponse<T> {
    #[serde(rename = "Response")]
    response: T,
}

#[derive(serde::Deserialize)]
struct RegistrationResponse {
    #[serde(rename = "Response")]
    success: bool,
    #[serde(rename = "Profiles")]
    profiles: Vec<RegistrationProfile>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistrationProfile {
    username: Option<String>,
    profile_id: Option<String>,
}

pub(crate) fn registered_profile_id(body: &str, username: &str) -> Result<String> {
    let response: RegistrationResponse =
        serde_json::from_str(body).context("failed to decode launcher v2 registration response")?;
    anyhow::ensure!(response.success, "SPT profile registration was rejected");
    let mut matching = response
        .profiles
        .into_iter()
        .filter(|p| p.username.as_deref() == Some(username));
    let profile = matching
        .next()
        .context("registered profile is missing from response")?;
    anyhow::ensure!(
        matching.next().is_none(),
        "registration returned ambiguous profile identity"
    );
    profile
        .profile_id
        .filter(|id| !id.is_empty())
        .context("registered profile has no profile ID")
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
            .get(format!("{}/launcher/v2/ping", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await;

        let ok = match resp {
            Ok(r) if r.status().is_success() => r
                .json::<LauncherResponse<String>>()
                .await
                .is_ok_and(|reply| reply.response.eq_ignore_ascii_case("Pong!")),
            _ => false,
        };
        Ok(PingResult {
            ok,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    pub async fn server_version(&self) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/launcher/v2/version", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach SPT server for version")?
            .error_for_status()
            .context("SPT server version endpoint returned error")?;

        let reply: LauncherResponse<String> = resp
            .json()
            .await
            .context("failed to decode launcher v2 version response")?;
        anyhow::ensure!(
            !reply.response.trim().is_empty(),
            "SPT returned an empty version"
        );
        Ok(reply.response)
    }

    pub async fn loaded_server_mods(&self) -> Result<HashMap<String, serde_json::Value>> {
        let resp = self
            .client
            .get(format!("{}/launcher/v2/mods", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach SPT server for loaded mods")?
            .error_for_status()
            .context("SPT loaded mods endpoint returned error")?;

        let reply: LauncherResponse<HashMap<String, serde_json::Value>> = resp
            .json()
            .await
            .context("failed to decode launcher v2 mods response")?;
        Ok(reply.response)
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

    pub async fn register_profile(&self, username: &str, edition: &str) -> Result<String> {
        let body = serde_json::json!({
            "username": username,
            "edition": edition,
        });

        let resp = self
            .client
            .post(format!("{}/launcher/v2/register", self.base_url))
            .header("responsecompressed", "0")
            .json(&body)
            .send()
            .await
            .context("failed to reach SPT server for profile registration")?
            .error_for_status()
            .context("SPT server rejected profile registration")?;

        let body = resp
            .text()
            .await
            .context("failed to read SPT registration response")?;
        registered_profile_id(&body, username)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn launcher_v2_reads_unwrap_response_envelopes() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        for (route, body) in [
            (
                "/launcher/v2/ping",
                serde_json::json!({"Response": "Pong!"}),
            ),
            (
                "/launcher/v2/version",
                serde_json::json!({"Response": "4.1.6"}),
            ),
            (
                "/launcher/v2/mods",
                serde_json::json!({"Response": {"server": {"ModGuid": "Fika", "Version": "2.4.2"}}}),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
        }
        let mut client = SptClient::new("127.0.0.1", 6969).unwrap();
        client.base_url = server.uri();
        let ping = client.ping().await.unwrap();
        let version = client.server_version().await;
        let mods = client.loaded_server_mods().await;
        assert!(ping.ok);
        assert_eq!(version.unwrap(), "4.1.6");
        assert_eq!(mods.unwrap()["server"]["ModGuid"], "Fika");
    }

    #[test]
    fn registration_selects_exact_profile_and_rejects_invalid_identity() {
        let valid = r#"{"Response":true,"Profiles":[{"username":"other","profileId":"other-id"},{"username":"alice","profileId":"alice-id"}]}"#;
        assert_eq!(registered_profile_id(valid, "alice").unwrap(), "alice-id");
        for body in [
            r#"{"Response":false,"Profiles":[]}"#,
            r#"{"Response":true,"Profiles":[]}"#,
            r#"{"Response":true,"Profiles":[{"username":"alice","profileId":""}]}"#,
            r#"{"Response":true,"Profiles":[{"username":"alice","profileId":"one"},{"username":"alice","profileId":"two"}]}"#,
        ] {
            assert!(registered_profile_id(body, "alice").is_err());
        }
    }

    #[tokio::test]
    async fn registration_posts_once_with_current_request_shape() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/launcher/v2/register"))
            .and(body_json(
                serde_json::json!({"username":"alice", "edition":"Standard"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Response":true,"Profiles":[{"username":"alice","profileId":"alice-id"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut client = SptClient::new("127.0.0.1", 6969).unwrap();
        client.base_url = server.uri();
        assert_eq!(
            client.register_profile("alice", "Standard").await.unwrap(),
            "alice-id"
        );
    }

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

    #[test]
    fn spt_client_register_url() {
        let client = SptClient::new("192.168.1.10", 6969).unwrap();
        // Verify the URL that register_profile would target
        assert_eq!(
            format!("{}/launcher/v2/register", client.base_url()),
            "https://192.168.1.10:6969/launcher/v2/register"
        );
    }
}
