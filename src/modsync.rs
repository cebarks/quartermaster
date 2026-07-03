use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::{find_narconet_dir, Config, ModSyncConfig};
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

/// NarcoNet expects paths relative to the SPT game root.
fn normalize_sync_path(path: &str) -> String {
    path.strip_prefix("../").unwrap_or(path).to_string()
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
///
/// Emits group-based syncPaths for `BepInEx/plugins/` files. For non-plugin
/// BepInEx files (patchers, configs, etc.), emits directory-level sync paths
/// when QM tracks every file in that directory, otherwise individual file paths.
/// Disabled mods are excluded.
fn generate_config(
    ms_config: &ModSyncConfig,
    db: &Database,
    for_headless: bool,
    spt_dir: Option<&Path>,
) -> Result<ModSyncOutputConfig> {
    let mods = db.list_mods()?;

    // Build reverse lookup: forge_mod_id → group key
    let group_for_mod: std::collections::HashMap<i64, &str> = ms_config
        .groups
        .iter()
        .flat_map(|(key, group)| group.members.iter().map(move |&id| (id, key.as_str())))
        .collect();

    // Single pass: track plugin groups, collect non-plugin BepInEx files,
    // and gather archive file paths for runtime exclusion detection
    let mut has_ungrouped_plugins = false;
    let mut groups_with_plugins: std::collections::BTreeSet<&str> =
        std::collections::BTreeSet::new();
    let mut non_plugin_files: BTreeSet<String> = BTreeSet::new();
    let mut archive_plugin_paths: BTreeSet<String> = BTreeSet::new();
    let mut mod_plugin_dirs: BTreeSet<String> = BTreeSet::new();

    for m in &mods {
        if m.disabled {
            continue;
        }

        let files = db.get_files_for_mod(m.id)?;

        let has_plugin_files = files
            .iter()
            .any(|f| f.file_path.starts_with("BepInEx/plugins/"));

        if has_plugin_files {
            if let Some(&group_slug) = group_for_mod.get(&m.forge_mod_id) {
                if group_slug == CATCH_ALL_GROUP_SLUG {
                    has_ungrouped_plugins = true;
                } else {
                    groups_with_plugins.insert(group_slug);
                }
            } else {
                has_ungrouped_plugins = true;
            }
        }

        for f in &files {
            if f.file_path.starts_with("BepInEx/") && !f.file_path.starts_with("BepInEx/plugins/") {
                non_plugin_files.insert(f.file_path.clone());
            }

            if f.file_path.starts_with("BepInEx/plugins/") {
                if f.source == "archive" {
                    archive_plugin_paths.insert(f.file_path.clone());
                }
                let parts: Vec<&str> = f.file_path.split('/').collect();
                if parts.len() >= 4 && parts[2].starts_with(GROUP_DIR_PREFIX) {
                    mod_plugin_dirs.insert(format!(
                        "{}/{}/{}/{}",
                        parts[0], parts[1], parts[2], parts[3]
                    ));
                } else if parts.len() >= 3 {
                    mod_plugin_dirs.insert(format!("{}/{}/{}", parts[0], parts[1], parts[2]));
                }
            }
        }
    }

    let mut sync_paths: Vec<SyncPathEntry> = Vec::new();
    let has_groups = !groups_with_plugins.is_empty();

    // Emit parent plugins syncPath if there are ungrouped mods OR if groups
    // need a parent path for NarcoNet's specificity/exclusion system.
    // Always uses global settings — the "default" group is virtual/automatic.
    if has_ungrouped_plugins || has_groups {
        sync_paths.push(SyncPathEntry {
            path: "BepInEx/plugins".to_string(),
            name: "BepInEx/plugins".to_string(),
            enabled: true,
            enforced: ms_config.enforced,
            silent: ms_config.silent,
            restart_required: ms_config.restart_required,
        });
    }

    // Emit one syncPath per group
    for group_slug in &groups_with_plugins {
        let group = match ms_config.groups.get(*group_slug) {
            Some(g) => g,
            None => continue,
        };

        let mut enabled = group.enabled.unwrap_or(true);
        let enforced = group.enforced.unwrap_or(ms_config.enforced);
        let silent = group.silent.unwrap_or(ms_config.silent);
        let restart_required = group.restart_required.unwrap_or(ms_config.restart_required);

        if for_headless && group.exclude_headless {
            enabled = false;
        }

        sync_paths.push(SyncPathEntry {
            path: format!("BepInEx/plugins/quma-{group_slug}"),
            name: sanitize_name_for_bepinex(&group.display_name),
            enabled,
            enforced,
            silent,
            restart_required,
        });
    }

    // Non-plugin BepInEx files (patchers, configs, etc.)
    let extra_set: BTreeSet<&str> = ms_config
        .extra_sync_paths
        .iter()
        .map(|s| s.as_str())
        .collect();

    if !non_plugin_files.is_empty() {
        // Group files by their top-level BepInEx subdirectory (e.g. "BepInEx/patchers")
        let mut by_subdir: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for file_path in &non_plugin_files {
            let subdir = file_path
                .splitn(3, '/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/");
            by_subdir
                .entry(subdir)
                .or_default()
                .push(file_path.as_str());
        }

        for (subdir, tracked_files) in &by_subdir {
            if extra_set.contains(subdir.as_str()) {
                continue;
            }

            // Check if QM owns every file in this directory on disk
            let use_directory = spt_dir
                .map(|dir| all_files_tracked(dir, subdir, &non_plugin_files))
                .unwrap_or(false);

            if use_directory {
                sync_paths.push(SyncPathEntry {
                    path: normalize_sync_path(subdir),
                    name: subdir.clone(),
                    enabled: true,
                    enforced: ms_config.enforced,
                    silent: ms_config.silent,
                    restart_required: ms_config.restart_required,
                });
            } else {
                for &file_path in tracked_files {
                    if extra_set.contains(file_path) {
                        continue;
                    }
                    sync_paths.push(SyncPathEntry {
                        path: normalize_sync_path(file_path),
                        name: file_path.to_string(),
                        enabled: true,
                        enforced: ms_config.enforced,
                        silent: ms_config.silent,
                        restart_required: ms_config.restart_required,
                    });
                }
            }
        }
    }

    // Extra sync paths
    for extra in &ms_config.extra_sync_paths {
        let path = normalize_sync_path(extra);
        sync_paths.push(SyncPathEntry {
            path,
            name: extra.clone(),
            enabled: true,
            enforced: ms_config.enforced,
            silent: ms_config.silent,
            restart_required: ms_config.restart_required,
        });
    }

    // Sort for deterministic output
    sync_paths.sort_by(|a, b| a.path.cmp(&b.path));

    // Build exclusions
    let mut exclusions: Vec<String> = ms_config
        .exclusions
        .iter()
        .map(|e| normalize_sync_path(e))
        .collect();
    if has_groups {
        exclusions.push("quma-*".to_string());
    }

    // Auto-exclude files in mod directories that aren't from the original
    // archive. Mods like SAIN generate config files at runtime inside
    // BepInEx/plugins/ — without exclusions, NarcoNet flags them as
    // mismatched on every client boot.
    if let Some(spt_dir) = spt_dir {
        let auto = collect_runtime_exclusions(spt_dir, &archive_plugin_paths, &mod_plugin_dirs);
        for path in auto {
            if !exclusions.contains(&path) {
                exclusions.push(path);
            }
        }
    }

    Ok(ModSyncOutputConfig {
        sync_paths,
        exclusions,
    })
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

/// The `quma-` prefix used for group directories on disk.
const GROUP_DIR_PREFIX: &str = "quma-";

/// A group with this slug is the catch-all: it auto-includes all mods not in
/// another group. Its mods stay in the standard `BepInEx/plugins/` location
/// (no `quma-default/` move), and its flags apply to the parent syncPath.
const CATCH_ALL_GROUP_SLUG: &str = "default";

/// Ensure a mod's BepInEx files are in the correct directory based on group
/// membership. Moves files and updates DB paths if needed. Returns true if
/// any files were moved.
pub fn ensure_mod_layout(
    spt_dir: &Path,
    ms_config: &ModSyncConfig,
    db: &Database,
    mod_db_id: i64,
) -> Result<bool> {
    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod not found: {mod_db_id}"))?;

    if mod_info.disabled {
        return Ok(false);
    }

    // Find which group this mod belongs to (if any).
    // The catch-all "default" group is treated as ungrouped — its mods stay
    // in the standard BepInEx/plugins/ location.
    let group_slug: Option<&str> = ms_config
        .groups
        .iter()
        .find(|(slug, g)| {
            slug.as_str() != CATCH_ALL_GROUP_SLUG && g.members.contains(&mod_info.forge_mod_id)
        })
        .map(|(slug, _)| slug.as_str());

    let files = db.get_files_for_mod(mod_db_id)?;

    // Only move BepInEx/plugins/ files for group layout (non-plugin files are synced directly)
    let plugin_files: Vec<&str> = files
        .iter()
        .filter(|f| f.file_path.starts_with("BepInEx/plugins/"))
        .map(|f| f.file_path.as_str())
        .collect();

    // Skip flat files (e.g., BepInEx/plugins/SomeFile.dll) — they're synced via parent path
    let plugin_files: Vec<&str> = plugin_files
        .into_iter()
        .filter(|p| p.split('/').count() >= 4)
        .collect();

    if plugin_files.is_empty() {
        return Ok(false);
    }

    // Find the current mod directory from a sample file
    let sample = plugin_files[0];
    let parts: Vec<&str> = sample.split('/').collect();

    let (current_prefix, mod_dir_name) =
        if parts.len() >= 4 && parts[2].starts_with(GROUP_DIR_PREFIX) {
            // Currently in a group dir: BepInEx/plugins/quma-<group>/<mod>/...
            let prefix = format!("BepInEx/plugins/{}/{}", parts[2], parts[3]);
            (prefix, parts[3].to_string())
        } else if parts.len() >= 3 {
            // Standard location: BepInEx/plugins/<mod>/...
            let prefix = format!("BepInEx/plugins/{}", parts[2]);
            (prefix, parts[2].to_string())
        } else {
            return Ok(false);
        };

    let expected_prefix = if let Some(slug) = group_slug {
        format!("BepInEx/plugins/quma-{slug}/{mod_dir_name}")
    } else {
        format!("BepInEx/plugins/{mod_dir_name}")
    };

    if current_prefix == expected_prefix {
        return Ok(false);
    }

    // Safety: check if other mods share this directory. If so, only move
    // if ALL mods sharing the directory belong to the same target group.
    // Otherwise skip to avoid corrupting other mods' DB records.
    let src = spt_dir.join(&current_prefix);
    if src.is_dir() {
        let all_mods = db.list_mods()?;
        for other in &all_mods {
            if other.id == mod_db_id {
                continue;
            }
            let other_files = db.get_files_for_mod(other.id)?;
            let shares_dir = other_files
                .iter()
                .any(|f| f.file_path.starts_with(&format!("{current_prefix}/")));
            if shares_dir {
                // Another mod shares this directory — find its expected group
                let other_group = ms_config
                    .groups
                    .iter()
                    .find(|(_, g)| g.members.contains(&other.forge_mod_id))
                    .map(|(slug, _)| slug.as_str());
                if other_group != group_slug {
                    tracing::warn!(
                        mod_name = %mod_info.name,
                        other_mod = %other.name,
                        dir = %current_prefix,
                        "skipping layout move — mods share directory but are in different groups"
                    );
                    return Ok(false);
                }
            }
        }
    }

    let dst = spt_dir.join(&expected_prefix);

    if src.is_symlink() {
        tracing::warn!(path = %src.display(), "skipping symlink during layout move");
        return Ok(false);
    }

    if !src.exists() {
        tracing::warn!(
            path = %src.display(),
            "source directory missing during layout move"
        );
        return Ok(false);
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if let Err(_rename_err) = std::fs::rename(&src, &dst) {
        // Cross-device fallback: copy + delete
        copy_dir_recursive(&src, &dst)?;
        std::fs::remove_dir_all(&src)?;
    }

    // Update DB paths for this mod
    db.reprefix_mod_files(mod_db_id, &current_prefix, &expected_prefix)?;

    // Also update DB paths for any other mods sharing this directory
    // (they were validated above to be in the same group)
    let all_mods = db.list_mods()?;
    for other in &all_mods {
        if other.id == mod_db_id {
            continue;
        }
        let other_files = db.get_files_for_mod(other.id)?;
        if other_files
            .iter()
            .any(|f| f.file_path.starts_with(&format!("{current_prefix}/")))
        {
            db.reprefix_mod_files(other.id, &current_prefix, &expected_prefix)?;
        }
    }

    // Clean up empty parent directory (e.g., empty quma-<group>/ after moving out)
    if let Some(parent) = src.parent() {
        if parent.is_dir() && is_dir_empty(parent) {
            let _ = std::fs::remove_dir(parent);
        }
    }

    tracing::debug!(
        mod_name = %mod_info.name,
        from = %current_prefix,
        to = %expected_prefix,
        "moved mod files for group layout"
    );

    Ok(true)
}

/// Ensure all mods with client files are in the correct layout.
/// Returns the number of mods that were moved.
pub fn ensure_all_mod_layouts(
    spt_dir: &Path,
    ms_config: &ModSyncConfig,
    db: &Database,
) -> Result<usize> {
    let mods = db.list_mods()?;
    let mut moved_count = 0;
    for m in &mods {
        match ensure_mod_layout(spt_dir, ms_config, db, m.id) {
            Ok(true) => moved_count += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(
                    mod_name = %m.name,
                    err = %e,
                    "failed to ensure mod layout"
                );
            }
        }
    }
    if moved_count > 0 {
        tracing::info!(
            moved_count,
            "reconciled mod file layouts for NarcoNet groups"
        );
    }
    Ok(moved_count)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Scan mod directories on disk and find files/directories not from mod archives.
/// Excludes entire subdirectories when none of their contents are archive files,
/// otherwise excludes individual untracked files.
fn collect_runtime_exclusions(
    spt_dir: &Path,
    archive_paths: &BTreeSet<String>,
    mod_dirs: &BTreeSet<String>,
) -> Vec<String> {
    let mut exclusions: BTreeSet<String> = BTreeSet::new();

    for mod_dir_rel in mod_dirs {
        let mod_dir_abs = spt_dir.join(mod_dir_rel);
        if !mod_dir_abs.is_dir() {
            continue;
        }
        exclude_untracked_recursive(&mod_dir_abs, spt_dir, archive_paths, &mut exclusions);
    }

    exclusions.into_iter().collect()
}

/// Walk `dir` and add exclusions for files/subdirectories not in `archive_paths`.
/// If a subdirectory contains zero archive files, exclude it wholesale instead of
/// listing every file individually.
fn exclude_untracked_recursive(
    dir: &Path,
    spt_dir: &Path,
    archive_paths: &BTreeSet<String>,
    exclusions: &mut BTreeSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }

        if path.is_dir() {
            let dir_prefix = path
                .strip_prefix(spt_dir)
                .ok()
                .and_then(|r| r.to_str())
                .map(|r| format!("{r}/"));

            let has_archive = dir_prefix.as_ref().is_some_and(|prefix| {
                archive_paths
                    .range::<str, _>((
                        std::ops::Bound::Included(prefix.as_str()),
                        std::ops::Bound::Unbounded,
                    ))
                    .next()
                    .is_some_and(|p| p.starts_with(prefix.as_str()))
            });

            if has_archive {
                exclude_untracked_recursive(&path, spt_dir, archive_paths, exclusions);
            } else if let Some(prefix) = dir_prefix {
                exclusions.insert(prefix.trim_end_matches('/').to_string());
            }
        } else if let Ok(rel) = path.strip_prefix(spt_dir) {
            if let Some(rel_str) = rel.to_str() {
                if !archive_paths.contains(rel_str) {
                    exclusions.insert(rel_str.to_string());
                }
            }
        }
    }
}

/// Check if every file on disk under `spt_dir/subdir` is in `tracked_files`.
fn all_files_tracked(spt_dir: &Path, subdir: &str, tracked_files: &BTreeSet<String>) -> bool {
    let dir = spt_dir.join(subdir);
    if !dir.is_dir() {
        return false;
    }
    collect_files_recursive(&dir).into_iter().all(|disk_path| {
        disk_path
            .strip_prefix(spt_dir)
            .ok()
            .and_then(|rel| rel.to_str())
            .is_some_and(|rel| tracked_files.contains(rel))
    })
}

fn collect_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            files.extend(collect_files_recursive(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn is_dir_empty(path: &Path) -> bool {
    path.read_dir()
        .map(|mut d| d.next().is_none())
        .unwrap_or(false)
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
                std::fs::remove_file(&config_path)
                    .context("failed to remove NarcoNet config.yaml")?;
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

    let output = generate_config(ms_config, db, false, Some(spt_dir))?;
    write_config(&config_path, &output)?;

    Ok(true)
}

/// Generate and return a preview of the ModSync config as a YAML string.
pub fn preview_config(
    ms_config: &ModSyncConfig,
    db: &Database,
    for_headless: bool,
    spt_dir: Option<&Path>,
) -> Result<String> {
    let output = generate_config(ms_config, db, for_headless, spt_dir)?;
    let yaml = serde_saphyr::to_string(&output).context("failed to serialize preview config")?;
    Ok(format!(
        "# Generated by quartermaster — manual edits will be overwritten\n{yaml}"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::{ModSyncConfig, NARCONET_FORGE_MOD_ID};
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
    fn normalize_sync_path_strips_parent_prefix() {
        assert_eq!(normalize_sync_path("BepInEx"), "BepInEx");
        assert_eq!(normalize_sync_path("BepInEx/plugins"), "BepInEx/plugins");
        assert_eq!(normalize_sync_path("../BepInEx/plugins"), "BepInEx/plugins");
        assert_eq!(normalize_sync_path("user/mods"), "user/mods");
    }

    #[test]
    fn generate_config_client_mod_creates_sync_path() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert_eq!(output.sync_paths[0].name, "BepInEx/plugins");
        assert!(output.sync_paths[0].enforced);
        assert!(!output.sync_paths[0].silent);
        assert!(output.sync_paths[0].restart_required);
        assert!(output.sync_paths[0].enabled);
        assert!(output.exclusions.is_empty()); // no groups = no quma-* exclusion
    }

    #[test]
    fn generate_config_server_only_mod_excluded() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_server_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_hybrid_mod_only_syncs_client_paths() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_hybrid_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert_eq!(output.sync_paths[0].name, "BepInEx/plugins");
    }

    #[test]
    fn generate_config_group_override_applied() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "custom".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Custom".to_string(),
                members: vec![100],
                enabled: None,
                enforced: Some(false),
                silent: Some(true),
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Parent + group syncpath
        assert_eq!(output.sync_paths.len(), 2);
        let group_path = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(!group_path.enforced); // group override
        assert!(group_path.silent); // group override
        assert!(group_path.restart_required); // inherited global default
        assert!(output.exclusions.contains(&"quma-*".to_string()));
    }

    #[test]
    fn generate_config_group_disabled_mod() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "disabled".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Disabled".to_string(),
                members: vec![100],
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Parent + group syncpath
        assert_eq!(output.sync_paths.len(), 2);
        let group_path = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(!group_path.enabled);
    }

    #[test]
    fn generate_config_extra_sync_paths_get_prefix() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.extra_sync_paths = vec!["BepInEx/config".to_string()];

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/config");
        assert_eq!(output.sync_paths[0].name, "BepInEx/config");
        assert!(output.sync_paths[0].enforced);
    }

    #[test]
    fn generate_config_exclusions_get_prefix() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.exclusions = vec!["**/*.nosync".to_string(), "BepInEx/plugins/spt".to_string()];

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(
            output.exclusions,
            vec!["**/*.nosync", "BepInEx/plugins/spt"]
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
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Both ungrouped → single category-level syncPath
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert_eq!(output.sync_paths[0].name, "BepInEx/plugins");
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
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Ungrouped mod → parent syncpath gets global defaults
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
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
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Patcher files synced as individual files (no spt_dir → can't check ownership)
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/patchers/PatcherMod.dll");
    }

    #[test]
    fn generate_config_narconet_included() {
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

        // A normal client mod should also appear
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Both NarcoNet and SAIN contribute to the ungrouped plugins path
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert_eq!(output.sync_paths[0].name, "BepInEx/plugins");
    }

    #[test]
    fn generate_config_narconet_only_mod() {
        let db = Database::open_in_memory().unwrap();
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
            Some("aaa111"),
            Some(4096),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
    }

    #[test]
    fn write_config_creates_yaml_with_header() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.yaml");

        let output = ModSyncOutputConfig {
            sync_paths: vec![SyncPathEntry {
                path: "BepInEx/plugins/Test".to_string(),
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
        assert_eq!(sync_path["path"].as_str().unwrap(), "BepInEx/plugins/Test");
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
        assert!(content.contains("BepInEx/plugins"));
        assert!(content.contains("BepInEx/plugins"));

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
        // No mods → no syncPaths (only the empty array marker)
        assert!(!content.contains("BepInEx/plugins"));
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

        assert!(paths.contains(&"BepInEx/plugins".to_string()));
        assert!(paths.contains(&"BepInEx/config".to_string()));
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

        let output = generate_config(&ms_config, &db, false, None).unwrap();
        // Parent + group syncpath
        assert_eq!(output.sync_paths.len(), 2);
        let group_path = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(!group_path.enabled);
        assert!(output.exclusions.contains(&"quma-*".to_string()));
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

        let output = generate_config(&ms_config, &db, false, None).unwrap();
        // Parent + group syncpath
        assert_eq!(output.sync_paths.len(), 2);
        let group_path = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(!group_path.enforced); // group override
        assert!(!group_path.silent); // inherited global
        assert!(group_path.restart_required); // inherited global
        assert!(group_path.enabled); // default true
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

        // Player config: group syncpath is enabled
        let player = generate_config(&ms_config, &db, false, None).unwrap();
        let player_group = player
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(player_group.enabled);

        // Headless config: group syncpath is disabled
        let headless = generate_config(&ms_config, &db, true, None).unwrap();
        let headless_group = headless
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(!headless_group.enabled);
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

        let player = generate_config(&ms_config, &db, false, None).unwrap();
        let group_path = player
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert!(group_path.enabled); // exclude_headless ignored for players
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

        let output = generate_config(&ms_config, &db, false, None).unwrap();
        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_extra_sync_paths_unaffected_by_headless() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.extra_sync_paths = vec!["BepInEx/config".to_string()];

        let player = generate_config(&ms_config, &db, false, None).unwrap();
        let headless = generate_config(&ms_config, &db, true, None).unwrap();

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

        let output = generate_config(&ms_config, &db, false, None).unwrap();
        // SAIN is ungrouped, empty group has no plugin-bearing members
        // → only the parent syncpath, no group syncpaths
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert!(output.sync_paths[0].enabled); // mod not in group, gets global default
        assert!(output.exclusions.is_empty()); // no groups with plugins → no quma-* exclusion
    }

    #[test]
    fn generate_config_multiple_ungrouped_mods_single_syncpath() {
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
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Both ungrouped → single category-level syncPath
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert_eq!(output.sync_paths[0].name, "BepInEx/plugins");
    }

    #[test]
    fn generate_config_multiple_groups_get_independent_settings() {
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

        // ModA in a group with enforced=true
        ms_config.groups.insert(
            "grp-a".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Group A".to_string(),
                members: vec![100],
                enabled: None,
                enforced: Some(true),
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        // ModB in a group with silent=false
        ms_config.groups.insert(
            "grp-b".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Group B".to_string(),
                members: vec![101],
                enabled: None,
                enforced: None,
                silent: Some(false),
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Parent + 2 group syncpaths
        assert_eq!(output.sync_paths.len(), 3);

        let grp_a = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-grp-a"))
            .unwrap();
        assert!(grp_a.enforced); // group override
        assert!(grp_a.silent); // inherited global (true)

        let grp_b = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-grp-b"))
            .unwrap();
        assert!(!grp_b.enforced); // inherited global (false)
        assert!(!grp_b.silent); // group override

        assert!(output.exclusions.contains(&"quma-*".to_string()));
    }

    #[test]
    fn full_lifecycle_enable_disable_reenable() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let narconet_dir = spt_dir.join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());

        // Enable: config.yaml is created
        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);
        let config_path = modsync_config_path(spt_dir).unwrap();
        assert!(config_path.exists());

        // Disable: config.yaml is removed
        config.modsync.as_mut().unwrap().enabled = false;
        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(!result);
        assert!(!config_path.exists());

        // Re-enable: config.yaml is recreated
        config.modsync.as_mut().unwrap().enabled = true;
        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);
        assert!(config_path.exists());
    }

    #[test]
    fn generate_config_ungrouped_emits_category_syncpath() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db); // SAIN in BepInEx/plugins/

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins");
        assert_eq!(output.sync_paths[0].name, "BepInEx/plugins");
        assert!(output.sync_paths[0].enforced);
        assert!(output.exclusions.is_empty()); // no groups = no quma-* exclusion
    }

    #[test]
    fn generate_config_grouped_mod_emits_group_syncpath() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db); // forge_mod_id=100

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "optional".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Optional".to_string(),
                members: vec![100],
                enabled: Some(false),
                enforced: Some(false),
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Should have group syncpath only (no ungrouped mods)
        let group_path = output
            .sync_paths
            .iter()
            .find(|p| p.path.contains("quma-"))
            .unwrap();
        assert_eq!(group_path.path, "BepInEx/plugins/quma-optional");
        assert_eq!(group_path.name, "Optional");
        assert!(!group_path.enabled);
        assert!(!group_path.enforced);

        // quma-* exclusion present
        assert!(output.exclusions.iter().any(|e| e == "quma-*"));
    }

    #[test]
    fn generate_config_mixed_grouped_and_ungrouped() {
        let db = Database::open_in_memory().unwrap();

        // Ungrouped mod
        let mod1 = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod1,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc"),
            Some(1024),
        )
        .unwrap();

        // Grouped mod
        let mod2 = db
            .insert_mod(200, 300, "Donuts", Some("donuts"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod2,
            "BepInEx/plugins/Donuts/Donuts.dll",
            Some("def"),
            Some(512),
        )
        .unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "optional".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Optional".to_string(),
                members: vec![200],
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        let paths: Vec<&str> = output.sync_paths.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"BepInEx/plugins")); // default for SAIN
        assert!(paths.contains(&"BepInEx/plugins/quma-optional")); // group for Donuts
        assert!(output.exclusions.contains(&"quma-*".to_string()));
    }

    #[test]
    fn generate_config_no_client_mods_no_syncpaths() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_server_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_patcher_only_mod_synced_as_files() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod/patch.dll",
            Some("ppp"),
            Some(512),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Without spt_dir, patcher files are synced individually
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(
            output.sync_paths[0].path,
            "BepInEx/patchers/PatcherMod/patch.dll"
        );
    }

    #[test]
    fn ensure_mod_layout_moves_to_group_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let db = Database::open_in_memory().unwrap();

        // Create mod files on disk
        let mod_dir = spt_dir.join("BepInEx/plugins/SAIN");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("SAIN.dll"), b"test").unwrap();

        // Register in DB
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc"),
            Some(4),
        )
        .unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "optional".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Optional".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let moved = ensure_mod_layout(spt_dir, &ms_config, &db, mod_id).unwrap();
        assert!(moved);

        // Files should be at new location
        assert!(spt_dir
            .join("BepInEx/plugins/quma-optional/SAIN/SAIN.dll")
            .exists());
        assert!(!spt_dir.join("BepInEx/plugins/SAIN/SAIN.dll").exists());

        // DB should be updated
        let files = db.get_files_for_mod(mod_id).unwrap();
        assert_eq!(
            files[0].file_path,
            "BepInEx/plugins/quma-optional/SAIN/SAIN.dll"
        );
    }

    #[test]
    fn ensure_mod_layout_moves_back_from_group() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let db = Database::open_in_memory().unwrap();

        // Files already in group dir
        let mod_dir = spt_dir.join("BepInEx/plugins/quma-optional/SAIN");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("SAIN.dll"), b"test").unwrap();

        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/quma-optional/SAIN/SAIN.dll",
            Some("abc"),
            Some(4),
        )
        .unwrap();

        // No groups — mod is ungrouped now
        let ms_config = ModSyncConfig::default();

        let moved = ensure_mod_layout(spt_dir, &ms_config, &db, mod_id).unwrap();
        assert!(moved);

        assert!(spt_dir.join("BepInEx/plugins/SAIN/SAIN.dll").exists());
        assert!(!spt_dir.join("BepInEx/plugins/quma-optional").exists()); // cleaned up

        let files = db.get_files_for_mod(mod_id).unwrap();
        assert_eq!(files[0].file_path, "BepInEx/plugins/SAIN/SAIN.dll");
    }

    #[test]
    fn ensure_mod_layout_noop_when_already_correct() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let db = Database::open_in_memory().unwrap();

        let mod_dir = spt_dir.join("BepInEx/plugins/SAIN");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("SAIN.dll"), b"test").unwrap();

        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc"),
            Some(4),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default(); // no groups

        let moved = ensure_mod_layout(spt_dir, &ms_config, &db, mod_id).unwrap();
        assert!(!moved);
    }

    #[test]
    fn ensure_mod_layout_skips_disabled_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        let db = Database::open_in_memory().unwrap();

        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN.disabled/SAIN.dll",
            Some("abc"),
            Some(4),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "optional".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Optional".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let moved = ensure_mod_layout(spt_dir, &ms_config, &db, mod_id).unwrap();
        assert!(!moved); // disabled mods are skipped
    }

    // ─── Non-plugin BepInEx file sync tests ────────────────────────────────

    #[test]
    fn generate_config_patcher_files_synced_individually_without_spt_dir() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod/patch.dll",
            Some("ppp"),
            Some(512),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod/helper.dll",
            Some("hhh"),
            Some(256),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 2);
        let paths: Vec<&str> = output.sync_paths.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"BepInEx/patchers/PatcherMod/helper.dll"));
        assert!(paths.contains(&"BepInEx/patchers/PatcherMod/patch.dll"));
    }

    #[test]
    fn generate_config_patcher_dir_synced_when_all_files_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Create patcher files on disk — all tracked by QM
        let patchers_dir = spt_dir.join("BepInEx/patchers");
        std::fs::create_dir_all(&patchers_dir).unwrap();
        std::fs::write(patchers_dir.join("PatcherMod.dll"), b"data").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod.dll",
            Some("ppp"),
            Some(4),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        // All files in patchers/ are tracked → directory-level sync
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/patchers");
        assert_eq!(output.sync_paths[0].name, "BepInEx/patchers");
    }

    #[test]
    fn generate_config_patcher_dir_falls_back_to_files_when_untracked_present() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Create patcher files on disk — one tracked, one not
        let patchers_dir = spt_dir.join("BepInEx/patchers");
        std::fs::create_dir_all(&patchers_dir).unwrap();
        std::fs::write(patchers_dir.join("PatcherMod.dll"), b"data").unwrap();
        std::fs::write(patchers_dir.join("OtherPatcher.dll"), b"other").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod.dll",
            Some("ppp"),
            Some(4),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        // Untracked file on disk → individual file sync
        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/patchers/PatcherMod.dll");
    }

    #[test]
    fn generate_config_mixed_plugins_and_patchers() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "FullMod", Some("full-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/FullMod/FullMod.dll",
            Some("aaa"),
            Some(1024),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/FullMod/patcher.dll",
            Some("bbb"),
            Some(512),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        let paths: Vec<&str> = output.sync_paths.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"BepInEx/plugins"));
        assert!(paths.contains(&"BepInEx/patchers/FullMod/patcher.dll"));
        assert_eq!(output.sync_paths.len(), 2);
    }

    #[test]
    fn generate_config_non_plugin_skips_extra_sync_paths() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "ConfigMod", Some("config-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/config/ConfigMod.cfg",
            Some("ccc"),
            Some(128),
        )
        .unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.extra_sync_paths = vec!["BepInEx/config".to_string()];

        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // BepInEx/config is already in extra_sync_paths → no duplicate
        let config_paths: Vec<&str> = output
            .sync_paths
            .iter()
            .filter(|p| p.path.contains("config"))
            .map(|p| p.path.as_str())
            .collect();
        assert_eq!(config_paths.len(), 1);
        assert_eq!(config_paths[0], "BepInEx/config");
    }

    #[test]
    fn generate_config_non_plugin_uses_global_defaults() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod.dll",
            Some("ppp"),
            Some(512),
        )
        .unwrap();

        let ms_config = ModSyncConfig {
            enforced: false,
            silent: true,
            restart_required: false,
            ..ModSyncConfig::default()
        };
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert!(!output.sync_paths[0].enforced);
        assert!(output.sync_paths[0].silent);
        assert!(!output.sync_paths[0].restart_required);
    }

    #[test]
    fn generate_config_disabled_mod_non_plugin_files_excluded() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod.dll",
            Some("ppp"),
            Some(512),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_multiple_non_plugin_subdirs() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "BigMod", Some("big-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/BigMod.dll",
            Some("aaa"),
            Some(512),
        )
        .unwrap();
        db.insert_file(mod_id, "BepInEx/config/BigMod.cfg", Some("bbb"), Some(128))
            .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        let paths: Vec<&str> = output.sync_paths.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"BepInEx/config/BigMod.cfg"));
        assert!(paths.contains(&"BepInEx/patchers/BigMod.dll"));
        assert_eq!(output.sync_paths.len(), 2);
    }

    // ─── Runtime exclusion tests ──────────────────────────────────────

    #[test]
    fn generate_config_excludes_runtime_files_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Create archive files on disk
        let sain_dir = spt_dir.join("BepInEx/plugins/SAIN");
        std::fs::create_dir_all(&sain_dir).unwrap();
        std::fs::write(sain_dir.join("SAIN.dll"), b"dll").unwrap();

        // Create runtime-generated files (not from archive)
        std::fs::write(sain_dir.join("BotTypes.json"), b"{}").unwrap();
        let config_dir = sain_dir.join("Default Bot Config Values");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("Rashala.json"), b"{}").unwrap();
        std::fs::write(config_dir.join("Killa.json"), b"{}").unwrap();
        let presets_dir = sain_dir.join("Presets");
        std::fs::create_dir_all(&presets_dir).unwrap();
        std::fs::write(presets_dir.join("ConfigSettings.json"), b"{}").unwrap();

        // DB only tracks the archive file
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc123"),
            Some(3),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        // Individual runtime file excluded
        assert!(output
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/BotTypes.json".to_string()));
        // Entire directories with no archive files excluded wholesale
        assert!(output
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/Default Bot Config Values".to_string()));
        assert!(output
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/Presets".to_string()));
        // Archive files NOT excluded
        assert!(!output
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/SAIN.dll".to_string()));
    }

    #[test]
    fn generate_config_no_runtime_exclusions_without_spt_dir() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, None).unwrap();

        // Without spt_dir, no disk scan → no auto-exclusions
        assert!(output.exclusions.is_empty());
    }

    #[test]
    fn generate_config_runtime_exclusions_merged_with_user_exclusions() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        let sain_dir = spt_dir.join("BepInEx/plugins/SAIN");
        std::fs::create_dir_all(&sain_dir).unwrap();
        std::fs::write(sain_dir.join("SAIN.dll"), b"dll").unwrap();
        std::fs::write(sain_dir.join("BotTypes.json"), b"{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc"),
            Some(3),
        )
        .unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.exclusions = vec!["**/*.log".to_string()];

        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        // Both user and auto exclusions present
        assert!(output.exclusions.contains(&"**/*.log".to_string()));
        assert!(output
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/BotTypes.json".to_string()));
    }

    #[test]
    fn generate_config_runtime_exclusions_skip_disabled_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        let sain_dir = spt_dir.join("BepInEx/plugins/SAIN");
        std::fs::create_dir_all(&sain_dir).unwrap();
        std::fs::write(sain_dir.join("SAIN.dll"), b"dll").unwrap();
        std::fs::write(sain_dir.join("BotTypes.json"), b"{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc"),
            Some(3),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        // Disabled mod → no sync paths AND no runtime exclusions
        assert!(output.sync_paths.is_empty());
        assert!(output.exclusions.is_empty());
    }

    #[test]
    fn generate_config_runtime_exclusions_with_groups() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Mod is in a group directory
        let mod_dir = spt_dir.join("BepInEx/plugins/quma-optional/SAIN");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("SAIN.dll"), b"dll").unwrap();
        std::fs::write(mod_dir.join("BotTypes.json"), b"{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/quma-optional/SAIN/SAIN.dll",
            Some("abc"),
            Some(3),
        )
        .unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.groups.insert(
            "optional".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Optional".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );

        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        assert!(output
            .exclusions
            .contains(&"BepInEx/plugins/quma-optional/SAIN/BotTypes.json".to_string()));
    }

    #[test]
    fn generate_config_no_false_exclusions_when_all_tracked() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        let sain_dir = spt_dir.join("BepInEx/plugins/SAIN");
        std::fs::create_dir_all(&sain_dir).unwrap();
        std::fs::write(sain_dir.join("SAIN.dll"), b"dll").unwrap();
        std::fs::write(sain_dir.join("config.json"), b"{}").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc"),
            Some(3),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/config.json",
            Some("def"),
            Some(2),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db, false, Some(spt_dir)).unwrap();

        // All files on disk are from the archive → no auto-exclusions
        assert!(output.exclusions.is_empty());
    }
}
