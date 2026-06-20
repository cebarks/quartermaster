use anyhow::Result;
use serde::Serialize;

use crate::cli::common::CliContext;
use crate::server_detect::resolve_server_addr;
use crate::spt::mods::{compute_file_hash, scan_mod_directories};
use crate::spt::server::SptClient;

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub server: ServerHealth,
    pub mods: ModsHealth,
    pub integrity: IntegrityHealth,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerHealth {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub version: Option<String>,
    pub version_matches: Option<bool>,
    pub address: String,
    pub error: Option<String>,
    pub transition: Option<String>,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModsHealth {
    pub installed_count: usize,
    pub loaded_count: Option<usize>,
    pub load_failures: Vec<String>,
    pub untracked_loaded: Vec<String>,
    pub updates_available: usize,
    pub incompatible_mods: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrityHealth {
    pub tracked_files: usize,
    pub missing_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub untracked_dirs: Vec<UntrackedDir>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UntrackedDir {
    pub path: String,
    pub file_count: usize,
}

impl HealthReport {
    /// Exit code per spec:
    /// - 0: all checks pass
    /// - 1: server is down or unreachable
    /// - 2: mod issues (incompatible mods, missing files, modified files)
    pub fn exit_code(&self) -> i32 {
        if !self.server.reachable {
            return 1;
        }
        if !self.mods.incompatible_mods.is_empty()
            || !self.mods.load_failures.is_empty()
            || !self.integrity.missing_files.is_empty()
            || !self.integrity.modified_files.is_empty()
        {
            return 2;
        }
        0
    }
}

pub async fn run_checks(ctx: &CliContext) -> Result<HealthReport> {
    let (host, port) = resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    let server = check_server(&spt_client, &ctx.spt_info.spt_version, &address).await;

    let loaded_mods = if server.reachable {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let installed_mods = ctx.db.list_mods()?;
    let mods = check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &ctx.forge,
        &ctx.spt_info.spt_version,
    )
    .await;

    let tracked_files = ctx.db.get_all_tracked_files()?;
    let integrity = check_integrity_from(&tracked_files, &ctx.spt_dir)?;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}

pub async fn check_server(
    spt_client: &SptClient,
    expected_version: &str,
    address: &str,
) -> ServerHealth {
    let ping = spt_client.ping().await;

    let (reachable, latency_ms, error) = match &ping {
        Ok(p) if p.ok => (true, Some(p.latency_ms), None),
        Ok(_) => (false, None, Some("server returned error".to_string())),
        Err(e) => (false, None, Some(format!("{e:#}"))),
    };

    if !reachable {
        return ServerHealth {
            reachable,
            latency_ms,
            version: None,
            version_matches: None,
            address: address.to_string(),
            error,
            transition: None,
            started_at: None,
        };
    }

    let version = spt_client.server_version().await.ok();
    let version_matches = version.as_deref().map(|v| v == expected_version);

    ServerHealth {
        reachable,
        latency_ms,
        version,
        version_matches,
        address: address.to_string(),
        error: None,
        transition: None,
        started_at: None,
    }
}

pub async fn check_mods_health(
    installed_mods: &[crate::db::mods::InstalledMod],
    loaded_mods: Option<&std::collections::HashMap<String, serde_json::Value>>,
    forge: &crate::forge::client::ForgeClient,
    spt_version: &str,
) -> ModsHealth {
    let mut updates_available = 0;
    let mut incompatible_mods = Vec::new();

    if !installed_mods.is_empty() {
        let check_list: Vec<(i64, String)> = installed_mods
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();

        if let Ok(results) = forge.check_updates(&check_list, spt_version).await {
            updates_available = results.updates.len();

            for m in &results.incompatible_with_spt {
                incompatible_mods.push(m.name.clone());
            }
        }
    }

    let (loaded_count, load_failures, untracked_loaded) = match loaded_mods {
        Some(loaded) => {
            let (failures, untracked) = check_mod_loads(installed_mods, loaded);
            (Some(loaded.len()), failures, untracked)
        }
        None => (None, vec![], vec![]),
    };

    ModsHealth {
        installed_count: installed_mods.len(),
        loaded_count,
        load_failures,
        untracked_loaded,
        updates_available,
        incompatible_mods,
    }
}

pub fn check_integrity_from(
    tracked_files: &[crate::db::mods::InstalledFile],
    spt_dir: &std::path::Path,
) -> Result<IntegrityHealth> {
    let mut missing_files = Vec::new();
    let mut modified_files = Vec::new();

    for file in tracked_files {
        let full_path = spt_dir.join(&file.file_path);
        if !full_path.exists() {
            missing_files.push(file.file_path.clone());
            continue;
        }

        if let Some(ref expected_hash) = file.file_hash {
            match compute_file_hash(&full_path) {
                Ok(actual_hash) => {
                    if actual_hash != *expected_hash {
                        modified_files.push(file.file_path.clone());
                    }
                }
                Err(_) => {
                    modified_files.push(file.file_path.clone());
                }
            }
        }
    }

    let all_disk_files = scan_mod_directories(spt_dir)?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let untracked: Vec<&str> = all_disk_files
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let mut dir_counts = crate::cli::common::group_untracked_by_mod_dir(&untracked);
    dir_counts.remove("BepInEx/plugins/spt");

    let untracked_dirs: Vec<UntrackedDir> = dir_counts
        .into_iter()
        .map(|(path, file_count)| UntrackedDir { path, file_count })
        .collect();

    Ok(IntegrityHealth {
        tracked_files: tracked_files.len(),
        missing_files,
        modified_files,
        untracked_dirs,
    })
}

/// Compare installed mods (from DB) against loaded mods (from SPT server).
/// Uses case-insensitive name matching because the SPT server may report
/// mod names using the package.json `name` field which can differ in casing
/// from the Forge display name stored in the DB.
pub fn check_mod_loads(
    installed_mods: &[crate::db::mods::InstalledMod],
    loaded_mods: &std::collections::HashMap<String, serde_json::Value>,
) -> (Vec<String>, Vec<String>) {
    let installed_lower: std::collections::HashSet<String> = installed_mods
        .iter()
        .map(|m| m.name.to_lowercase())
        .collect();

    let loaded_lower: std::collections::HashSet<String> =
        loaded_mods.keys().map(|k| k.to_lowercase()).collect();

    let load_failures: Vec<String> = installed_mods
        .iter()
        .filter(|m| !loaded_lower.contains(&m.name.to_lowercase()))
        .map(|m| m.name.clone())
        .collect();

    let untracked_loaded: Vec<String> = loaded_mods
        .keys()
        .filter(|name| !installed_lower.contains(&name.to_lowercase()))
        .cloned()
        .collect();

    (load_failures, untracked_loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::mods::InstalledMod;

    fn good_server() -> ServerHealth {
        ServerHealth {
            reachable: true,
            latency_ms: Some(12),
            version: Some("4.0.13".to_string()),
            version_matches: Some(true),
            address: "https://127.0.0.1:6969".to_string(),
            error: None,
            transition: None,
            started_at: None,
        }
    }

    fn good_mods() -> ModsHealth {
        ModsHealth {
            installed_count: 5,
            loaded_count: Some(5),
            load_failures: vec![],
            untracked_loaded: vec![],
            updates_available: 0,
            incompatible_mods: vec![],
        }
    }

    fn good_integrity() -> IntegrityHealth {
        IntegrityHealth {
            tracked_files: 100,
            missing_files: vec![],
            modified_files: vec![],
            untracked_dirs: vec![],
        }
    }

    #[test]
    fn server_health_with_transition() {
        let health = ServerHealth {
            reachable: false,
            latency_ms: None,
            version: None,
            version_matches: None,
            address: "https://127.0.0.1:6969".to_string(),
            error: Some("connection refused".to_string()),
            transition: Some("restarting".to_string()),
            started_at: None,
        };
        assert_eq!(health.transition.as_deref(), Some("restarting"));
        assert!(!health.reachable);
    }

    #[test]
    fn server_health_without_transition() {
        let health = ServerHealth {
            reachable: true,
            latency_ms: Some(10),
            version: Some("4.0.13".to_string()),
            version_matches: Some(true),
            address: "https://127.0.0.1:6969".to_string(),
            error: None,
            transition: None,
            started_at: Some("2026-06-19T10:00:00Z".to_string()),
        };
        assert!(health.transition.is_none());
        assert!(health.started_at.is_some());
    }

    #[test]
    fn check_mod_loads_all_matching() {
        let installed = vec![
            InstalledMod {
                id: 1,
                forge_mod_id: 100,
                forge_version_id: 200,
                name: "ModA".to_string(),
                slug: None,
                version: "1.0.0".to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: None,
                disabled: false,
            },
            InstalledMod {
                id: 2,
                forge_mod_id: 101,
                forge_version_id: 201,
                name: "ModB".to_string(),
                slug: None,
                version: "2.0.0".to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: None,
                disabled: false,
            },
        ];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("ModA".to_string(), serde_json::json!({}));
        loaded.insert("ModB".to_string(), serde_json::json!({}));

        let (failures, untracked) = check_mod_loads(&installed, &loaded);
        assert!(failures.is_empty());
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_detects_load_failure() {
        let installed = vec![
            InstalledMod {
                id: 1,
                forge_mod_id: 100,
                forge_version_id: 200,
                name: "WorkingMod".to_string(),
                slug: None,
                version: "1.0.0".to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: None,
                disabled: false,
            },
            InstalledMod {
                id: 2,
                forge_mod_id: 101,
                forge_version_id: 201,
                name: "BrokenMod".to_string(),
                slug: None,
                version: "2.0.0".to_string(),
                installed_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: None,
                disabled: false,
            },
        ];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("WorkingMod".to_string(), serde_json::json!({}));

        let (failures, untracked) = check_mod_loads(&installed, &loaded);
        assert_eq!(failures, vec!["BrokenMod"]);
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_detects_untracked() {
        let installed = vec![InstalledMod {
            id: 1,
            forge_mod_id: 100,
            forge_version_id: 200,
            name: "TrackedMod".to_string(),
            slug: None,
            version: "1.0.0".to_string(),
            installed_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            disabled: false,
        }];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("TrackedMod".to_string(), serde_json::json!({}));
        loaded.insert("ManualMod".to_string(), serde_json::json!({}));

        let (failures, untracked) = check_mod_loads(&installed, &loaded);
        assert!(failures.is_empty());
        assert_eq!(untracked, vec!["ManualMod"]);
    }

    #[test]
    fn check_mod_loads_case_insensitive() {
        let installed = vec![InstalledMod {
            id: 1,
            forge_mod_id: 100,
            forge_version_id: 200,
            name: "SAIN".to_string(),
            slug: None,
            version: "1.0.0".to_string(),
            installed_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            disabled: false,
        }];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("sain".to_string(), serde_json::json!({}));

        let (failures, untracked) = check_mod_loads(&installed, &loaded);
        assert!(
            failures.is_empty(),
            "case-insensitive match should not report failure"
        );
        assert!(
            untracked.is_empty(),
            "case-insensitive match should not report untracked"
        );
    }

    #[test]
    fn exit_code_all_good() {
        let report = HealthReport {
            server: good_server(),
            mods: good_mods(),
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn exit_code_server_down() {
        let report = HealthReport {
            server: ServerHealth {
                reachable: false,
                latency_ms: None,
                version: None,
                version_matches: None,
                address: "https://127.0.0.1:6969".to_string(),
                error: Some("connection refused".to_string()),
                transition: None,
                started_at: None,
            },
            mods: good_mods(),
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn exit_code_incompatible_mods() {
        let report = HealthReport {
            server: good_server(),
            mods: ModsHealth {
                installed_count: 5,
                loaded_count: None,
                load_failures: vec![],
                untracked_loaded: vec![],
                updates_available: 0,
                incompatible_mods: vec!["OldMod".to_string()],
            },
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_missing_files() {
        let report = HealthReport {
            server: good_server(),
            mods: good_mods(),
            integrity: IntegrityHealth {
                tracked_files: 50,
                missing_files: vec!["user/mods/Gone/file.dll".to_string()],
                modified_files: vec![],
                untracked_dirs: vec![],
            },
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_modified_files() {
        let report = HealthReport {
            server: good_server(),
            mods: good_mods(),
            integrity: IntegrityHealth {
                tracked_files: 50,
                missing_files: vec![],
                modified_files: vec!["user/mods/X/x.dll".to_string()],
                untracked_dirs: vec![],
            },
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_load_failures() {
        let report = HealthReport {
            server: good_server(),
            mods: ModsHealth {
                installed_count: 5,
                loaded_count: Some(4),
                load_failures: vec!["BrokenMod".to_string()],
                untracked_loaded: vec![],
                updates_available: 0,
                incompatible_mods: vec![],
            },
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_server_down_trumps_mod_issues() {
        let report = HealthReport {
            server: ServerHealth {
                reachable: false,
                latency_ms: None,
                version: None,
                version_matches: None,
                address: "https://127.0.0.1:6969".to_string(),
                error: Some("timeout".to_string()),
                transition: None,
                started_at: None,
            },
            mods: ModsHealth {
                installed_count: 5,
                loaded_count: None,
                load_failures: vec![],
                untracked_loaded: vec![],
                updates_available: 0,
                incompatible_mods: vec!["X".to_string()],
            },
            integrity: IntegrityHealth {
                tracked_files: 50,
                missing_files: vec!["a.dll".to_string()],
                modified_files: vec![],
                untracked_dirs: vec![],
            },
        };
        assert_eq!(
            report.exit_code(),
            1,
            "server down (1) should take precedence over mod issues (2)"
        );
    }

    #[test]
    fn check_integrity_detects_missing_file() {
        use crate::cli::common::CliContext;
        use crate::config::Config;
        use crate::db::Database;
        use crate::forge::client::ForgeClient;
        use crate::spt::detect::SptInfo;

        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/test.dll",
            Some("abc123"),
            Some(100),
        )
        .unwrap();

        let ctx = CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            db,
            forge: ForgeClient::new(None).unwrap(),
            container_mgr: None,
        };

        let result = {
            let tracked = ctx.db.get_all_tracked_files().unwrap();
            check_integrity_from(&tracked, &ctx.spt_dir).unwrap()
        };
        assert_eq!(result.tracked_files, 1);
        assert_eq!(result.missing_files, vec!["SPT/user/mods/TestMod/test.dll"]);
        assert!(result.modified_files.is_empty());
    }

    #[test]
    fn check_integrity_detects_modified_file() {
        use crate::cli::common::CliContext;
        use crate::config::Config;
        use crate::db::Database;
        use crate::forge::client::ForgeClient;
        use crate::spt::detect::SptInfo;
        use crate::spt::mods::compute_file_hash;

        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/TestMod")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let file_path = spt_dir.join("SPT/user/mods/TestMod/test.dll");
        std::fs::write(&file_path, b"original content").unwrap();
        let original_hash = compute_file_hash(&file_path).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/test.dll",
            Some(&original_hash),
            Some(16),
        )
        .unwrap();

        // Tamper with the file after recording
        std::fs::write(&file_path, b"tampered content").unwrap();

        let ctx = CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            db,
            forge: ForgeClient::new(None).unwrap(),
            container_mgr: None,
        };

        let result = {
            let tracked = ctx.db.get_all_tracked_files().unwrap();
            check_integrity_from(&tracked, &ctx.spt_dir).unwrap()
        };
        assert!(result.missing_files.is_empty());
        assert_eq!(
            result.modified_files,
            vec!["SPT/user/mods/TestMod/test.dll"]
        );
    }

    #[test]
    fn check_integrity_detects_untracked_files() {
        use crate::cli::common::CliContext;
        use crate::config::Config;
        use crate::db::Database;
        use crate::forge::client::ForgeClient;
        use crate::spt::detect::SptInfo;

        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/UnknownMod")).unwrap();
        std::fs::write(spt_dir.join("SPT/user/mods/UnknownMod/mod.dll"), b"x").unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();

        let ctx = CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            db,
            forge: ForgeClient::new(None).unwrap(),
            container_mgr: None,
        };

        let result = {
            let tracked = ctx.db.get_all_tracked_files().unwrap();
            check_integrity_from(&tracked, &ctx.spt_dir).unwrap()
        };
        assert_eq!(result.tracked_files, 0);
        assert_eq!(result.untracked_dirs.len(), 1);
        assert_eq!(result.untracked_dirs[0].path, "SPT/user/mods/UnknownMod");
        assert_eq!(result.untracked_dirs[0].file_count, 1);
    }
}
