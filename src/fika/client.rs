// ponytail: Many types here unused until later tasks; allow dead_code module-wide
#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct FikaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl FikaClient {
    pub fn new(base_url: &str, api_key: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.to_string(),
            api_key,
        })
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// POST /fika/api/restartheadless — send ShutdownClient WS message to headless
    pub async fn shutdown_headless(&self, profile_id: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.api_url("/fika/api/restartheadless"))
            .bearer_auth(&self.api_key)
            .header("requestcompressed", "0")
            .json(&serde_json::json!({ "profileId": profile_id }))
            .send()
            .await
            .context("failed to call Fika restartheadless API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Fika restartheadless returned {status}: {body}");
        }
        Ok(())
    }

    /// POST /fika/presence/get — player presence (SPT static route)
    pub async fn presence(&self) -> Result<Vec<FikaPlayerPresence>> {
        let resp = self
            .http
            .post(self.api_url("/fika/presence/get"))
            .header("requestcompressed", "0")
            .header("responsecompressed", "0")
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("failed to call Fika presence API")?;
        let body: Vec<FikaPlayerPresence> = resp
            .json()
            .await
            .context("failed to parse Fika presence response")?;
        Ok(body)
    }

    /// POST /fika/raid/headless/start — trigger headless raid (SPT static route)
    pub async fn start_headless_raid(
        &self,
        req: &StartHeadlessRaidRequest,
    ) -> Result<StartHeadlessRaidResponse> {
        let resp = self
            .http
            .post(self.api_url("/fika/raid/headless/start"))
            .header("requestcompressed", "0")
            .header("responsecompressed", "0")
            .json(req)
            .send()
            .await
            .context("failed to call Fika start headless raid API")?;
        resp.json()
            .await
            .context("failed to parse start raid response")
    }

    /// POST /fika/notification/push — broadcast notification (SPT static route)
    pub async fn push_notification(&self, message: &str, icon: u8) -> Result<()> {
        let resp = self
            .http
            .post(self.api_url("/fika/notification/push"))
            .header("requestcompressed", "0")
            .header("responsecompressed", "0")
            .json(&serde_json::json!({
                "notification": message,
                "notificationIcon": icon,
                "type": 3
            }))
            .send()
            .await
            .context("failed to push Fika notification")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Fika push notification returned {status}: {body}");
        }
        Ok(())
    }

    /// POST /fika/api/sendmessage — send message to a player
    pub async fn send_message(&self, profile_id: &str, message: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.api_url("/fika/api/sendmessage"))
            .bearer_auth(&self.api_key)
            .header("requestcompressed", "0")
            .json(&serde_json::json!({
                "profileId": profile_id,
                "message": message
            }))
            .send()
            .await
            .context("failed to send Fika message")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Fika sendmessage returned {status}: {body}");
        }
        Ok(())
    }

    /// GET /fika/api/players — online players list
    pub async fn players(&self) -> Result<FikaPlayersResponse> {
        let resp = self
            .http
            .get(self.api_url("/fika/api/players"))
            .bearer_auth(&self.api_key)
            .header("requestcompressed", "0")
            .send()
            .await
            .context("failed to call Fika players API")?;
        resp.json()
            .await
            .context("failed to parse Fika players response")
    }

    /// GET /fika/api/items — list all sendable items
    pub async fn get_items(&self) -> Result<FikaGetItemsResponse> {
        let resp = self
            .http
            .get(self.api_url("/fika/api/items"))
            .bearer_auth(&self.api_key)
            .header("requestcompressed", "0")
            .send()
            .await
            .context("failed to call Fika items API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Fika items API returned {status}: {body}");
        }
        resp.json()
            .await
            .context("failed to parse Fika items response")
    }

    /// POST /fika/api/senditem — send item to a player via in-game mail
    pub async fn send_item(&self, req: &FikaSendItemRequest) -> Result<()> {
        let resp = self
            .http
            .post(self.api_url("/fika/api/senditem"))
            .bearer_auth(&self.api_key)
            .header("requestcompressed", "0")
            .json(req)
            .send()
            .await
            .context("failed to call Fika senditem API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Fika senditem returned {status}: {body}");
        }
        Ok(())
    }

    /// POST /fika/api/senditemtoall — send item to multiple players
    pub async fn send_item_to_all(&self, req: &FikaSendItemToAllRequest) -> Result<()> {
        let resp = self
            .http
            .post(self.api_url("/fika/api/senditemtoall"))
            .bearer_auth(&self.api_key)
            .header("requestcompressed", "0")
            .json(req)
            .send()
            .await
            .context("failed to call Fika senditemtoall API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Fika senditemtoall returned {status}: {body}");
        }
        Ok(())
    }
}

impl std::fmt::Debug for FikaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FikaClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

// --- Response types ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaPlayerPresence {
    pub profile_id: String,
    pub nickname: String,
    pub level: i32,
    pub activity: u8,
    pub activity_started_timestamp: i64,
    pub raid_information: Option<FikaRaidInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaRaidInfo {
    pub location: String,
    pub side: i32,
    pub time: i32,
    pub started: bool,
    pub match_id: String,
}

#[derive(Debug, Deserialize)]
pub struct FikaPlayersResponse {
    pub players: Vec<FikaPlayer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaPlayer {
    pub profile_id: String,
    pub nickname: String,
    pub level: i32,
    pub location: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartHeadlessRaidRequest {
    pub headless_session_id: String,
    pub location_id: String,
    pub time: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_and_weather_settings: Option<serde_json::Value>,
    pub use_event: bool,
    pub side: i32,
    pub spawn_place: i32,
    pub metabolism_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_settings: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waves_settings: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_raid_settings: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartHeadlessRaidResponse {
    pub match_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FikaGetItemsResponse {
    pub items: HashMap<String, FikaItemInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FikaItemInfo {
    pub name: String,
    pub description: String,
    #[serde(rename = "stackable")]
    pub stack_amount: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaSendItemRequest {
    pub profile_id: String,
    pub item_tpl: String,
    pub amount: i32,
    pub message: String,
    pub fir: bool,
    pub expiration_days: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaSendItemToAllRequest {
    pub profile_ids: Vec<String>,
    pub item_tpl: String,
    pub amount: i32,
    pub message: String,
    pub fir: bool,
    pub expiration_days: i32,
}
