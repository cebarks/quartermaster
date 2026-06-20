use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct TraderMeta {
    pub name: String,
    pub currency: String,
}

#[derive(Debug, Clone)]
pub struct HideoutMeta {
    pub name: String,
    pub max_level: i32,
}

#[derive(Debug)]
pub struct GameData {
    quest_names: HashMap<String, String>,
    trader_info: HashMap<String, TraderMeta>,
    hideout_areas: HashMap<i32, HideoutMeta>,
    prices: HashMap<String, i64>,
}

#[derive(Deserialize)]
struct QuestJsonEntry {
    #[serde(rename = "QuestName")]
    quest_name: Option<String>,
}

#[derive(Deserialize)]
struct TraderBase {
    nickname: Option<String>,
    currency: Option<String>,
}

const CORE_TRADER_IDS: &[&str] = &[
    "54cb50c76803fa8b248b4571", // Prapor
    "54cb57776803fa99248b456e", // Therapist
    "579dc571d53a0658a154fbec", // Fence
    "58330581ace78e27b8b10cee", // Skier
    "5935c25fb3acc3127c3d8cd9", // Peacekeeper
    "5a7c2eca46aef81a7ca2145d", // Mechanic
    "5ac3b934156ae10c4430e83c", // Ragman
    "5c0647fdd443bc2504c2d371", // Jaeger
    "638f541a29ffd1183d187f57", // Lightkeeper
];

fn build_hideout_areas() -> HashMap<i32, HideoutMeta> {
    let areas: &[(i32, &str, i32)] = &[
        (0, "Vents", 3),
        (1, "Security", 3),
        (2, "Lavatory", 3),
        (3, "Stash", 4),
        (4, "Generator", 3),
        (5, "Heating", 3),
        (6, "Water Collector", 3),
        (7, "Medstation", 3),
        (8, "Nutrition Unit", 3),
        (9, "Illumination", 3),
        (10, "Workbench", 3),
        (11, "Rest Space", 3),
        (12, "Library", 2),
        (13, "Scav Case", 1),
        (14, "Intelligence Center", 3),
        (15, "Shooting Range", 1),
        (16, "Gym", 1),
        (17, "Defective Wall", 7),
        (18, "Emergency Wall", 4),
        (19, "Hall of Fame", 1),
        (20, "Bitcoin Farm", 3),
        (21, "Solar Power", 1),
        (22, "Booze Generator", 1),
        (23, "Christmas Tree", 1),
        (24, "Air Filtering Unit", 1),
        (25, "Weapon Stand", 2),
        (26, "Equipment Stand", 2),
        (27, "Culture Center", 2),
    ];
    areas
        .iter()
        .map(|(id, name, max)| {
            (
                *id,
                HideoutMeta {
                    name: name.to_string(),
                    max_level: *max,
                },
            )
        })
        .collect()
}

impl GameData {
    pub fn load(spt_dir: &Path) -> Result<Self> {
        let quest_names = Self::load_quest_names(spt_dir)?;
        let trader_info = Self::load_trader_info(spt_dir)?;
        let hideout_areas = build_hideout_areas();
        let prices = Self::load_prices(spt_dir)?;

        tracing::info!(
            quests = quest_names.len(),
            traders = trader_info.len(),
            hideout_areas = hideout_areas.len(),
            prices = prices.len(),
            "loaded SPT game data"
        );

        Ok(Self {
            quest_names,
            trader_info,
            hideout_areas,
            prices,
        })
    }

    pub fn load_empty() -> Self {
        Self {
            quest_names: HashMap::new(),
            trader_info: HashMap::new(),
            hideout_areas: build_hideout_areas(),
            prices: HashMap::new(),
        }
    }

    fn load_quest_names(spt_dir: &Path) -> Result<HashMap<String, String>> {
        let path = spt_dir.join("SPT/SPT_Data/database/templates/quests.json");
        if !path.exists() {
            tracing::warn!(
                "quests.json not found at {}, quest names will show raw IDs",
                path.display()
            );
            return Ok(HashMap::new());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let quests: HashMap<String, QuestJsonEntry> = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(quests
            .into_iter()
            .filter_map(|(id, entry)| entry.quest_name.map(|name| (id, name)))
            .collect())
    }

    fn load_trader_info(spt_dir: &Path) -> Result<HashMap<String, TraderMeta>> {
        let traders_dir = spt_dir.join("SPT/SPT_Data/database/traders");
        let mut info = HashMap::new();
        if !traders_dir.is_dir() {
            tracing::warn!("traders directory not found at {}", traders_dir.display());
            return Ok(info);
        }
        for trader_id in CORE_TRADER_IDS {
            let base_path = traders_dir.join(trader_id).join("base.json");
            if !base_path.exists() {
                continue;
            }
            let contents = match std::fs::read_to_string(&base_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(trader_id, error = %e, "failed to read trader base.json");
                    continue;
                }
            };
            let base: TraderBase = match serde_json::from_str(&contents) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(trader_id, error = %e, "failed to parse trader base.json");
                    continue;
                }
            };
            if let Some(name) = base.nickname {
                info.insert(
                    trader_id.to_string(),
                    TraderMeta {
                        name,
                        currency: base.currency.unwrap_or_else(|| "RUB".to_string()),
                    },
                );
            }
        }
        Ok(info)
    }

    fn load_prices(spt_dir: &Path) -> Result<HashMap<String, i64>> {
        let path = spt_dir.join("SPT/SPT_Data/database/templates/prices.json");
        if !path.exists() {
            tracing::warn!(
                "prices.json not found at {}, stash values will be unavailable",
                path.display()
            );
            return Ok(HashMap::new());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let prices: HashMap<String, i64> = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(prices)
    }

    pub fn quest_name<'a>(&'a self, qid: &'a str) -> &'a str {
        self.quest_names.get(qid).map(|s| s.as_str()).unwrap_or(qid)
    }

    pub fn trader_meta(&self, trader_id: &str) -> Option<&TraderMeta> {
        self.trader_info.get(trader_id)
    }

    pub fn hideout_area(&self, area_type: i32) -> Option<&HideoutMeta> {
        self.hideout_areas.get(&area_type)
    }

    pub fn item_price(&self, tpl: &str) -> Option<i64> {
        self.prices.get(tpl).copied()
    }

    pub fn prices(&self) -> &HashMap<String, i64> {
        &self.prices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_spt_dir(dir: &Path) {
        let quests_path = dir.join("SPT/SPT_Data/database/templates/quests.json");
        std::fs::create_dir_all(quests_path.parent().unwrap()).unwrap();
        let quests = serde_json::json!({
            "5936d90786f7742b1420ba5b": {
                "QuestName": "Debut",
                "_id": "5936d90786f7742b1420ba5b"
            },
            "5936da9e86f7742d65037edf": {
                "QuestName": "Background Check",
                "_id": "5936da9e86f7742d65037edf"
            }
        });
        std::fs::write(&quests_path, serde_json::to_string(&quests).unwrap()).unwrap();

        let trader_id = "54cb50c76803fa8b248b4571";
        let trader_dir = dir.join(format!("SPT/SPT_Data/database/traders/{trader_id}"));
        std::fs::create_dir_all(&trader_dir).unwrap();
        let base = serde_json::json!({
            "nickname": "Prapor",
            "currency": "RUB",
            "_id": trader_id
        });
        std::fs::write(
            trader_dir.join("base.json"),
            serde_json::to_string(&base).unwrap(),
        )
        .unwrap();

        let pk_id = "5935c25fb3acc3127c3d8cd9";
        let pk_dir = dir.join(format!("SPT/SPT_Data/database/traders/{pk_id}"));
        std::fs::create_dir_all(&pk_dir).unwrap();
        let pk_base = serde_json::json!({
            "nickname": "Peacekeeper",
            "currency": "USD",
            "_id": pk_id
        });
        std::fs::write(
            pk_dir.join("base.json"),
            serde_json::to_string(&pk_base).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn loads_quest_names() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        let gd = GameData::load(tmp.path()).unwrap();
        assert_eq!(gd.quest_name("5936d90786f7742b1420ba5b"), "Debut");
        assert_eq!(
            gd.quest_name("5936da9e86f7742d65037edf"),
            "Background Check"
        );
    }

    #[test]
    fn unknown_quest_returns_raw_id() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        let gd = GameData::load(tmp.path()).unwrap();
        assert_eq!(gd.quest_name("unknown_id"), "unknown_id");
    }

    #[test]
    fn loads_trader_info() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        let gd = GameData::load(tmp.path()).unwrap();
        let prapor = gd.trader_meta("54cb50c76803fa8b248b4571").unwrap();
        assert_eq!(prapor.name, "Prapor");
        assert_eq!(prapor.currency, "RUB");
        let pk = gd.trader_meta("5935c25fb3acc3127c3d8cd9").unwrap();
        assert_eq!(pk.name, "Peacekeeper");
        assert_eq!(pk.currency, "USD");
    }

    #[test]
    fn unknown_trader_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        let gd = GameData::load(tmp.path()).unwrap();
        assert!(gd.trader_meta("nonexistent").is_none());
    }

    #[test]
    fn hideout_area_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        let gd = GameData::load(tmp.path()).unwrap();
        let vents = gd.hideout_area(0).unwrap();
        assert_eq!(vents.name, "Vents");
        assert!(vents.max_level > 0);
    }

    #[test]
    fn unknown_hideout_area_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        let gd = GameData::load(tmp.path()).unwrap();
        assert!(gd.hideout_area(999).is_none());
    }

    #[test]
    fn loads_prices() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        // create_test_spt_dir doesn't create prices.json yet, so add it
        let prices_path = tmp
            .path()
            .join("SPT/SPT_Data/database/templates/prices.json");
        let prices = serde_json::json!({
            "5447a9cd4bdc2dbd208b4567": 132725,
            "5449016a4bdc2d6f028b456f": 1
        });
        std::fs::write(&prices_path, serde_json::to_string(&prices).unwrap()).unwrap();

        let gd = GameData::load(tmp.path()).unwrap();
        assert_eq!(gd.item_price("5447a9cd4bdc2dbd208b4567"), Some(132725));
        assert_eq!(gd.item_price("5449016a4bdc2d6f028b456f"), Some(1));
        assert_eq!(gd.item_price("nonexistent"), None);
        assert_eq!(gd.prices().len(), 2);
    }

    #[test]
    fn prices_missing_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_spt_dir(tmp.path());
        // No prices.json created — should still load fine with empty prices
        let gd = GameData::load(tmp.path()).unwrap();
        assert!(gd.prices().is_empty());
    }
}
