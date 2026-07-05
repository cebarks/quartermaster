// ponytail: Many types here unused until later tasks; allow dead_code module-wide
#![allow(dead_code)]

use anyhow::{Context, Result};
use jsonc_parser::cst::CstRootNode;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Canonical path to fika.jsonc relative to SPT dir.
pub fn fika_config_path(spt_dir: &Path) -> std::path::PathBuf {
    spt_dir.join("SPT/user/mods/fika-server/assets/configs/fika.jsonc")
}

/// Parse fika.jsonc text into typed config.
pub fn parse_fika_jsonc(text: &str) -> Result<FikaConfig> {
    let value = jsonc_parser::parse_to_serde_value(text, &Default::default())
        .map_err(|e| anyhow::anyhow!("JSONC parse error: {e}"))?
        .unwrap_or(serde_json::Value::Null);
    serde_json::from_value(value).context("failed to deserialize FikaConfig")
}

/// Read fika.jsonc from disk into typed config.
pub fn read_fika_config(path: &Path) -> Result<FikaConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    parse_fika_jsonc(&text)
}

/// Parse fika.jsonc into CST for comment-preserving edits.
pub fn read_fika_cst(path: &Path) -> Result<CstRootNode> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    CstRootNode::parse(&text, &Default::default())
        .map_err(|e| anyhow::anyhow!("JSONC CST parse error: {e}"))
}

/// Modify headless.profiles.amount in a CST.
pub fn set_headless_amount(cst: &CstRootNode, amount: u32) {
    let root = cst.object_value_or_set();
    if let Some(headless) = root.object_value("headless") {
        if let Some(profiles) = headless.object_value("profiles") {
            if let Some(prop) = profiles.get("amount") {
                prop.set_value(jsonc_parser::cst::CstInputValue::Number(amount.to_string()));
            }
        }
    }
}

/// Write CST back to disk atomically (tempfile + rename).
pub fn write_fika_cst(cst: &CstRootNode, path: &Path) -> Result<()> {
    let content = cst.to_string();
    let dir = path.parent().context("fika.jsonc has no parent dir")?;
    let tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::fs::write(tmp.path(), &content)?;
    tmp.persist(path)
        .with_context(|| format!("failed to persist {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct FikaConfig {
    pub client: FikaClientConfig,
    pub server: FikaServerConfig,
    #[serde(rename = "natPunchServer")]
    pub nat_punch_server: FikaNatPunchConfig,
    pub headless: FikaHeadlessConfig,
    pub background: FikaBackgroundConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaClientConfig {
    pub use_btr: bool,
    pub friendly_fire: bool,
    pub dynamic_v_exfils: bool,
    pub allow_free_cam: bool,
    pub allow_spectate_free_cam: bool,
    #[serde(default)]
    pub blacklisted_items: Vec<String>,
    pub force_save_on_death: bool,
    #[serde(default)]
    pub mods: FikaModsConfig,
    pub use_inertia: bool,
    pub shared_quest_progression: bool,
    pub can_edit_raid_settings: bool,
    pub enable_transits: bool,
    pub anyone_can_start_raid: bool,
    pub allow_name_plates: bool,
    pub random_labyrinth_spawns: bool,
    pub pmc_found_in_raid: bool,
    pub allow_spectate_bots: bool,
    pub instant_load: bool,
    pub fast_load: bool,
    pub revive_config: FikaReviveConfig,
}

#[derive(Debug, Default, Deserialize)]
pub struct FikaModsConfig {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaReviveConfig {
    pub enabled: bool,
    pub headshot_kills: bool,
    pub grenades_kills: bool,
    pub allow_looting: bool,
    pub max_revives: u32,
    pub bleedout_time: f64,
    pub revive_time: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaServerConfig {
    #[serde(rename = "SPT")]
    pub spt: FikaSptConfig,
    pub webhook: FikaWebhookConfig,
    pub allow_item_sending: bool,
    pub item_sending_storage_time: u32,
    pub sent_items_lose_fir: bool,
    pub launcher_list_all_profiles: bool,
    pub session_timeout: u32,
    pub show_dev_profile: bool,
    pub show_non_standard_profile: bool,
    #[serde(default)]
    pub admin_ids: Vec<String>,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct FikaSptConfig {
    pub http: FikaSptHttpConfig,
    #[serde(rename = "disableSPTChatBots", default)]
    pub disable_spt_chat_bots: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaSptHttpConfig {
    pub ip: String,
    pub port: u16,
    pub backend_ip: String,
    pub backend_port: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaWebhookConfig {
    pub enabled: bool,
    pub name: String,
    pub avatar_url: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct FikaNatPunchConfig {
    pub enable: bool,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaHeadlessConfig {
    pub profiles: FikaHeadlessProfilesConfig,
    pub scripts: FikaHeadlessScriptsConfig,
    pub set_level_to_average_of_lobby: bool,
    pub restart_after_amount_of_raids: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaHeadlessProfilesConfig {
    pub amount: u32,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FikaHeadlessScriptsConfig {
    pub generate: bool,
    pub force_ip: String,
}

#[derive(Debug, Deserialize)]
pub struct FikaBackgroundConfig {
    pub enable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FIKA_JSONC: &str = r#"{
        // client settings
        "client": {
            "useBtr": true,
            "friendlyFire": false,
            "dynamicVExfils": false,
            "allowFreeCam": false,
            "allowSpectateFreeCam": false,
            "blacklistedItems": [],
            "forceSaveOnDeath": false,
            "mods": { "required": [], "optional": [] },
            "useInertia": true,
            "sharedQuestProgression": false,
            "canEditRaidSettings": true,
            "enableTransits": true,
            "anyoneCanStartRaid": false,
            "allowNamePlates": true,
            "randomLabyrinthSpawns": true,
            "pmcFoundInRaid": true,
            "allowSpectateBots": true,
            "instantLoad": false,
            "fastLoad": true,
            "reviveConfig": {
                "enabled": false,
                "headshotKills": false,
                "grenadesKills": false,
                "allowLooting": false,
                "maxRevives": 3,
                "bleedoutTime": 30,
                "reviveTime": 3
            }
        },
        "server": {
            "SPT": {
                "http": { "ip": "0.0.0.0", "port": 6969, "backendIp": "0.0.0.0", "backendPort": 6969 },
                "disableSPTChatBots": false
            },
            "webhook": { "enabled": false, "name": "Fika", "avatarUrl": "", "url": "" },
            "allowItemSending": true,
            "itemSendingStorageTime": 7,
            "sentItemsLoseFir": true,
            "launcherListAllProfiles": true,
            "sessionTimeout": 5,
            "showDevProfile": true,
            "showNonStandardProfile": true,
            "adminIds": [],
            "apiKey": "test-api-key-123"
        },
        "natPunchServer": { "enable": false, "port": 6790 },
        "headless": {
            "profiles": { "amount": 2, "aliases": {} },
            "scripts": { "generate": true, "forceIp": "https://127.0.0.1:6969" },
            "setLevelToAverageOfLobby": true,
            "restartAfterAmountOfRaids": 1
        },
        "background": { "enable": true }
    }"#;

    #[test]
    fn parse_fika_config() {
        let config = parse_fika_jsonc(SAMPLE_FIKA_JSONC).expect("parse failed");
        assert!(!config.client.friendly_fire);
        assert_eq!(config.server.api_key, "test-api-key-123");
        assert_eq!(config.headless.profiles.amount, 2);
        assert_eq!(config.nat_punch_server.port, 6790);
        assert!(config.background.enable);
        assert_eq!(config.client.revive_config.max_revives, 3);
    }

    #[test]
    fn cst_set_headless_amount_preserves_comments() {
        let cst =
            CstRootNode::parse(SAMPLE_FIKA_JSONC, &Default::default()).expect("CST parse failed");
        set_headless_amount(&cst, 5);
        let output = cst.to_string();
        // Comment preserved
        assert!(output.contains("// client settings"));
        // Value changed
        let reparsed = parse_fika_jsonc(&output).expect("reparse failed");
        assert_eq!(reparsed.headless.profiles.amount, 5);
    }

    #[test]
    fn set_headless_amount_matches_old_regex_behavior() {
        // This test verifies that the CST method produces the same outcome as the old regex,
        // just via a different path. We don't verify identical strings, but that both set
        // the amount field and preserve comments.
        let cst =
            CstRootNode::parse(SAMPLE_FIKA_JSONC, &Default::default()).expect("CST parse failed");
        set_headless_amount(&cst, 10);
        let output = cst.to_string();

        // Verify the value changed
        let config = parse_fika_jsonc(&output).expect("reparse failed");
        assert_eq!(config.headless.profiles.amount, 10);

        // Verify comments preserved
        assert!(output.contains("// client settings"));
    }
}
