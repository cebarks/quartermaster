use std::path::Path;

use crate::config::Config;
use crate::db::Database;

/// One-time migration: moves NarcoNet modsync groups from config TOML into
/// the mod_groups DB table, assigns group_id on installed_mods, and
/// un-layouts quma-<slug> directories.
///
/// Safe to call multiple times — skips if mod_groups table already has rows.
pub fn migrate_modsync_to_convoy(
    config: &Config,
    db: &Database,
    spt_dir: &Path,
) -> anyhow::Result<bool> {
    // Check if migration already ran (groups exist in DB)
    let existing = db.list_groups()?;
    if !existing.is_empty() {
        return Ok(false);
    }

    // Un-layout first: move files out of quma-<slug> directories
    // regardless of whether there are groups to migrate
    un_layout_group_directories(db, spt_dir)?;

    let modsync = match &config.modsync {
        Some(ms) => ms,
        None => return Ok(false),
    };

    if modsync.groups.is_empty() {
        return Ok(false);
    }

    let mut migrated = false;

    for (slug, group) in &modsync.groups {
        // Drop disabled groups — their mods become ungrouped (default required)
        if !group.enabled.unwrap_or(true) {
            tracing::info!(
                "dropping disabled modsync group '{}' during convoy migration — \
                 member mods will be ungrouped (required)",
                slug
            );
            continue;
        }

        let group_id = db.insert_group(
            &group.display_name,
            slug,
            "required",
            group.exclude_headless,
        )?;

        for &forge_id in &group.members {
            if let Some(m) = db.get_mod_by_forge_id(forge_id)? {
                db.set_mod_group(m.id, Some(group_id))?;
            }
        }

        migrated = true;
    }

    if migrated {
        tracing::info!("migrated modsync groups to convoy DB tables");
    }

    Ok(migrated)
}

/// Reverses the quma-<slug> directory layout. Moves files from
/// BepInEx/plugins/quma-<slug>/ModName/file.dll back to
/// BepInEx/plugins/ModName/file.dll and updates DB paths.
fn un_layout_group_directories(db: &Database, spt_dir: &Path) -> anyhow::Result<()> {
    let plugins_dir = spt_dir.join("BepInEx/plugins");
    if !plugins_dir.exists() {
        return Ok(());
    }

    // Collect quma-* dirs first
    let quma_dirs: Vec<_> = std::fs::read_dir(&plugins_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("quma-") && e.file_type().map_or(false, |t| t.is_dir())
        })
        .collect();

    for entry in quma_dirs {
        let name = entry.file_name().to_string_lossy().to_string();
        let old_prefix = format!("BepInEx/plugins/{}/", name);
        let new_prefix = "BepInEx/plugins/";

        // Query tracked files once per quma-* dir (not per child)
        let files = db.get_all_tracked_files()?;
        let affected: Vec<_> = files
            .iter()
            .filter(|f| f.file_path.starts_with(&old_prefix))
            .collect();

        // Move all contents up one level
        for child in std::fs::read_dir(entry.path())? {
            let child = child?;
            let child_name = child.file_name();
            let dest = plugins_dir.join(&child_name);

            if dest.exists() {
                tracing::warn!(
                    "convoy migration: cannot move {:?} — destination already exists",
                    dest
                );
                continue;
            }

            std::fs::rename(child.path(), &dest)?;
        }

        // Update DB paths
        for file in &affected {
            let new_path = format!("{}{}", new_prefix, &file.file_path[old_prefix.len()..]);
            db.rename_file_path(file.id, &new_path)?;
        }

        // Remove empty quma-* dir
        // ponytail: inline is_dir_empty
        let is_empty = entry
            .path()
            .read_dir()
            .map_or(true, |mut d| d.next().is_none());
        if is_empty {
            std::fs::remove_dir(entry.path()).ok();
        }

        tracing::info!("un-layouted convoy group directory: {}", name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_migrate_modsync_to_convoy() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let spt_dir = tmp.path();

        // Set up directory structure
        let plugins = spt_dir.join("BepInEx/plugins");
        std::fs::create_dir_all(&plugins).unwrap();

        // Create quma-test-group/ModOne/plugin.dll
        let quma_dir = plugins.join("quma-test-group");
        let mod_dir = quma_dir.join("ModOne");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("plugin.dll"), b"fake dll").unwrap();

        // Create quma-disabled/ModTwo/plugin.dll
        let quma_disabled = plugins.join("quma-disabled");
        let mod_two_dir = quma_disabled.join("ModTwo");
        std::fs::create_dir_all(&mod_two_dir).unwrap();
        std::fs::write(mod_two_dir.join("plugin.dll"), b"fake dll 2").unwrap();

        // Set up database
        let db_path = spt_dir.join("test.db");
        let db = Database::open(&db_path).unwrap();

        // Insert test mods
        db.insert_mod(
            Some(101),
            None,
            "ModOne",
            Some("mod-one"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
        let mod_one_id = db.get_mod_by_forge_id(101).unwrap().unwrap().id;

        db.insert_mod(
            Some(102),
            None,
            "ModTwo",
            Some("mod-two"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
        let mod_two_id = db.get_mod_by_forge_id(102).unwrap().unwrap().id;

        // Track files with quma-* paths
        db.insert_file(
            mod_one_id,
            "BepInEx/plugins/quma-test-group/ModOne/plugin.dll",
            Some("abc123"),
            None,
        )
        .unwrap();
        db.insert_file(
            mod_two_id,
            "BepInEx/plugins/quma-disabled/ModTwo/plugin.dll",
            Some("def456"),
            None,
        )
        .unwrap();

        // Set up config with modsync groups
        let mut config = Config::default();
        let mut groups = BTreeMap::new();
        groups.insert(
            "test-group".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Test Group".to_string(),
                members: vec![101],
                enabled: Some(true),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );
        groups.insert(
            "disabled".to_string(),
            crate::config::ModSyncGroup {
                display_name: "Disabled Group".to_string(),
                members: vec![102],
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );
        config.modsync = Some(crate::config::ModSyncConfig {
            enabled: true,
            enforced: false,
            silent: false,
            restart_required: false,
            extra_sync_paths: vec![],
            exclusions: vec![],
            overrides: BTreeMap::new(),
            groups,
        });

        // Run migration
        let result = migrate_modsync_to_convoy(&config, &db, spt_dir).unwrap();
        assert!(result, "migration should return true");

        // Check groups table
        let groups = db.list_groups().unwrap();
        assert_eq!(groups.len(), 1, "only enabled group should be migrated");
        assert_eq!(groups[0].name, "Test Group");
        assert_eq!(groups[0].slug, "test-group");
        assert_eq!(groups[0].tier, "required");

        // Check mod group assignments
        let mod_one = db.get_mod_by_forge_id(101).unwrap().unwrap();
        assert_eq!(mod_one.group_id, Some(groups[0].id));

        let mod_two = db.get_mod_by_forge_id(102).unwrap().unwrap();
        assert_eq!(
            mod_two.group_id, None,
            "disabled group's mod should be ungrouped"
        );

        // Check filesystem un-layout
        assert!(
            !quma_dir.exists() || quma_dir.read_dir().unwrap().next().is_none(),
            "quma-test-group should be removed or empty"
        );
        assert!(
            plugins.join("ModOne/plugin.dll").exists(),
            "ModOne should be moved to BepInEx/plugins"
        );

        // Check DB paths updated
        let files = db.get_all_tracked_files().unwrap();
        let mod_one_file = files.iter().find(|f| f.mod_id == Some(mod_one_id)).unwrap();
        assert_eq!(mod_one_file.file_path, "BepInEx/plugins/ModOne/plugin.dll");

        // Second run should be idempotent
        let result2 = migrate_modsync_to_convoy(&config, &db, spt_dir).unwrap();
        assert!(!result2, "second migration should return false");
    }
}
