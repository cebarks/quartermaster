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
}

#[derive(Debug, Clone, Serialize)]
pub struct ModsHealth {
    pub installed_count: usize,
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

    let installed_mods = ctx.db.list_mods()?;
    let mods = check_mods_compat(&installed_mods, ctx).await;

    let integrity = check_integrity(ctx)?;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}

async fn check_server(
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
    }
}

async fn check_mods_compat(
    installed_mods: &[crate::db::mods::InstalledMod],
    ctx: &CliContext,
) -> ModsHealth {
    let mut updates_available = 0;
    let mut incompatible_mods = Vec::new();

    if !installed_mods.is_empty() {
        let check_list: Vec<(i64, String)> = installed_mods
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();

        if let Ok(results) = ctx
            .forge
            .check_updates(&check_list, &ctx.spt_info.spt_version)
            .await
        {
            updates_available = results.iter().filter(|r| r.status == "updated").count();

            for r in &results {
                if r.status == "incompatible" {
                    let name = installed_mods
                        .iter()
                        .find(|m| m.forge_mod_id == r.mod_id)
                        .map(|m| m.name.as_str())
                        .unwrap_or("unknown");
                    incompatible_mods.push(name.to_string());
                }
            }
        }
    }

    ModsHealth {
        installed_count: installed_mods.len(),
        updates_available,
        incompatible_mods,
    }
}

fn check_integrity(ctx: &CliContext) -> Result<IntegrityHealth> {
    let tracked_files = ctx.db.get_all_tracked_files()?;
    let mut missing_files = Vec::new();
    let mut modified_files = Vec::new();

    for file in &tracked_files {
        let full_path = ctx.spt_dir.join(&file.file_path);
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

    let all_disk_files = scan_mod_directories(&ctx.spt_dir)?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let untracked: Vec<&str> = all_disk_files
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let mut dir_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for path in &untracked {
        let parts: Vec<&str> = path.split('/').collect();
        let dir = if parts.len() >= 3 {
            format!("{}/{}/{}", parts[0], parts[1], parts[2])
        } else {
            path.to_string()
        };
        *dir_counts.entry(dir).or_default() += 1;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn good_server() -> ServerHealth {
        ServerHealth {
            reachable: true,
            latency_ms: Some(12),
            version: Some("4.0.13".to_string()),
            version_matches: Some(true),
            address: "https://127.0.0.1:6969".to_string(),
            error: None,
        }
    }

    fn good_mods() -> ModsHealth {
        ModsHealth {
            installed_count: 5,
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
    fn exit_code_server_down_trumps_mod_issues() {
        let report = HealthReport {
            server: ServerHealth {
                reachable: false,
                latency_ms: None,
                version: None,
                version_matches: None,
                address: "https://127.0.0.1:6969".to_string(),
                error: Some("timeout".to_string()),
            },
            mods: ModsHealth {
                installed_count: 5,
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
        std::fs::create_dir_all(spt_dir.join("user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "user/mods/TestMod/test.dll",
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
            config_path: tmp.path().join("quartermaster.toml"),
            db,
            forge: ForgeClient::new(None).unwrap(),
        };

        let result = check_integrity(&ctx).unwrap();
        assert_eq!(result.tracked_files, 1);
        assert_eq!(result.missing_files, vec!["user/mods/TestMod/test.dll"]);
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
        std::fs::create_dir_all(spt_dir.join("user/mods/TestMod")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let file_path = spt_dir.join("user/mods/TestMod/test.dll");
        std::fs::write(&file_path, b"original content").unwrap();
        let original_hash = compute_file_hash(&file_path).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "user/mods/TestMod/test.dll",
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
            config_path: tmp.path().join("quartermaster.toml"),
            db,
            forge: ForgeClient::new(None).unwrap(),
        };

        let result = check_integrity(&ctx).unwrap();
        assert!(result.missing_files.is_empty());
        assert_eq!(result.modified_files, vec!["user/mods/TestMod/test.dll"]);
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
        std::fs::create_dir_all(spt_dir.join("user/mods/UnknownMod")).unwrap();
        std::fs::write(spt_dir.join("user/mods/UnknownMod/mod.dll"), b"x").unwrap();
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
            config_path: tmp.path().join("quartermaster.toml"),
            db,
            forge: ForgeClient::new(None).unwrap(),
        };

        let result = check_integrity(&ctx).unwrap();
        assert_eq!(result.tracked_files, 0);
        assert_eq!(result.untracked_dirs.len(), 1);
        assert_eq!(result.untracked_dirs[0].path, "user/mods/UnknownMod");
        assert_eq!(result.untracked_dirs[0].file_count, 1);
    }
}
