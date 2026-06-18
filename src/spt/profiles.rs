use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SptProfile {
    pub aid: String,
    pub username: String,
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
}
