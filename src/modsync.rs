use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::{find_narconet_dir, Config, ModSyncConfig, NARCONET_FORGE_MOD_ID};
use crate::db::Database;

/// A single syncPath entry in NarcoNet's config.yaml.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncPathEntry {
    path: String,
    name: String,
    enabled: bool,
    enforced: bool,
    silent: bool,
    restart_required: bool,
}

/// The full NarcoNet config.yaml structure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModSyncOutputConfig {
    sync_paths: Vec<SyncPathEntry>,
    exclusions: Vec<String>,
}

pub fn modsync_config_path(spt_dir: &Path) -> Option<PathBuf> {
    find_narconet_dir(spt_dir).map(|dir| dir.join("config.yaml"))
}

fn prepend_parent_if_needed(path: &str) -> String {
    if path.starts_with("BepInEx/") || path == "BepInEx" {
        format!("../{path}")
    } else {
        path.to_string()
    }
}

/// Strip characters that BepInEx forbids in config section/key names.
/// NarcoNet uses sync path names as BepInEx config keys, so these chars
/// cause `ConfigDefinition` to throw on the client.
fn sanitize_name_for_bepinex(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '=' | '\n' | '\t' | '\\' | '"' | '\'' | '[' | ']'))
        .collect()
}

/// Generate ModSync config from DB state + quartermaster config.
fn generate_config(
    ms_config: &ModSyncConfig,
    db: &Database,
    for_headless: bool,
) -> Result<ModSyncOutputConfig> {
    let mods = db.list_mods()?;
    let mut sync_paths: std::collections::BTreeMap<String, SyncPathEntry> =
        std::collections::BTreeMap::new();

    // Build reverse lookup: mod forge_id → group key
    let group_for_mod: std::collections::HashMap<i64, &str> = ms_config
        .groups
        .iter()
        .flat_map(|(key, group)| group.members.iter().map(move |&id| (id, key.as_str())))
        .collect();

    for m in &mods {
        if m.forge_mod_id == NARCONET_FORGE_MOD_ID {
            continue;
        }

        let files = db.get_files_for_mod(m.id)?;
        let client_files: Vec<&str> = files
            .iter()
            .filter(|f| f.file_path.starts_with("BepInEx/"))
            .map(|f| f.file_path.as_str())
            .collect();

        if client_files.is_empty() {
            continue;
        }

        let forge_id_str = m.forge_mod_id.to_string();
        let overrides = ms_config.overrides.get(&forge_id_str);
        let group = group_for_mod
            .get(&m.forge_mod_id)
            .and_then(|key| ms_config.groups.get(*key));

        // Step 1: start with global defaults
        let mut enabled = true;
        let mut enforced = ms_config.enforced;
        let mut silent = ms_config.silent;
        let mut restart_required = ms_config.restart_required;

        // Step 2+3: apply group settings over global
        if let Some(g) = group {
            if let Some(v) = g.enabled {
                enabled = v;
            }
            if let Some(v) = g.enforced {
                enforced = v;
            }
            if let Some(v) = g.silent {
                silent = v;
            }
            if let Some(v) = g.restart_required {
                restart_required = v;
            }

            // Step 4: headless exclusion
            if for_headless && g.exclude_headless {
                enabled = false;
            }
        }

        // Step 5: per-mod overrides win
        if let Some(o) = overrides {
            if let Some(v) = o.enabled {
                enabled = v;
            }
            if let Some(v) = o.enforced {
                enforced = v;
            }
            if let Some(v) = o.silent {
                silent = v;
            }
            if let Some(v) = o.restart_required {
                restart_required = v;
            }
        }

        for dir in deduplicate_to_directories(&client_files) {
            let path = prepend_parent_if_needed(&dir);
            sync_paths
                .entry(path.clone())
                .and_modify(|existing| {
                    // When multiple mods share a directory, merge flags conservatively:
                    // enable if any mod enables, enforce if any enforces, etc.
                    existing.enabled = existing.enabled || enabled;
                    existing.enforced = existing.enforced || enforced;
                    existing.silent = existing.silent && silent;
                    existing.restart_required = existing.restart_required || restart_required;
                })
                .or_insert(SyncPathEntry {
                    path,
                    name: sanitize_name_for_bepinex(&m.name),
                    enabled,
                    enforced,
                    silent,
                    restart_required,
                });
        }
    }

    for extra in &ms_config.extra_sync_paths {
        let path = prepend_parent_if_needed(extra);
        sync_paths.entry(path.clone()).or_insert(SyncPathEntry {
            path,
            name: extra.clone(),
            enabled: true,
            enforced: ms_config.enforced,
            silent: ms_config.silent,
            restart_required: ms_config.restart_required,
        });
    }

    let sync_paths: Vec<SyncPathEntry> = sync_paths.into_values().collect();

    let exclusions = ms_config
        .exclusions
        .iter()
        .map(|e| prepend_parent_if_needed(e))
        .collect();

    Ok(ModSyncOutputConfig {
        sync_paths,
        exclusions,
    })
}

/// Given a list of file paths under BepInEx/, deduplicate to the minimal
/// set of directories. If multiple files share a common parent like
/// `BepInEx/plugins/SAIN/`, use the directory. If a file is alone at a
/// level (e.g., `BepInEx/patchers/Mod.dll`), use the file path directly.
fn deduplicate_to_directories(paths: &[&str]) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut dirs: BTreeSet<String> = BTreeSet::new();

    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        // BepInEx/<category>/<mod_name_or_file>/...
        // If 3+ parts and the 3rd part has no extension, treat first 3 as directory
        // If 3 parts and the last has an extension, it's a standalone file
        if parts.len() >= 3 {
            let third = parts[2];
            if parts.len() == 3 && third.contains('.') {
                // Standalone file like BepInEx/patchers/Mod.dll
                dirs.insert(path.to_string());
            } else {
                // Directory like BepInEx/plugins/SAIN
                dirs.insert(format!("{}/{}/{}", parts[0], parts[1], third));
            }
        }
    }

    dirs.into_iter().collect()
}

/// Write a ModSyncOutputConfig to config.yaml atomically.
fn write_config(config_path: &Path, output: &ModSyncOutputConfig) -> Result<()> {
    use anyhow::Context;
    let yaml =
        serde_saphyr::to_string(output).context("failed to serialize modsync config to YAML")?;
    let content =
        format!("# Generated by quartermaster — manual edits will be overwritten\n{yaml}");

    let tmp_path = config_path.with_extension("yaml.tmp");
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, config_path)?;

    tracing::debug!(path = %config_path.display(), "wrote NarcoNet config");
    Ok(())
}

/// Regenerate config.yaml if NarcoNet is installed and [modsync] config is present.
/// Returns true if the config was written, false if skipped.
pub fn regenerate_if_enabled(spt_dir: &Path, config: &Config, db: &Database) -> Result<bool> {
    let ms_config = match &config.modsync {
        Some(c) => c,
        None => return Ok(false),
    };

    if !ms_config.enabled {
        if let Some(config_path) = modsync_config_path(spt_dir) {
            if config_path.exists() {
                std::fs::remove_file(&config_path).context("failed to remove NarcoNet config.yaml")?;
                tracing::info!("NarcoNet management disabled — removed config.yaml");
            }
        }
        return Ok(false);
    }

    let config_path = match modsync_config_path(spt_dir) {
        Some(p) => p,
        None => {
            tracing::debug!("NarcoNet not installed, skipping config generation");
            return Ok(false);
        }
    };

    let output = generate_config(ms_config, db, false)?;
    write_config(&config_path, &output)?;

    Ok(true)
}

/// Generate and return a preview of the ModSync config as a YAML string.
pub fn preview_config(
    ms_config: &ModSyncConfig,
    db: &Database,
    for_headless: bool,
) -> Result<String> {
    let output = generate_config(ms_config, db, for_headless)?;
    let yaml = serde_saphyr::to_string(&output).context("failed to serialize preview config")?;
    Ok(format!(
        "# Generated by quartermaster — manual edits will be overwritten\n{yaml}"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::{ModSyncConfig, ModSyncOverride, NARCONET_FORGE_MOD_ID};
    use crate::db::Database;

    fn setup_db_with_client_mod(db: &Database) -> i64 {
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc123"),
            Some(1024),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/config.json",
            Some("def456"),
            Some(256),
        )
        .unwrap();
        mod_id
    }

    fn setup_db_with_server_mod(db: &Database) -> i64 {
        let mod_id = db
            .insert_mod(200, 300, "ServerMod", Some("server-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/ServerMod/package.json",
            Some("aaa111"),
            Some(512),
        )
        .unwrap();
        mod_id
    }

    fn setup_db_with_hybrid_mod(db: &Database) -> i64 {
        let mod_id = db
            .insert_mod(300, 400, "HybridMod", Some("hybrid-mod"), "2.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/HybridMod/hybrid.dll",
            Some("bbb222"),
            Some(2048),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/HybridMod/package.json",
            Some("ccc333"),
            Some(128),
        )
        .unwrap();
        mod_id
    }

    #[test]
    fn sanitize_name_strips_bepinex_invalid_chars() {
        assert_eq!(
            sanitize_name_for_bepinex("[SAIN] Twitch Players"),
            "SAIN Twitch Players"
        );
        assert_eq!(
            sanitize_name_for_bepinex("Normal Mod Name"),
            "Normal Mod Name"
        );
        assert_eq!(sanitize_name_for_bepinex("Mod=\"test\""), "Modtest");
        assert_eq!(sanitize_name_for_bepinex("It's a mod"), "Its a mod");
    }

    #[test]
    fn prepend_parent_handles_bare_bepinex() {
        assert_eq!(prepend_parent_if_needed("BepInEx"), "../BepInEx");
        assert_eq!(
            prepend_parent_if_needed("BepInEx/plugins"),
            "../BepInEx/plugins"
        );
        assert_eq!(prepend_parent_if_needed("user/mods"), "user/mods");
    }

    #[test]
    fn generate_config_client_mod_creates_sync_path() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "../BepInEx/plugins/SAIN");
        assert_eq!(output.sync_paths[0].name, "SAIN");
        assert!(output.sync_paths[0].enforced);
        assert!(!output.sync_paths[0].silent);
        assert!(output.sync_paths[0].restart_required);
        assert!(output.sync_paths[0].enabled);
    }

    #[test]
    fn generate_config_server_only_mod_excluded() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_server_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_hybrid_mod_only_syncs_client_paths() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_hybrid_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "../BepInEx/plugins/HybridMod");
        assert_eq!(output.sync_paths[0].name, "HybridMod");
    }

    #[test]
    fn generate_config_per_mod_override_applied() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(false),
                silent: Some(true),
                restart_required: None,
                enabled: None,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert!(!output.sync_paths[0].enforced);
        assert!(output.sync_paths[0].silent);
        assert!(output.sync_paths[0].restart_required);
    }

    #[test]
    fn generate_config_disabled_mod_override() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert!(!output.sync_paths[0].enabled);
    }

    #[test]
    fn generate_config_extra_sync_paths_get_prefix() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.extra_sync_paths = vec!["BepInEx/config".to_string()];

        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "../BepInEx/config");
        assert_eq!(output.sync_paths[0].name, "BepInEx/config");
        assert!(output.sync_paths[0].enforced);
    }

    #[test]
    fn generate_config_exclusions_get_prefix() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.exclusions = vec!["**/*.nosync".to_string(), "BepInEx/plugins/spt".to_string()];

        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(
            output.exclusions,
            vec!["**/*.nosync", "../BepInEx/plugins/spt"]
        );
    }

    #[test]
    fn generate_config_multiple_mods_sorted() {
        let db = Database::open_in_memory().unwrap();
        let mod1 = db
            .insert_mod(100, 200, "Zebra", Some("zebra"), "1.0.0")
            .unwrap();
        db.insert_file(mod1, "BepInEx/plugins/Zebra/z.dll", Some("aaa"), Some(100))
            .unwrap();

        let mod2 = db
            .insert_mod(101, 201, "Alpha", Some("alpha"), "1.0.0")
            .unwrap();
        db.insert_file(mod2, "BepInEx/plugins/Alpha/a.dll", Some("bbb"), Some(100))
            .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 2);
        assert_eq!(output.sync_paths[0].path, "../BepInEx/plugins/Alpha");
        assert_eq!(output.sync_paths[1].path, "../BepInEx/plugins/Zebra");
    }

    #[test]
    fn generate_config_global_defaults_applied() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig {
            enforced: false,
            silent: true,
            restart_required: false,
            ..ModSyncConfig::default()
        };
        let output = generate_config(&ms_config, &db, false).unwrap();

        assert!(!output.sync_paths[0].enforced);
        assert!(output.sync_paths[0].silent);
        assert!(!output.sync_paths[0].restart_required);
    }

    #[test]
    fn generate_config_mod_with_patcher_files() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod.dll",
            Some("ppp111"),
            Some(512),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(
            output.sync_paths[0].path,
            "../BepInEx/patchers/PatcherMod.dll"
        );
    }

    #[test]
    fn generate_config_narconet_self_excluded() {
        let db = Database::open_in_memory().unwrap();
        // NarcoNet's own mod with Forge ID 2441
        let mod_id = db
            .insert_mod(
                NARCONET_FORGE_MOD_ID,
                999,
                "NarcoNet",
                Some("narconet"),
                "1.0.16",
            )
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/MadManBeavis-NarcoNet/NarcoNet.dll",
            Some("nnn111"),
            Some(4096),
        )
        .unwrap();

        // A normal client mod should still appear
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        // Only SAIN should appear, not NarcoNet
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].name, "SAIN");
    }

    #[test]
    fn write_config_creates_yaml_with_header() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");

        let output = ModSyncOutputConfig {
            sync_paths: vec![SyncPathEntry {
                path: "../BepInEx/plugins/Test".to_string(),
                name: "Test".to_string(),
                enabled: true,
                enforced: true,
                silent: false,
                restart_required: true,
            }],
            exclusions: vec!["**/*.nosync".to_string()],
        };

        write_config(&config_path, &output).unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.starts_with("# Generated by quartermaster"));
        // Should be valid YAML
        let parsed: serde_json::Value = serde_saphyr::from_str(&content).unwrap();
        assert!(parsed["syncPaths"].is_array());

        // Verify field values
        let sync_path = &parsed["syncPaths"][0];
        assert_eq!(
            sync_path["path"].as_str().unwrap(),
            "../BepInEx/plugins/Test"
        );
        assert_eq!(sync_path["name"].as_str().unwrap(), "Test");
        assert_eq!(sync_path["enabled"].as_bool().unwrap(), true);
    }

    #[test]
    fn regenerate_if_enabled_skips_when_no_modsync_config() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let config = Config::default();

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(!result);
    }

    #[test]
    fn regenerate_if_enabled_skips_when_narconet_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(!result);
    }

    #[test]
    fn regenerate_if_enabled_writes_when_configured_and_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let narconet_dir = tmp.path().join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(result);

        let config_path = modsync_config_path(tmp.path()).unwrap();
        assert!(config_path.exists());
        assert!(config_path.to_str().unwrap().ends_with("config.yaml"));
    }

    #[test]
    fn regenerate_if_enabled_skips_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let narconet_dir = tmp.path().join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut config = Config::default();
        let mut ms = ModSyncConfig::default();
        ms.enabled = false;
        config.modsync = Some(ms);

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(!result);
    }

    #[test]
    fn regenerate_if_enabled_deletes_config_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let narconet_dir = tmp.path().join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();

        let config_path = narconet_dir.join("config.yaml");
        std::fs::write(&config_path, "old content").unwrap();
        assert!(config_path.exists());

        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        let mut ms = ModSyncConfig::default();
        ms.enabled = false;
        config.modsync = Some(ms);

        regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(!config_path.exists());
    }

    #[test]
    fn full_lifecycle_install_update_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let narconet_dir = spt_dir.join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());

        // Install a client mod
        let mod_id = db
            .insert_mod(100, 200, "TestClientMod", Some("test-client"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/TestClientMod/test.dll",
            Some("hash1"),
            Some(1024),
        )
        .unwrap();

        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);

        let content = std::fs::read_to_string(modsync_config_path(spt_dir).unwrap()).unwrap();
        assert!(content.contains("TestClientMod"));
        assert!(content.contains("../BepInEx/plugins/TestClientMod"));

        // Update — add a second file
        db.insert_file(
            mod_id,
            "BepInEx/plugins/TestClientMod/extra.dll",
            Some("hash2"),
            Some(512),
        )
        .unwrap();

        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);

        // Remove the mod
        db.delete_files_for_mod(mod_id).unwrap();
        db.delete_mod(mod_id).unwrap();

        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);

        let content = std::fs::read_to_string(modsync_config_path(spt_dir).unwrap()).unwrap();
        assert!(!content.contains("TestClientMod"));
    }

    #[test]
    fn full_lifecycle_mixed_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let narconet_dir = spt_dir.join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig {
            extra_sync_paths: vec!["BepInEx/config".to_string()],
            exclusions: vec!["**/*.log".to_string()],
            ..ModSyncConfig::default()
        });

        // Server-only mod
        let _server_mod = db
            .insert_mod(200, 300, "ServerOnly", None, "1.0.0")
            .unwrap();
        db.insert_file(
            _server_mod,
            "SPT/user/mods/ServerOnly/package.json",
            Some("s1"),
            Some(100),
        )
        .unwrap();

        // Client mod
        let client_mod = db.insert_mod(300, 400, "ClientMod", None, "1.0.0").unwrap();
        db.insert_file(
            client_mod,
            "BepInEx/plugins/ClientMod/client.dll",
            Some("c1"),
            Some(200),
        )
        .unwrap();

        regenerate_if_enabled(spt_dir, &config, &db).unwrap();

        let content = std::fs::read_to_string(modsync_config_path(spt_dir).unwrap()).unwrap();
        let parsed: serde_json::Value = serde_saphyr::from_str(&content).unwrap();

        let paths: Vec<String> = parsed["syncPaths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["path"].as_str().unwrap().to_string())
            .collect();

        assert!(paths.contains(&"../BepInEx/plugins/ClientMod".to_string()));
        assert!(paths.contains(&"../BepInEx/config".to_string()));
        assert!(!paths.iter().any(|p| p.contains("ServerOnly")));

        let exclusions: Vec<String> = parsed["exclusions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_string())
            .collect();
        assert_eq!(exclusions, vec!["**/*.log"]);
    }

    #[test]
    fn generate_config_group_disables_mod() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db); // forge_mod_id=100

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "optional".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Optional".to_string(),
                members: vec![100],
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();
        assert_eq!(output.sync_paths.len(), 1);
        assert!(!output.sync_paths[0].enabled);
    }

    #[test]
    fn generate_config_group_partial_settings_inherit_global() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig {
            enforced: true,
            silent: false,
            restart_required: true,
            ..ModSyncConfig::default()
        };
        ms_config.groups.insert(
            "custom".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Custom".to_string(),
                members: vec![100],
                enabled: None,
                enforced: Some(false),  // override global
                silent: None,           // inherit global (false)
                restart_required: None, // inherit global (true)
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();
        assert!(!output.sync_paths[0].enforced); // group override
        assert!(!output.sync_paths[0].silent); // inherited global
        assert!(output.sync_paths[0].restart_required); // inherited global
        assert!(output.sync_paths[0].enabled); // default true
    }

    #[test]
    fn generate_config_per_mod_override_wins_over_group() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "grp".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Grp".to_string(),
                members: vec![100],
                enabled: None,
                enforced: Some(true),
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(false), // override group
                silent: None,
                restart_required: None,
                enabled: None,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();
        assert!(!output.sync_paths[0].enforced); // per-mod wins
    }

    #[test]
    fn generate_config_headless_excludes_group_members() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "no-hl".to_string(),
            crate::config::ModSyncGroup {
                display_name: "No Headless".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: true,
            },
        );

        // Player config: mod is enabled
        let player = generate_config(&ms_config, &db, false).unwrap();
        assert!(player.sync_paths[0].enabled);

        // Headless config: mod is disabled
        let headless = generate_config(&ms_config, &db, true).unwrap();
        assert!(!headless.sync_paths[0].enabled);
    }

    #[test]
    fn generate_config_per_mod_enabled_overrides_headless_exclude() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "no-hl".to_string(),
            crate::config::ModSyncGroup {
                display_name: "No Headless".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: true,
            },
        );
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enabled: Some(true), // explicitly enabled
                enforced: None,
                silent: None,
                restart_required: None,
            },
        );

        let headless = generate_config(&ms_config, &db, true).unwrap();
        assert!(headless.sync_paths[0].enabled); // per-mod wins
    }

    #[test]
    fn generate_config_headless_false_ignores_exclude_headless() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "no-hl".to_string(),
            crate::config::ModSyncGroup {
                display_name: "No Headless".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: true,
            },
        );

        let player = generate_config(&ms_config, &db, false).unwrap();
        assert!(player.sync_paths[0].enabled); // exclude_headless ignored for players
    }

    #[test]
    fn generate_config_stale_group_member_skipped() {
        let db = Database::open_in_memory().unwrap();
        // forge_mod_id 9999 doesn't exist in DB
        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "grp".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Grp".to_string(),
                members: vec![9999],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();
        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_extra_sync_paths_unaffected_by_headless() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.extra_sync_paths = vec!["BepInEx/config".to_string()];

        let player = generate_config(&ms_config, &db, false).unwrap();
        let headless = generate_config(&ms_config, &db, true).unwrap();

        assert_eq!(player.sync_paths.len(), 1);
        assert!(player.sync_paths[0].enabled);
        assert_eq!(headless.sync_paths.len(), 1);
        assert!(headless.sync_paths[0].enabled); // extra paths always enabled
    }

    #[test]
    fn generate_config_empty_group_no_effect() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "empty".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Empty Group".to_string(),
                members: vec![],
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();
        assert_eq!(output.sync_paths.len(), 1);
        assert!(output.sync_paths[0].enabled); // mod not in group, gets global default
    }

    #[test]
    fn generate_config_deduplicates_shared_directory() {
        let db = Database::open_in_memory().unwrap();

        // Two mods installing files to the same BepInEx/plugins/SAIN/ directory
        let mod1 = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod1,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc123"),
            Some(1024),
        )
        .unwrap();

        let mod2 = db
            .insert_mod(
                101,
                201,
                "[SAIN] Twitch Players",
                Some("sain-twitch"),
                "1.0.0",
            )
            .unwrap();
        db.insert_file(
            mod2,
            "BepInEx/plugins/SAIN/TwitchPlayers.dll",
            Some("def456"),
            Some(512),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false).unwrap();

        // Should produce ONE entry, not two
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "../BepInEx/plugins/SAIN");
    }

    #[test]
    fn generate_config_shared_directory_merges_flags() {
        let db = Database::open_in_memory().unwrap();

        let mod1 = db
            .insert_mod(100, 200, "ModA", Some("mod-a"), "1.0.0")
            .unwrap();
        db.insert_file(mod1, "BepInEx/plugins/Shared/a.dll", Some("aaa"), Some(100))
            .unwrap();

        let mod2 = db
            .insert_mod(101, 201, "ModB", Some("mod-b"), "1.0.0")
            .unwrap();
        db.insert_file(mod2, "BepInEx/plugins/Shared/b.dll", Some("bbb"), Some(100))
            .unwrap();

        let mut ms_config = ModSyncConfig {
            enforced: false,
            silent: true,
            restart_required: false,
            ..ModSyncConfig::default()
        };

        // ModA: override enforced=true
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(true),
                silent: None,
                restart_required: None,
                enabled: None,
            },
        );

        // ModB: override silent=false
        ms_config.overrides.insert(
            "101".to_string(),
            ModSyncOverride {
                enforced: None,
                silent: Some(false),
                restart_required: None,
                enabled: None,
            },
        );

        let output = generate_config(&ms_config, &db, false).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        // enforced: true (OR — ModA has it)
        assert!(output.sync_paths[0].enforced);
        // silent: false (AND — ModB has it false)
        assert!(!output.sync_paths[0].silent);
    }
}
