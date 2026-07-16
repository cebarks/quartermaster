use std::path::Path;

use anyhow::Result;

use crate::config::Config;
use crate::db::Database;
use crate::dirs::QumaDirs;

/// A "client file" is anything that belongs in the game client install —
/// everything EXCEPT server-side mods (`SPT/user/mods/`) and BepInEx config
/// (per-client overlay).
pub fn is_client_file(path: &str) -> bool {
    !path.starts_with("SPT/") && !path.starts_with("BepInEx/config/")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOp {
    Install,
    Remove,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub copied: usize,
    pub removed: usize,
    pub errors: usize,
}

#[derive(Debug, Clone)]
pub enum HeadlessSyncScope {
    /// Reconcile one mod and all its addons.
    Mod(i64),
    /// Full reconcile — all mods and all addons.
    Full,
}

/// Unified headless mod sync. Reads the DB, computes desired file state,
/// and syncs client files to the headless install directory.
///
/// - `Mod(id)`: after a single mod/addon operation in ops.rs
/// - `Full`: during convergence
#[allow(deprecated)]
pub fn sync_headless(
    db: &Database,
    config: &Config,
    dirs: &QumaDirs,
    scope: HeadlessSyncScope,
) -> Result<()> {
    let install_dir = match config.headless.as_ref().map(|h| &h.install_dir) {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => return Ok(()),
    };

    let mut active_files: Vec<String> = Vec::new();
    let mut excluded_files: Vec<String> = Vec::new();

    let mods = match &scope {
        HeadlessSyncScope::Mod(mod_id) => match db.get_mod(*mod_id)? {
            Some(m) => vec![m],
            None => return Ok(()),
        },
        HeadlessSyncScope::Full => db.list_mods()?,
    };

    for m in &mods {
        let mod_excluded = m.disabled || crate::ops::is_excluded_from_headless(db, m.id);

        let files = db.get_files_for_mod(m.id)?;
        for f in files {
            if !is_client_file(&f.file_path) {
                continue;
            }
            if mod_excluded {
                excluded_files.push(f.file_path);
            } else {
                active_files.push(f.file_path);
            }
        }

        let addons = db.list_addons_for_mod(m.id)?;
        for addon in &addons {
            let addon_excluded = mod_excluded || addon.disabled;
            let addon_files = db.get_files_for_addon(addon.id)?;
            for f in addon_files {
                if !is_client_file(&f.file_path) {
                    continue;
                }
                if addon_excluded {
                    excluded_files.push(f.file_path);
                } else {
                    active_files.push(f.file_path);
                }
            }
        }
    }

    // Remove excluded files first
    if !excluded_files.is_empty() {
        let report = sync_client_files_to_headless(
            &dirs.spt_server,
            install_dir,
            &excluded_files,
            SyncOp::Remove,
        )?;
        if report.removed > 0 {
            tracing::info!(
                removed = report.removed,
                errors = report.errors,
                "Removed {} excluded/disabled mod files from headless",
                report.removed
            );
        }
    }

    // Copy active files
    if !active_files.is_empty() {
        let report = sync_client_files_to_headless(
            &dirs.spt_server,
            install_dir,
            &active_files,
            SyncOp::Install,
        )?;
        if report.copied > 0 {
            tracing::info!(
                copied = report.copied,
                errors = report.errors,
                "Synced {} client-side mod files to headless",
                report.copied
            );
        }
    }

    Ok(())
}

pub fn sync_client_files_to_headless(
    spt_dir: &Path,
    install_dir: &Path,
    client_files: &[String],
    op: SyncOp,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();

    for file_path in client_files {
        if !is_client_file(file_path) {
            continue;
        }

        match op {
            SyncOp::Install => {
                let src = spt_dir.join(file_path);
                let dst = install_dir.join(file_path);
                if let Some(parent) = dst.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        tracing::warn!(path = %file_path, err = %e, "headless sync: failed to create dir");
                        report.errors += 1;
                        continue;
                    }
                }
                match std::fs::copy(&src, &dst) {
                    Ok(_) => report.copied += 1,
                    Err(e) => {
                        tracing::warn!(path = %file_path, err = %e, "headless sync: failed to copy file");
                        report.errors += 1;
                    }
                }
            }
            SyncOp::Remove => {
                let dst = install_dir.join(file_path);
                if !dst.exists() {
                    continue;
                }
                match std::fs::remove_file(&dst) {
                    Ok(()) => {
                        report.removed += 1;
                        cleanup_empty_parents(install_dir, file_path);
                    }
                    Err(e) => {
                        tracing::warn!(path = %file_path, err = %e, "headless sync: failed to remove file");
                        report.errors += 1;
                    }
                }
            }
        }
    }

    if report.copied > 0 || report.removed > 0 {
        tracing::info!(
            copied = report.copied,
            removed = report.removed,
            errors = report.errors,
            "headless sync complete"
        );
    }

    Ok(report)
}

/// Walk up from the file's parent directory, removing empty dirs.
/// Stops before removing top-level BepInEx subdirs (plugins, patchers, etc).
fn cleanup_empty_parents(install_dir: &Path, file_path: &str) {
    let full = install_dir.join(file_path);
    let bepinex = install_dir.join("BepInEx");
    let mut dir = match full.parent() {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    while dir.starts_with(&bepinex) {
        // Stop if we've reached a top-level BepInEx subdir (BepInEx/plugins, BepInEx/patchers, etc)
        if dir.parent() == Some(&bepinex) {
            break;
        }
        match std::fs::read_dir(&dir) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    break;
                }
                let _ = std::fs::remove_dir(&dir);
            }
            Err(_) => break,
        }
        dir = match dir.parent() {
            Some(p) => p.to_path_buf(),
            None => break,
        };
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn is_client_file_filters_correctly() {
        // Client files — should sync
        assert!(is_client_file("BepInEx/plugins/SAIN/SAIN.dll"));
        assert!(is_client_file("BepInEx/plugins/loose.dll"));
        assert!(is_client_file("BepInEx/patchers/something.dll"));

        // Server-only — should NOT sync
        assert!(!is_client_file("SPT/user/mods/MyMod/package.json"));

        // Config — should NOT sync (per-client overlay)
        assert!(!is_client_file("BepInEx/config/com.fika.core.cfg"));

        // Fika files — NOW synced normally (no longer managed separately)
        assert!(is_client_file("BepInEx/plugins/Fika/Fika.Core.dll"));
        assert!(is_client_file(
            "BepInEx/plugins/Fika.Headless/Fika.Headless.dll"
        ));
    }

    #[test]
    fn install_copies_files_to_headless() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let plugin_dir = spt.path().join("BepInEx/plugins/TestMod");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("test.dll"), b"dll-content").unwrap();
        fs::write(plugin_dir.join("config.json"), b"{}").unwrap();

        let files = vec![
            "BepInEx/plugins/TestMod/test.dll".to_string(),
            "BepInEx/plugins/TestMod/config.json".to_string(),
        ];

        let report =
            sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Install)
                .unwrap();

        assert_eq!(report.copied, 2);
        assert_eq!(report.errors, 0);
        assert!(headless
            .path()
            .join("BepInEx/plugins/TestMod/test.dll")
            .exists());
        assert_eq!(
            fs::read(headless.path().join("BepInEx/plugins/TestMod/test.dll")).unwrap(),
            b"dll-content"
        );
    }

    #[test]
    fn remove_deletes_files_and_cleans_empty_dirs() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let plugin_dir = headless.path().join("BepInEx/plugins/TestMod");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("test.dll"), b"dll-content").unwrap();

        let files = vec!["BepInEx/plugins/TestMod/test.dll".to_string()];

        let report =
            sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Remove)
                .unwrap();

        assert_eq!(report.removed, 1);
        assert!(!headless
            .path()
            .join("BepInEx/plugins/TestMod/test.dll")
            .exists());
        assert!(!headless.path().join("BepInEx/plugins/TestMod").exists());
    }

    #[test]
    fn remove_preserves_bepinex_plugins_dir() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let plugin_dir = headless.path().join("BepInEx/plugins");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("loose.dll"), b"content").unwrap();

        let files = vec!["BepInEx/plugins/loose.dll".to_string()];
        sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Remove).unwrap();

        assert!(!headless.path().join("BepInEx/plugins/loose.dll").exists());
        assert!(headless.path().join("BepInEx/plugins").exists());
    }

    #[test]
    fn install_skips_server_and_config_files() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        // Server mod
        let server_dir = spt.path().join("SPT/user/mods/TestMod");
        fs::create_dir_all(&server_dir).unwrap();
        fs::write(server_dir.join("package.json"), b"{}").unwrap();

        // BepInEx config
        let config_dir = spt.path().join("BepInEx/config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("plugin.cfg"), b"cfg").unwrap();

        let files = vec![
            "SPT/user/mods/TestMod/package.json".to_string(),
            "BepInEx/config/plugin.cfg".to_string(),
        ];

        let report =
            sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Install)
                .unwrap();

        assert_eq!(report.copied, 0);
    }

    #[test]
    fn install_syncs_fika_files() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let fika_dir = spt.path().join("BepInEx/plugins/Fika");
        fs::create_dir_all(&fika_dir).unwrap();
        fs::write(fika_dir.join("Fika.Core.dll"), b"fika").unwrap();

        let files = vec!["BepInEx/plugins/Fika/Fika.Core.dll".to_string()];

        let report =
            sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Install)
                .unwrap();

        assert_eq!(report.copied, 1);
        assert!(headless
            .path()
            .join("BepInEx/plugins/Fika/Fika.Core.dll")
            .exists());
    }

    #[test]
    fn install_overwrites_existing_files() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        fs::create_dir_all(spt.path().join("BepInEx/plugins")).unwrap();
        fs::write(spt.path().join("BepInEx/plugins/mod.dll"), b"v2").unwrap();

        fs::create_dir_all(headless.path().join("BepInEx/plugins")).unwrap();
        fs::write(headless.path().join("BepInEx/plugins/mod.dll"), b"v1").unwrap();

        let files = vec!["BepInEx/plugins/mod.dll".to_string()];
        sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Install)
            .unwrap();

        assert_eq!(
            fs::read(headless.path().join("BepInEx/plugins/mod.dll")).unwrap(),
            b"v2"
        );
    }

    #[test]
    fn remove_ignores_missing_files() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();
        fs::create_dir_all(headless.path().join("BepInEx/plugins")).unwrap();

        let files = vec!["BepInEx/plugins/nonexistent.dll".to_string()];

        let report =
            sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Remove)
                .unwrap();

        assert_eq!(report.removed, 0);
        assert_eq!(report.errors, 0);
    }

    #[test]
    fn install_syncs_patchers() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let patcher_dir = spt.path().join("BepInEx/patchers/MyPatcher");
        fs::create_dir_all(&patcher_dir).unwrap();
        fs::write(patcher_dir.join("patcher.dll"), b"patcher").unwrap();

        let files = vec!["BepInEx/patchers/MyPatcher/patcher.dll".to_string()];

        let report =
            sync_client_files_to_headless(spt.path(), headless.path(), &files, SyncOp::Install)
                .unwrap();

        assert_eq!(report.copied, 1);
        assert!(headless
            .path()
            .join("BepInEx/patchers/MyPatcher/patcher.dll")
            .exists());
    }

    #[test]
    fn sync_headless_scoped_syncs_mod_and_addon_files() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();

        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/TestMod/mod.dll",
            Some("aaa"),
            Some(100),
        )
        .unwrap();

        let addon_id = db
            .insert_addon(100, mod_id, 300, "TestAddon", None, "1.0.0", None)
            .unwrap();
        db.insert_addon_file(
            addon_id,
            "BepInEx/plugins/TestAddon/addon.dll",
            Some("bbb"),
            Some(50),
        )
        .unwrap();

        // Create source files on disk
        let mod_dir = spt.path().join("BepInEx/plugins/TestMod");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("mod.dll"), b"mod-content").unwrap();

        let addon_dir = spt.path().join("BepInEx/plugins/TestAddon");
        fs::create_dir_all(&addon_dir).unwrap();
        fs::write(addon_dir.join("addon.dll"), b"addon-content").unwrap();

        let dirs = crate::dirs::QumaDirs::from_legacy(spt.path().to_path_buf());
        let mut config = crate::config::Config::default();
        config.headless = Some(crate::config::HeadlessConfig {
            install_dir: headless.path().to_path_buf(),
            ..Default::default()
        });

        sync_headless(&db, &config, &dirs, HeadlessSyncScope::Mod(mod_id)).unwrap();

        assert!(headless
            .path()
            .join("BepInEx/plugins/TestMod/mod.dll")
            .exists());
        assert_eq!(
            fs::read(headless.path().join("BepInEx/plugins/TestMod/mod.dll")).unwrap(),
            b"mod-content"
        );
        assert!(headless
            .path()
            .join("BepInEx/plugins/TestAddon/addon.dll")
            .exists());
        assert_eq!(
            fs::read(headless.path().join("BepInEx/plugins/TestAddon/addon.dll")).unwrap(),
            b"addon-content"
        );
    }

    #[test]
    fn sync_headless_scoped_removes_disabled_addon_files() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();

        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();

        let addon_id = db
            .insert_addon(100, mod_id, 300, "DisabledAddon", None, "1.0.0", None)
            .unwrap();
        db.set_addon_disabled(addon_id, true).unwrap();
        db.insert_addon_file(
            addon_id,
            "BepInEx/plugins/DisabledAddon/addon.dll",
            Some("ccc"),
            Some(100),
        )
        .unwrap();

        // Pre-populate headless with the addon file
        let headless_plugin = headless.path().join("BepInEx/plugins/DisabledAddon");
        fs::create_dir_all(&headless_plugin).unwrap();
        fs::write(headless_plugin.join("addon.dll"), b"old").unwrap();

        let dirs = crate::dirs::QumaDirs::from_legacy(spt.path().to_path_buf());
        let mut config = crate::config::Config::default();
        config.headless = Some(crate::config::HeadlessConfig {
            install_dir: headless.path().to_path_buf(),
            ..Default::default()
        });

        sync_headless(&db, &config, &dirs, HeadlessSyncScope::Mod(mod_id)).unwrap();

        assert!(
            !headless
                .path()
                .join("BepInEx/plugins/DisabledAddon/addon.dll")
                .exists(),
            "disabled addon file should be removed from headless"
        );
    }

    #[test]
    fn sync_headless_full_syncs_all_mods_and_addons() {
        let spt = tempfile::tempdir().unwrap();
        let headless = tempfile::tempdir().unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();

        // Mod A with addon
        let mod_a = db
            .insert_mod(Some(100), Some(200), "ModA", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_file(mod_a, "BepInEx/plugins/ModA/a.dll", Some("aaa"), Some(100))
            .unwrap();
        let addon_a = db
            .insert_addon(100, mod_a, 300, "AddonA", None, "1.0.0", None)
            .unwrap();
        db.insert_addon_file(
            addon_a,
            "BepInEx/plugins/AddonA/aa.dll",
            Some("bbb"),
            Some(50),
        )
        .unwrap();

        // Mod B (disabled) with file already in headless
        let mod_b = db
            .insert_mod(Some(101), Some(201), "ModB", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_file(mod_b, "BepInEx/plugins/ModB/b.dll", Some("ccc"), Some(100))
            .unwrap();
        db.set_mod_disabled(mod_b, true).unwrap();

        // Create source files
        for (dir, file, content) in [
            ("BepInEx/plugins/ModA", "a.dll", b"mod-a" as &[u8]),
            ("BepInEx/plugins/AddonA", "aa.dll", b"addon-a"),
        ] {
            let d = spt.path().join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join(file), content).unwrap();
        }

        // Pre-populate headless with disabled mod's file
        let disabled_dir = headless.path().join("BepInEx/plugins/ModB");
        fs::create_dir_all(&disabled_dir).unwrap();
        fs::write(disabled_dir.join("b.dll"), b"stale").unwrap();

        let dirs = crate::dirs::QumaDirs::from_legacy(spt.path().to_path_buf());
        let mut config = crate::config::Config::default();
        config.headless = Some(crate::config::HeadlessConfig {
            install_dir: headless.path().to_path_buf(),
            ..Default::default()
        });

        sync_headless(&db, &config, &dirs, HeadlessSyncScope::Full).unwrap();

        // Active mod + addon synced
        assert!(headless.path().join("BepInEx/plugins/ModA/a.dll").exists());
        assert!(headless
            .path()
            .join("BepInEx/plugins/AddonA/aa.dll")
            .exists());
        // Disabled mod's file removed
        assert!(!headless.path().join("BepInEx/plugins/ModB/b.dll").exists());
    }
}
