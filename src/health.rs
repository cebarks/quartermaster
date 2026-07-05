use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityHealth {
    pub tracked_files: usize,
    pub missing_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub untracked_dirs: Vec<UntrackedDir>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let server_mod_ids = ctx.db.mods_with_server_files()?;
    let spt_names = resolve_spt_names(&ctx.db, &server_mod_ids);
    let mods = check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &ctx.forge,
        &ctx.spt_info.spt_version,
        &server_mod_ids,
        &spt_names,
    )
    .await;

    let integrity = match try_fetch_cached_integrity(&ctx.config).await {
        Some(cached) => cached,
        None => {
            let tracked_files = ctx.db.get_all_enabled_mod_files()?;
            let spt_dir = ctx.spt_dir.clone();
            tokio::task::spawn_blocking(move || {
                let progress = std::sync::atomic::AtomicUsize::new(0);
                let total = std::sync::atomic::AtomicUsize::new(0);
                check_integrity_parallel(&tracked_files, &spt_dir, &progress, &total)
            })
            .await
            .map_err(|e| anyhow::anyhow!("integrity check task failed: {e}"))??
        }
    };

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}

async fn try_fetch_cached_integrity(config: &crate::config::Config) -> Option<IntegrityHealth> {
    let port = config.web_port;
    let url = format!("http://127.0.0.1:{port}/quma/health/integrity");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<IntegrityHealth>().await.ok()
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

/// Extract the mod directory name from each mod's server file paths.
/// Returns a map of mod DB id → directory name under `user/mods/`.
///
/// The SPT server keys `loadedServerMods` by the directory name (the folder
/// name under `user/mods/`), NOT by the `package.json` `name` field. For
/// example, Fika Server lives in `fika-server/` but its `package.json` has
/// `"name": "server"`.
pub fn resolve_spt_names(
    db: &crate::db::Database,
    server_mod_ids: &std::collections::HashSet<i64>,
) -> std::collections::HashMap<i64, String> {
    let mut names = std::collections::HashMap::new();
    for &mod_id in server_mod_ids {
        if let Ok(files) = db.get_files_for_mod(mod_id) {
            for file in &files {
                if file.file_path.starts_with("SPT/user/mods/") {
                    let parts: Vec<&str> = file.file_path.splitn(5, '/').collect();
                    if parts.len() >= 4 {
                        names.insert(mod_id, parts[3].to_string());
                        break;
                    }
                }
            }
        }
    }
    names
}

pub async fn check_mods_health(
    installed_mods: &[crate::db::mods::InstalledMod],
    loaded_mods: Option<&std::collections::HashMap<String, serde_json::Value>>,
    forge: &crate::forge::client::ForgeClient,
    spt_version: &str,
    server_mod_ids: &std::collections::HashSet<i64>,
    spt_names: &std::collections::HashMap<i64, String>,
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
            let (failures, untracked) =
                check_mod_loads(installed_mods, loaded, server_mod_ids, spt_names);
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

/// Parallel integrity check using rayon for concurrent file hashing.
/// Updates `progress` and `total` atomics for progress reporting.
// ponytail: rayon parallel hash + TTL cache; upgrade to inotify (notify crate) per-file watching when file counts justify it
pub fn check_integrity_parallel(
    tracked_files: &[crate::db::mods::InstalledFile],
    spt_dir: &std::path::Path,
    progress: &AtomicUsize,
    total: &AtomicUsize,
) -> Result<IntegrityHealth> {
    total.store(tracked_files.len(), Ordering::Relaxed);
    progress.store(0, Ordering::Relaxed);

    let results: Vec<(Option<String>, Option<String>)> = tracked_files
        .par_iter()
        .map(|file| {
            let full_path = spt_dir.join(&file.file_path);
            let result = if !full_path.exists() {
                (Some(file.file_path.clone()), None)
            } else if let Some(ref expected_hash) = file.file_hash {
                match compute_file_hash(&full_path) {
                    Ok(actual_hash) if actual_hash != *expected_hash => {
                        (None, Some(file.file_path.clone()))
                    }
                    Err(_) => (None, Some(file.file_path.clone())),
                    _ => (None, None),
                }
            } else {
                (None, None)
            };
            progress.fetch_add(1, Ordering::Relaxed);
            result
        })
        .collect();

    let mut missing_files = Vec::new();
    let mut modified_files = Vec::new();
    for (missing, modified) in results {
        if let Some(path) = missing {
            missing_files.push(path);
        }
        if let Some(path) = modified {
            modified_files.push(path);
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

/// Strip non-alphanumeric characters and lowercase for fuzzy name comparison.
///
/// SPT keys `loadedServerMods` by the mod's self-reported internal name
/// (from DLL metadata), which differs from both the directory name and the
/// Forge display name. Normalizing lets us match via substring containment
/// (e.g. directory `acidphantasm-bosseshavegpcoins` contains normalized
/// loaded name `bosseshavegpcoins`).
fn normalize_mod_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Check if two mod names match after normalization, using either exact
/// equality or substring containment (in either direction).
fn names_match(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.contains(b) || b.contains(a)
}

/// Compare installed mods (from DB) against loaded mods (from SPT server).
///
/// SPT keys `loadedServerMods` by the mod's self-reported internal name
/// (from DLL metadata attributes like `[SptMod("name")]`), which can be
/// completely different from both the directory name under `user/mods/`
/// and the Forge display name. For example:
///   - directory `acidphantasm-bosseshavegpcoins` → loaded as `Bosses Have GP Coins`
///   - directory `fika-server` → loaded as `server`
///   - directory `Solarint-SAIN-ServerMod` → loaded as `SAIN`
///
/// We match using normalized substring containment: strip non-alphanumeric
/// chars and lowercase, then check if either name contains the other. This
/// works because directory names typically embed the mod name (with author
/// prefixes/suffixes). Both the directory name and Forge display name are
/// tried for maximum coverage.
pub fn check_mod_loads(
    installed_mods: &[crate::db::mods::InstalledMod],
    loaded_mods: &std::collections::HashMap<String, serde_json::Value>,
    server_mod_ids: &std::collections::HashSet<i64>,
    spt_names: &std::collections::HashMap<i64, String>,
) -> (Vec<String>, Vec<String>) {
    let checkable: Vec<&crate::db::mods::InstalledMod> = installed_mods
        .iter()
        .filter(|m| !m.disabled && server_mod_ids.contains(&m.id))
        .collect();

    let loaded_normalized: Vec<(&String, String)> = loaded_mods
        .keys()
        .map(|k| (k, normalize_mod_name(k)))
        .collect();

    let installed_names: Vec<(&crate::db::mods::InstalledMod, Vec<String>)> = checkable
        .iter()
        .map(|m| {
            let mut candidates = vec![normalize_mod_name(&m.name)];
            if let Some(dir_name) = spt_names.get(&m.id) {
                candidates.push(normalize_mod_name(dir_name));
            }
            (*m, candidates)
        })
        .collect();

    let load_failures: Vec<String> = installed_names
        .iter()
        .filter(|(_, candidates)| {
            !loaded_normalized
                .iter()
                .any(|(_, ln)| candidates.iter().any(|c| names_match(c, ln)))
        })
        .map(|(m, _)| m.name.clone())
        .collect();

    let untracked_loaded: Vec<String> = loaded_normalized
        .iter()
        .filter(|(_, ln)| {
            !installed_names
                .iter()
                .any(|(_, candidates)| candidates.iter().any(|c| names_match(c, ln)))
        })
        .map(|(name, _)| (*name).clone())
        .collect();

    (load_failures, untracked_loaded)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::db::mods::InstalledMod;

    /// Create a test InstalledMod with sensible defaults. Only the fields that
    /// vary across tests need to be specified; use struct update syntax for
    /// overrides like `disabled: true`.
    fn test_mod(id: i64, forge_mod_id: i64, name: &str) -> InstalledMod {
        InstalledMod {
            id,
            forge_mod_id,
            forge_version_id: forge_mod_id + 100,
            name: name.to_string(),
            slug: None,
            version: "1.0.0".to_string(),
            installed_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            disabled: false,
        }
    }

    /// Build a CliContext pointing at a temp SPT directory with standard
    /// subdirectories already created. Caller still owns the `TempDir`.
    fn test_cli_context(spt_dir: &std::path::Path) -> crate::cli::common::CliContext {
        crate::cli::common::CliContext {
            spt_dir: spt_dir.to_path_buf(),
            spt_info: crate::spt::detect::SptInfo {
                root: spt_dir.to_path_buf(),
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: crate::config::Config::default(),
            db: crate::db::Database::open_in_memory().unwrap(),
            forge: crate::forge::client::ForgeClient::new(None).unwrap(),
            container_mgr: None,
        }
    }

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
            test_mod(1, 100, "ModA"),
            InstalledMod {
                version: "2.0.0".to_string(),
                ..test_mod(2, 101, "ModB")
            },
        ];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("ModA".to_string(), serde_json::json!({}));
        loaded.insert("ModB".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1, 2].into_iter().collect();
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
        assert!(failures.is_empty());
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_detects_load_failure() {
        let installed = vec![
            test_mod(1, 100, "WorkingMod"),
            InstalledMod {
                version: "2.0.0".to_string(),
                ..test_mod(2, 101, "BrokenMod")
            },
        ];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("WorkingMod".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1, 2].into_iter().collect();
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
        assert_eq!(failures, vec!["BrokenMod"]);
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_detects_untracked() {
        let installed = vec![test_mod(1, 100, "TrackedMod")];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("TrackedMod".to_string(), serde_json::json!({}));
        loaded.insert("ManualMod".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
        assert!(failures.is_empty());
        assert_eq!(untracked, vec!["ManualMod"]);
    }

    #[test]
    fn check_mod_loads_case_insensitive() {
        let installed = vec![test_mod(1, 100, "SAIN")];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("sain".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
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
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();

        let ctx = test_cli_context(tmp.path());
        let mod_id = ctx
            .db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();
        ctx.db
            .insert_file(
                mod_id,
                "SPT/user/mods/TestMod/test.dll",
                Some("abc123"),
                Some(100),
            )
            .unwrap();

        let result = {
            let tracked = ctx.db.get_all_tracked_files().unwrap();
            let progress = AtomicUsize::new(0);
            let total = AtomicUsize::new(0);
            check_integrity_parallel(&tracked, &ctx.spt_dir, &progress, &total).unwrap()
        };
        assert_eq!(result.tracked_files, 1);
        assert_eq!(result.missing_files, vec!["SPT/user/mods/TestMod/test.dll"]);
        assert!(result.modified_files.is_empty());
    }

    #[test]
    fn check_integrity_detects_modified_file() {
        use crate::spt::mods::compute_file_hash;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();

        let file_path = tmp.path().join("SPT/user/mods/TestMod/test.dll");
        std::fs::write(&file_path, b"original content").unwrap();
        let original_hash = compute_file_hash(&file_path).unwrap();

        let ctx = test_cli_context(tmp.path());
        let mod_id = ctx
            .db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();
        ctx.db
            .insert_file(
                mod_id,
                "SPT/user/mods/TestMod/test.dll",
                Some(&original_hash),
                Some(16),
            )
            .unwrap();

        // Tamper with the file after recording
        std::fs::write(&file_path, b"tampered content").unwrap();

        let result = {
            let tracked = ctx.db.get_all_tracked_files().unwrap();
            let progress = AtomicUsize::new(0);
            let total = AtomicUsize::new(0);
            check_integrity_parallel(&tracked, &ctx.spt_dir, &progress, &total).unwrap()
        };
        assert!(result.missing_files.is_empty());
        assert_eq!(
            result.modified_files,
            vec!["SPT/user/mods/TestMod/test.dll"]
        );
    }

    #[test]
    fn check_integrity_detects_untracked_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/UnknownMod")).unwrap();
        std::fs::write(tmp.path().join("SPT/user/mods/UnknownMod/mod.dll"), b"x").unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();

        let ctx = test_cli_context(tmp.path());

        let result = {
            let tracked = ctx.db.get_all_tracked_files().unwrap();
            let progress = AtomicUsize::new(0);
            let total = AtomicUsize::new(0);
            check_integrity_parallel(&tracked, &ctx.spt_dir, &progress, &total).unwrap()
        };
        assert_eq!(result.tracked_files, 0);
        assert_eq!(result.untracked_dirs.len(), 1);
        assert_eq!(result.untracked_dirs[0].path, "SPT/user/mods/UnknownMod");
        assert_eq!(result.untracked_dirs[0].file_count, 1);
    }

    #[test]
    fn check_mod_loads_excludes_disabled_mods() {
        let installed = vec![
            test_mod(1, 100, "EnabledMod"),
            InstalledMod {
                disabled: true,
                ..test_mod(2, 101, "DisabledMod")
            },
        ];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("EnabledMod".to_string(), serde_json::json!({}));

        // Both mods have server files — test is specifically about the disabled filter
        let server_mod_ids: std::collections::HashSet<i64> = [1, 2].into_iter().collect();
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
        assert!(
            failures.is_empty(),
            "disabled mod should not be reported as load failure, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_excludes_client_only_mods() {
        let installed = vec![
            test_mod(1, 100, "ServerMod"),
            test_mod(2, 101, "ClientOnlyMod"),
        ];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("ServerMod".to_string(), serde_json::json!({}));

        // ClientOnlyMod has no server files — its ID is NOT in the server_mod_ids set
        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
        assert!(
            failures.is_empty(),
            "client-only mod should not be reported as load failure, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_normalized_forge_name_match() {
        let installed = vec![test_mod(1, 100, "Looting Bots")];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("LootingBots".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();

        // Normalized matching: "Looting Bots" → "lootingbots" matches "LootingBots" → "lootingbots"
        let (failures, untracked) = check_mod_loads(
            &installed,
            &loaded,
            &server_mod_ids,
            &std::collections::HashMap::new(),
        );
        assert!(
            failures.is_empty(),
            "normalized Forge name should match loaded name, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_author_prefixed_directory() {
        // Real-world: directory "acidphantasm-bosseshavegpcoins" loads as "Bosses Have GP Coins"
        let installed = vec![test_mod(1, 100, "Bosses Have Gp Coins")];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("Bosses Have GP Coins".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let mut spt_names = std::collections::HashMap::new();
        spt_names.insert(1_i64, "acidphantasm-bosseshavegpcoins".to_string());

        let (failures, untracked) =
            check_mod_loads(&installed, &loaded, &server_mod_ids, &spt_names);
        assert!(
            failures.is_empty(),
            "author-prefixed directory should match via containment, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_fika_server_matches() {
        // Real-world: directory "fika-server" loads as "server"
        let installed = vec![InstalledMod {
            version: "2.3.2".to_string(),
            ..test_mod(1, 100, "Project Fika - Server")
        }];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("server".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let mut spt_names = std::collections::HashMap::new();
        spt_names.insert(1_i64, "fika-server".to_string());

        let (failures, untracked) =
            check_mod_loads(&installed, &loaded, &server_mod_ids, &spt_names);
        assert!(
            failures.is_empty(),
            "fika-server directory should match 'server' via containment, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_sain_matches_via_containment() {
        // Real-world: directory "Solarint-SAIN-ServerMod" loads as "SAIN"
        let installed = vec![InstalledMod {
            version: "4.4.3".to_string(),
            ..test_mod(1, 100, "SAIN - Solarint's AI Modifications")
        }];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("SAIN".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let mut spt_names = std::collections::HashMap::new();
        spt_names.insert(1_i64, "Solarint-SAIN-ServerMod".to_string());

        let (failures, untracked) =
            check_mod_loads(&installed, &loaded, &server_mod_ids, &spt_names);
        assert!(
            failures.is_empty(),
            "SAIN should match via containment in directory name, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_dotted_directory_name() {
        // Real-world: directory "Tyfon.UIFixes.Server" loads as "UI Fixes"
        let installed = vec![InstalledMod {
            version: "5.3.9".to_string(),
            ..test_mod(1, 100, "UI Fixes")
        }];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("UI Fixes".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let mut spt_names = std::collections::HashMap::new();
        spt_names.insert(1_i64, "Tyfon.UIFixes.Server".to_string());

        let (failures, untracked) =
            check_mod_loads(&installed, &loaded, &server_mod_ids, &spt_names);
        assert!(
            failures.is_empty(),
            "dotted directory name should match via containment, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_reverse_package_directory() {
        // Real-world: directory "com.swiftxp.spt.showmethemoney" loads as "Show Me The Money"
        let installed = vec![InstalledMod {
            version: "2.7.0".to_string(),
            ..test_mod(1, 100, "Show Me The Money (Item Pricing)")
        }];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("Show Me The Money".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let mut spt_names = std::collections::HashMap::new();
        spt_names.insert(1_i64, "com.swiftxp.spt.showmethemoney".to_string());

        let (failures, untracked) =
            check_mod_loads(&installed, &loaded, &server_mod_ids, &spt_names);
        assert!(
            failures.is_empty(),
            "reverse-domain directory should match via containment, got: {:?}",
            failures
        );
        assert!(untracked.is_empty());
    }

    #[test]
    fn check_mod_loads_genuine_failure_still_detected() {
        // A mod that is truly not loaded should still be reported
        let installed = vec![test_mod(1, 100, "Completely Unique Mod Name")];
        let mut loaded = std::collections::HashMap::new();
        loaded.insert("Totally Different Thing".to_string(), serde_json::json!({}));

        let server_mod_ids: std::collections::HashSet<i64> = [1].into_iter().collect();
        let mut spt_names = std::collections::HashMap::new();
        spt_names.insert(1_i64, "UniqueModDir".to_string());

        let (failures, untracked) =
            check_mod_loads(&installed, &loaded, &server_mod_ids, &spt_names);
        assert_eq!(
            failures,
            vec!["Completely Unique Mod Name"],
            "genuinely unloaded mod should be reported as failure"
        );
        assert_eq!(
            untracked,
            vec!["Totally Different Thing"],
            "genuinely untracked mod should be reported"
        );
    }

    #[test]
    fn normalize_mod_name_strips_nonalpha() {
        assert_eq!(
            normalize_mod_name("Bosses Have GP Coins"),
            "bosseshavegpcoins"
        );
        assert_eq!(
            normalize_mod_name("acidphantasm-bosseshavegpcoins"),
            "acidphantasmbosseshavegpcoins"
        );
        assert_eq!(
            normalize_mod_name("SAIN - Solarint's AI"),
            "sainsolarintsai"
        );
        assert_eq!(
            normalize_mod_name("[SVM] Server Value Modifier"),
            "svmservervaluemodifier"
        );
        assert_eq!(normalize_mod_name("com.tyfon.uifixes"), "comtyfonuifixes");
    }

    #[test]
    fn check_integrity_excludes_disabled_mod_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();

        let ctx = test_cli_context(tmp.path());
        let mod_id = ctx
            .db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();

        // Create file at canonical location
        let file_path = tmp.path().join("SPT/user/mods/TestMod/package.json");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, b"{}").unwrap();
        let hash = crate::spt::mods::compute_file_hash(&file_path).unwrap();
        ctx.db
            .insert_file(
                mod_id,
                "SPT/user/mods/TestMod/package.json",
                Some(&hash),
                Some(2),
            )
            .unwrap();

        // Verify it shows up in enabled file list
        let enabled = ctx.db.get_all_enabled_mod_files().unwrap();
        assert_eq!(enabled.len(), 1);

        // Disable the mod
        ctx.db.set_mod_disabled(mod_id, true).unwrap();

        // Should no longer appear in enabled file list
        let enabled = ctx.db.get_all_enabled_mod_files().unwrap();
        assert!(enabled.is_empty());
    }

    #[test]
    fn names_match_containment() {
        // Exact match
        assert!(names_match("lootingbots", "lootingbots"));
        // Containment: a contains b
        assert!(names_match(
            "acidphantasmbosseshavegpcoins",
            "bosseshavegpcoins"
        ));
        // Containment: b contains a
        assert!(names_match("sain", "solarintsainservermod"));
        // No match
        assert!(!names_match("uniquemod", "totallydifferent"));
        // Empty strings
        assert!(!names_match("", "something"));
        assert!(!names_match("something", ""));
    }

    #[test]
    fn resolve_spt_names_uses_directory_name() {
        let db = crate::db::Database::open_in_memory().unwrap();

        let mod_id = db
            .insert_mod(100, 200, "Fika Server", None, "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/fika-server/package.json",
            None,
            Some(100),
        )
        .unwrap();

        let server_mod_ids: std::collections::HashSet<i64> = [mod_id].into_iter().collect();
        let names = resolve_spt_names(&db, &server_mod_ids);

        // Should use directory name "fika-server", NOT package.json name "server"
        assert_eq!(names.get(&mod_id).map(|s| s.as_str()), Some("fika-server"));
    }

    #[test]
    fn check_integrity_parallel_detects_missing_file() {
        use std::sync::atomic::AtomicUsize;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();

        let ctx = test_cli_context(tmp.path());
        let mod_id = ctx
            .db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();
        ctx.db
            .insert_file(
                mod_id,
                "SPT/user/mods/TestMod/test.dll",
                Some("abc123"),
                Some(100),
            )
            .unwrap();

        let tracked = ctx.db.get_all_tracked_files().unwrap();
        let progress = AtomicUsize::new(0);
        let total = AtomicUsize::new(0);
        let result = check_integrity_parallel(&tracked, &ctx.spt_dir, &progress, &total).unwrap();
        assert_eq!(result.tracked_files, 1);
        assert_eq!(result.missing_files, vec!["SPT/user/mods/TestMod/test.dll"]);
        assert!(result.modified_files.is_empty());
        assert_eq!(total.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn check_integrity_parallel_detects_modified_file() {
        use std::sync::atomic::AtomicUsize;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();

        let file_path = tmp.path().join("SPT/user/mods/TestMod/test.dll");
        std::fs::write(&file_path, b"original content").unwrap();
        let original_hash = crate::spt::mods::compute_file_hash(&file_path).unwrap();

        let ctx = test_cli_context(tmp.path());
        let mod_id = ctx
            .db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();
        ctx.db
            .insert_file(
                mod_id,
                "SPT/user/mods/TestMod/test.dll",
                Some(&original_hash),
                Some(16),
            )
            .unwrap();

        std::fs::write(&file_path, b"tampered content").unwrap();

        let tracked = ctx.db.get_all_tracked_files().unwrap();
        let progress = AtomicUsize::new(0);
        let total = AtomicUsize::new(0);
        let result = check_integrity_parallel(&tracked, &ctx.spt_dir, &progress, &total).unwrap();
        assert!(result.missing_files.is_empty());
        assert_eq!(
            result.modified_files,
            vec!["SPT/user/mods/TestMod/test.dll"]
        );
    }
}
