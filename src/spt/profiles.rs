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
}

pub enum ProfileStatus {
    Found(SptProfileStats),
    NotFound,
    ParseError,
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
struct PmcData {
    info: Option<PmcInfo>,
    stats: Option<PmcStats>,
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
            Err(_) => continue,
        };

        let aid = match parsed.info.and_then(|i| i.id) {
            Some(id) => id,
            None => continue,
        };

        let mut stats = SptProfileStats::default();

        if let Some(pmc) = parsed.characters.and_then(|c| c.pmc) {
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
                        for item in &counters {
                            if item.key.len() >= 2
                                && item.key[0] == "Sessions"
                                && item.key[1] == "Pmc"
                            {
                                total_raids += item.value;
                                if item.key.len() == 3 && item.key[2] == "Survived" {
                                    survived = item.value;
                                }
                            }
                        }
                        if total_raids > 0 {
                            stats.raid_count = Some(total_raids);
                            stats.survival_rate = Some(
                                (survived as f64 / total_raids as f64 * 100.0 * 100.0).round()
                                    / 100.0,
                            );
                        } else {
                            stats.raid_count = Some(0);
                        }
                    }

                    stats.kill_count = eft.victims.map(|v| v.len());
                }
            }
        }

        map.insert(aid, stats);
    }

    map
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
}
