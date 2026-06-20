use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SptProfile {
    pub aid: String,
    pub username: String,
}

#[derive(Debug, Clone, Default)]
pub struct SptProfileStats {
    pub nickname: Option<String>,
    pub level: Option<i64>,
    pub side: Option<String>,
    pub experience: Option<i64>,
    pub registration_date: Option<i64>,
    pub raid_count: Option<i64>,
    pub survival_rate: Option<f64>,
    pub kill_count: Option<usize>,
    pub scav_kills: Option<i64>,
}

pub enum ProfileStatus {
    Found(SptProfileStats),
    NotFound,
    ParseError,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuestState {
    Locked,
    AvailableForStart,
    Started,
    AvailableForFinish,
    Success,
    Fail,
    Expired,
    Unknown(i32),
}

impl QuestState {
    fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Locked,
            2 => Self::AvailableForStart,
            3 => Self::Started,
            4 => Self::AvailableForFinish,
            5 => Self::Success,
            6 => Self::Fail,
            7 => Self::Expired,
            other => Self::Unknown(other),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Locked => "Locked",
            Self::AvailableForStart => "Available",
            Self::Started => "Started",
            Self::AvailableForFinish => "Ready",
            Self::Success => "Completed",
            Self::Fail => "Failed",
            Self::Expired => "Expired",
            Self::Unknown(_) => "Unknown",
        }
    }

    pub fn css_class(&self) -> &str {
        match self {
            Self::Success => "badge-success",
            Self::Started | Self::AvailableForFinish => "badge-info",
            Self::AvailableForStart => "badge-warning",
            Self::Fail => "badge-danger",
            Self::Locked | Self::Expired | Self::Unknown(_) => "badge-muted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuestEntry {
    pub qid: String,
    pub status: QuestState,
}

#[derive(Debug, Clone)]
pub struct TraderEntry {
    pub trader_id: String,
    pub loyalty_level: i32,
    pub standing: f64,
    pub sales_sum: f64,
}

#[derive(Debug, Clone)]
pub struct HideoutAreaEntry {
    pub area_type: i32,
    pub level: i32,
}

#[derive(Debug, Clone)]
pub struct ProfileDetail {
    pub stats: SptProfileStats,
    pub quests: Vec<QuestEntry>,
    pub traders: Vec<TraderEntry>,
    pub hideout: Vec<HideoutAreaEntry>,
    pub stash_value: Option<i64>,
}

#[derive(Deserialize)]
struct ProfileJson {
    info: ProfileInfo,
}

#[derive(Deserialize)]
struct ProfileInfo {
    id: String,
    username: String,
}

#[derive(Deserialize, Default)]
struct FullProfileJson {
    info: Option<FullProfileInfo>,
    characters: Option<Characters>,
}

#[derive(Deserialize, Default)]
struct FullProfileInfo {
    id: Option<String>,
}

#[derive(Deserialize, Default)]
struct Characters {
    pmc: Option<PmcData>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct PmcData {
    info: Option<PmcInfo>,
    stats: Option<PmcStats>,
    quests: Option<Vec<ProfileQuestEntry>>,
    traders_info: Option<HashMap<String, ProfileTraderInfo>>,
    hideout: Option<HideoutData>,
    inventory: Option<InventoryData>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct PmcInfo {
    nickname: Option<String>,
    level: Option<i64>,
    side: Option<String>,
    experience: Option<i64>,
    registration_date: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct PmcStats {
    eft: Option<EftStats>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct EftStats {
    overall_counters: Option<OverallCounters>,
    victims: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct OverallCounters {
    items: Option<Vec<CounterItem>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CounterItem {
    key: Vec<String>,
    value: i64,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ProfileQuestEntry {
    qid: String,
    status: i32,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
#[allow(dead_code)]
struct ProfileTraderInfo {
    loyaltyLevel: Option<i32>,
    standing: Option<f64>,
    salesSum: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct HideoutData {
    areas: Option<Vec<HideoutAreaJson>>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct HideoutAreaJson {
    #[serde(rename = "type")]
    area_type: i32,
    level: i32,
}

#[derive(Deserialize, Default, Debug, Clone)]
struct InventoryData {
    stash: Option<String>,
    items: Option<Vec<InventoryItem>>,
}

#[derive(Deserialize, Debug, Clone)]
struct InventoryItem {
    #[serde(rename = "_id")]
    id: String,
    #[serde(rename = "_tpl")]
    tpl: String,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    upd: Option<ItemUpd>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
struct ItemUpd {
    stack_objects_count: Option<i64>,
}

fn extract_stats_from_pmc(pmc: PmcData) -> SptProfileStats {
    let mut stats = SptProfileStats::default();

    if let Some(info) = pmc.info {
        stats.nickname = info.nickname;
        stats.level = info.level;
        stats.side = info.side;
        stats.experience = info.experience;
        stats.registration_date = info.registration_date;
    }

    if let Some(pmc_stats) = pmc.stats {
        if let Some(eft) = pmc_stats.eft {
            if let Some(counters) = eft.overall_counters.and_then(|c| c.items) {
                let mut total_raids: i64 = 0;
                let mut survived: i64 = 0;
                let mut scav_kills: Option<i64> = None;
                for item in &counters {
                    if item.key.len() >= 2 && item.key[0] == "Sessions" && item.key[1] == "Pmc" {
                        total_raids += item.value;
                        if item.key.len() == 3 && item.key[2] == "Survived" {
                            survived = item.value;
                        }
                    }
                    if item.key.len() == 1 && item.key[0] == "KilledSavages" {
                        scav_kills = Some(item.value);
                    }
                }
                stats.scav_kills = scav_kills;
                if total_raids > 0 {
                    stats.raid_count = Some(total_raids);
                    stats.survival_rate = Some(
                        (survived as f64 / total_raids as f64 * 100.0 * 100.0).round() / 100.0,
                    );
                } else {
                    stats.raid_count = Some(0);
                }
            }

            stats.kill_count = eft.victims.map(|v| v.len());
        }
    }

    stats
}

fn calculate_stash_value(inventory: &InventoryData, prices: &HashMap<String, i64>) -> i64 {
    let stash_id = match &inventory.stash {
        Some(id) => id,
        None => return 0,
    };
    let items = match &inventory.items {
        Some(items) => items,
        None => return 0,
    };

    let mut children: HashMap<&str, Vec<&InventoryItem>> = HashMap::new();
    for item in items {
        if let Some(ref pid) = item.parent_id {
            children.entry(pid.as_str()).or_default().push(item);
        }
    }

    let mut total: i64 = 0;
    let mut stack = vec![stash_id.as_str()];
    while let Some(parent) = stack.pop() {
        if let Some(kids) = children.get(parent) {
            for item in kids {
                let count = item
                    .upd
                    .as_ref()
                    .and_then(|u| u.stack_objects_count)
                    .unwrap_or(1);
                if let Some(&price) = prices.get(&item.tpl) {
                    total = total.saturating_add(price.saturating_mul(count));
                }
                stack.push(&item.id);
            }
        }
    }

    total
}

pub fn list_profiles(spt_dir: &Path) -> Result<Vec<SptProfile>> {
    let profiles_dir = spt_dir.join("SPT/user/profiles");
    if !profiles_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    let entries = std::fs::read_dir(&profiles_dir)
        .with_context(|| format!("failed to read {}", profiles_dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let parsed: ProfileJson = match serde_json::from_str(&contents) {
            Ok(p) => p,
            Err(_) => continue,
        };

        profiles.push(SptProfile {
            aid: parsed.info.id,
            username: parsed.info.username,
        });
    }

    profiles.sort_by(|a, b| a.username.cmp(&b.username));
    Ok(profiles)
}

pub fn load_all_profile_stats(spt_dir: &Path) -> HashMap<String, SptProfileStats> {
    let profiles_dir = spt_dir.join("SPT/user/profiles");
    let mut map = HashMap::new();

    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(e) => e,
        Err(_) => return map,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let parsed: FullProfileJson = match serde_json::from_str(&contents) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to parse SPT profile");
                continue;
            }
        };

        let aid = match parsed.info.and_then(|i| i.id) {
            Some(id) => id,
            None => continue,
        };

        let stats = match parsed.characters.and_then(|c| c.pmc) {
            Some(pmc) => extract_stats_from_pmc(pmc),
            None => SptProfileStats::default(),
        };

        map.insert(aid, stats);
    }

    map
}

pub fn load_profile_detail(
    spt_dir: &Path,
    profile_id: &str,
    prices: &HashMap<String, i64>,
) -> Result<Option<ProfileDetail>> {
    let path = spt_dir
        .join("SPT/user/profiles")
        .join(format!("{profile_id}.json"));
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read profile {}", path.display()))?;
    let parsed: FullProfileJson = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse profile {}", path.display()))?;

    let mut pmc = match parsed.characters.and_then(|c| c.pmc) {
        Some(pmc) => pmc,
        None => return Ok(None),
    };

    // Fresh/wiped profile: Info is None means no character data yet
    if pmc.info.is_none() {
        return Ok(None);
    }

    // Extract fields we need before consuming pmc
    let quests = pmc
        .quests
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|q| QuestEntry {
            qid: q.qid,
            status: QuestState::from_i32(q.status),
        })
        .collect();

    let traders = pmc
        .traders_info
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, t)| TraderEntry {
            trader_id: id,
            loyalty_level: t.loyaltyLevel.unwrap_or(0),
            standing: t.standing.unwrap_or(0.0),
            sales_sum: t.salesSum.unwrap_or(0.0),
        })
        .collect();

    let hideout = pmc
        .hideout
        .take()
        .and_then(|h| h.areas)
        .unwrap_or_default()
        .into_iter()
        .map(|a| HideoutAreaEntry {
            area_type: a.area_type,
            level: a.level,
        })
        .collect();

    let stash_value = pmc
        .inventory
        .as_ref()
        .map(|inv| calculate_stash_value(inv, prices));

    // Now consume pmc for stats extraction
    let stats = extract_stats_from_pmc(pmc);

    Ok(Some(ProfileDetail {
        stats,
        quests,
        traders,
        hideout,
        stash_value,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_fake_profile(dir: &Path, aid: &str, username: &str) {
        let profiles_dir = dir.join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = format!(r#"{{"info":{{"id":"{}","username":"{}"}}}}"#, aid, username);
        std::fs::write(profiles_dir.join(format!("{aid}.json")), content).unwrap();
    }

    #[test]
    fn list_profiles_finds_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_profile(tmp.path(), "abc123", "Player1");
        create_fake_profile(tmp.path(), "def456", "Player2");

        let profiles = list_profiles(tmp.path()).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].username, "Player1");
        assert_eq!(profiles[1].username, "Player2");
    }

    #[test]
    fn list_profiles_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/profiles")).unwrap();
        let profiles = list_profiles(tmp.path()).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn list_profiles_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles = list_profiles(tmp.path()).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn list_profiles_skips_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_profile(tmp.path(), "good1", "GoodPlayer");
        let profiles_dir = tmp.path().join("SPT/user/profiles");
        std::fs::write(profiles_dir.join("bad.json"), "not json").unwrap();

        let profiles = list_profiles(tmp.path()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].username, "GoodPlayer");
    }

    fn create_full_profile(dir: &Path, aid: &str, username: &str) {
        let profiles_dir = dir.join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = serde_json::json!({
            "info": {"id": aid, "username": username},
            "characters": {
                "pmc": {
                    "Info": {
                        "Nickname": username,
                        "Level": 42,
                        "Side": "Usec",
                        "Experience": 1234567,
                        "RegistrationDate": 1700000000
                    },
                    "Stats": {
                        "Eft": {
                            "OverallCounters": {
                                "Items": [
                                    {"Key": ["Sessions", "Pmc", "Survived"], "Value": 30},
                                    {"Key": ["Sessions", "Pmc", "Died"], "Value": 10},
                                    {"Key": ["Sessions", "Pmc", "RunThrough"], "Value": 5},
                                    {"Key": ["KilledSavages"], "Value": 100}
                                ]
                            },
                            "Victims": [
                                {"Name": "Bot1"},
                                {"Name": "Bot2"},
                                {"Name": "Bot3"}
                            ]
                        }
                    }
                }
            }
        });
        std::fs::write(
            profiles_dir.join(format!("{aid}.json")),
            serde_json::to_string(&content).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_profile_stats_full() {
        let tmp = tempfile::tempdir().unwrap();
        create_full_profile(tmp.path(), "abc123", "Player1");

        let stats = load_all_profile_stats(tmp.path());
        assert_eq!(stats.len(), 1);
        let s = &stats["abc123"];
        assert_eq!(s.nickname.as_deref(), Some("Player1"));
        assert_eq!(s.level, Some(42));
        assert_eq!(s.side.as_deref(), Some("Usec"));
        assert_eq!(s.experience, Some(1234567));
        assert_eq!(s.registration_date, Some(1700000000));
        assert_eq!(s.raid_count, Some(45)); // 30 + 10 + 5
        assert!((s.survival_rate.unwrap() - 66.67).abs() < 0.1); // 30/45*100
        assert_eq!(s.kill_count, Some(3));
    }

    #[test]
    fn load_profile_stats_missing_pmc() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = r#"{"info":{"id":"abc","username":"NoCharacters"}}"#;
        std::fs::write(profiles_dir.join("abc.json"), content).unwrap();

        let stats = load_all_profile_stats(tmp.path());
        assert_eq!(stats.len(), 1);
        let s = &stats["abc"];
        assert!(s.nickname.is_none());
        assert!(s.level.is_none());
    }

    #[test]
    fn load_profile_stats_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/profiles")).unwrap();
        let stats = load_all_profile_stats(tmp.path());
        assert!(stats.is_empty());
    }

    #[test]
    fn load_profile_stats_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let stats = load_all_profile_stats(tmp.path());
        assert!(stats.is_empty());
    }

    #[test]
    fn load_profile_stats_zero_raids() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = serde_json::json!({
            "info": {"id": "new1", "username": "Newbie"},
            "characters": {
                "pmc": {
                    "Info": {"Nickname": "Newbie", "Level": 1, "Side": "Bear"},
                    "Stats": {
                        "Eft": {
                            "OverallCounters": {"Items": []},
                            "Victims": []
                        }
                    }
                }
            }
        });
        std::fs::write(
            profiles_dir.join("new1.json"),
            serde_json::to_string(&content).unwrap(),
        )
        .unwrap();

        let stats = load_all_profile_stats(tmp.path());
        let s = &stats["new1"];
        assert_eq!(s.nickname.as_deref(), Some("Newbie"));
        assert_eq!(s.level, Some(1));
        assert_eq!(s.raid_count, Some(0));
        assert!(s.survival_rate.is_none());
        assert_eq!(s.kill_count, Some(0));
    }

    fn create_detailed_profile(dir: &Path, aid: &str) {
        let profiles_dir = dir.join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = serde_json::json!({
            "info": {"id": aid, "username": "TestPlayer"},
            "characters": {
                "pmc": {
                    "Info": {
                        "Nickname": "TestPlayer",
                        "Level": 42,
                        "Side": "Usec",
                        "Experience": 1234567,
                        "RegistrationDate": 1700000000
                    },
                    "Stats": {
                        "Eft": {
                            "OverallCounters": {
                                "Items": [
                                    {"Key": ["Sessions", "Pmc", "Survived"], "Value": 30},
                                    {"Key": ["Sessions", "Pmc", "Died"], "Value": 10},
                                    {"Key": ["KilledSavages"], "Value": 247}
                                ]
                            },
                            "Victims": [
                                {"Name": "Player1"},
                                {"Name": "Player2"}
                            ]
                        }
                    },
                    "Quests": [
                        {"qid": "quest_abc", "status": 5},
                        {"qid": "quest_def", "status": 3},
                        {"qid": "quest_ghi", "status": 1}
                    ],
                    "TradersInfo": {
                        "trader_001": {
                            "loyaltyLevel": 3,
                            "standing": 0.42,
                            "salesSum": 2450000.0
                        },
                        "trader_002": {
                            "loyaltyLevel": 1,
                            "standing": -0.05,
                            "salesSum": 50000.0
                        }
                    },
                    "Hideout": {
                        "Areas": [
                            {"type": 0, "level": 3},
                            {"type": 1, "level": 2},
                            {"type": 4, "level": 0}
                        ]
                    }
                }
            }
        });
        std::fs::write(
            profiles_dir.join(format!("{aid}.json")),
            serde_json::to_string(&content).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_profile_detail_full() {
        let tmp = tempfile::tempdir().unwrap();
        create_detailed_profile(tmp.path(), "detail1");

        let detail = load_profile_detail(tmp.path(), "detail1", &HashMap::new())
            .unwrap()
            .unwrap();
        assert_eq!(detail.stats.nickname.as_deref(), Some("TestPlayer"));
        assert_eq!(detail.stats.level, Some(42));
        assert_eq!(detail.stats.scav_kills, Some(247));
        assert_eq!(detail.stats.kill_count, Some(2));

        assert_eq!(detail.quests.len(), 3);
        assert!(matches!(
            detail
                .quests
                .iter()
                .find(|q| q.qid == "quest_abc")
                .unwrap()
                .status,
            QuestState::Success
        ));
        assert!(matches!(
            detail
                .quests
                .iter()
                .find(|q| q.qid == "quest_def")
                .unwrap()
                .status,
            QuestState::Started
        ));
        assert!(matches!(
            detail
                .quests
                .iter()
                .find(|q| q.qid == "quest_ghi")
                .unwrap()
                .status,
            QuestState::Locked
        ));

        assert_eq!(detail.traders.len(), 2);
        let t1 = detail
            .traders
            .iter()
            .find(|t| t.trader_id == "trader_001")
            .unwrap();
        assert_eq!(t1.loyalty_level, 3);
        assert!((t1.standing - 0.42).abs() < 0.001);

        assert_eq!(detail.hideout.len(), 3);
        let h0 = detail.hideout.iter().find(|h| h.area_type == 0).unwrap();
        assert_eq!(h0.level, 3);
    }

    #[test]
    fn load_profile_detail_fresh_wipe() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = serde_json::json!({
            "info": {"id": "fresh1", "username": "NewPlayer"},
            "characters": {
                "pmc": {
                    "savage": null,
                    "Encyclopedia": null,
                    "Hideout": null,
                    "WishList": []
                }
            }
        });
        std::fs::write(
            profiles_dir.join("fresh1.json"),
            serde_json::to_string(&content).unwrap(),
        )
        .unwrap();

        let result = load_profile_detail(tmp.path(), "fresh1", &HashMap::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_profile_detail_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/profiles")).unwrap();

        let result = load_profile_detail(tmp.path(), "nonexistent", &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn scav_kills_extracted_in_all_stats() {
        let tmp = tempfile::tempdir().unwrap();
        create_detailed_profile(tmp.path(), "scav_test");
        let stats = load_all_profile_stats(tmp.path());
        let s = &stats["scav_test"];
        assert_eq!(s.scav_kills, Some(247));
    }

    #[test]
    fn calculate_stash_value_basic() {
        let mut prices = HashMap::new();
        prices.insert("item_a".to_string(), 1000_i64);
        prices.insert("item_b".to_string(), 500);

        let inventory = InventoryData {
            stash: Some("stash_root".to_string()),
            items: Some(vec![
                InventoryItem {
                    id: "stash_root".to_string(),
                    tpl: "stash_container".to_string(),
                    parent_id: None,
                    upd: None,
                },
                InventoryItem {
                    id: "i1".to_string(),
                    tpl: "item_a".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: None,
                },
                InventoryItem {
                    id: "i2".to_string(),
                    tpl: "item_b".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: None,
                },
            ]),
        };

        assert_eq!(calculate_stash_value(&inventory, &prices), 1500);
    }

    #[test]
    fn calculate_stash_value_with_stacks() {
        let mut prices = HashMap::new();
        prices.insert("ammo".to_string(), 100_i64);
        prices.insert("money".to_string(), 1);

        let inventory = InventoryData {
            stash: Some("stash_root".to_string()),
            items: Some(vec![
                InventoryItem {
                    id: "stash_root".to_string(),
                    tpl: "stash_container".to_string(),
                    parent_id: None,
                    upd: None,
                },
                InventoryItem {
                    id: "i1".to_string(),
                    tpl: "ammo".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: Some(ItemUpd {
                        stack_objects_count: Some(30),
                    }),
                },
                InventoryItem {
                    id: "i2".to_string(),
                    tpl: "money".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: Some(ItemUpd {
                        stack_objects_count: Some(500000),
                    }),
                },
            ]),
        };

        // 30*100 + 500000*1 = 503000
        assert_eq!(calculate_stash_value(&inventory, &prices), 503000);
    }

    #[test]
    fn calculate_stash_value_nested_items() {
        let mut prices = HashMap::new();
        prices.insert("backpack".to_string(), 20000_i64);
        prices.insert("item_inside".to_string(), 5000);

        let inventory = InventoryData {
            stash: Some("stash_root".to_string()),
            items: Some(vec![
                InventoryItem {
                    id: "stash_root".to_string(),
                    tpl: "stash_container".to_string(),
                    parent_id: None,
                    upd: None,
                },
                InventoryItem {
                    id: "bp1".to_string(),
                    tpl: "backpack".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: None,
                },
                InventoryItem {
                    id: "nested1".to_string(),
                    tpl: "item_inside".to_string(),
                    parent_id: Some("bp1".to_string()),
                    upd: None,
                },
            ]),
        };

        // backpack + item inside it
        assert_eq!(calculate_stash_value(&inventory, &prices), 25000);
    }

    #[test]
    fn calculate_stash_value_unpriced_items_skipped() {
        let mut prices = HashMap::new();
        prices.insert("known".to_string(), 1000_i64);

        let inventory = InventoryData {
            stash: Some("stash_root".to_string()),
            items: Some(vec![
                InventoryItem {
                    id: "stash_root".to_string(),
                    tpl: "stash_container".to_string(),
                    parent_id: None,
                    upd: None,
                },
                InventoryItem {
                    id: "i1".to_string(),
                    tpl: "known".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: None,
                },
                InventoryItem {
                    id: "i2".to_string(),
                    tpl: "unknown_item".to_string(),
                    parent_id: Some("stash_root".to_string()),
                    upd: None,
                },
            ]),
        };

        assert_eq!(calculate_stash_value(&inventory, &prices), 1000);
    }

    #[test]
    fn calculate_stash_value_empty_stash() {
        let prices = HashMap::new();

        let inventory = InventoryData {
            stash: Some("stash_root".to_string()),
            items: Some(vec![InventoryItem {
                id: "stash_root".to_string(),
                tpl: "stash_container".to_string(),
                parent_id: None,
                upd: None,
            }]),
        };

        assert_eq!(calculate_stash_value(&inventory, &prices), 0);
    }

    #[test]
    fn calculate_stash_value_no_stash_id() {
        let prices = HashMap::new();
        let inventory = InventoryData {
            stash: None,
            items: Some(vec![]),
        };
        assert_eq!(calculate_stash_value(&inventory, &prices), 0);
    }
}
