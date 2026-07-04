use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::RngExt;
use serde::{Deserialize, Serialize};

use crate::db::mods::InstalledFile;

/// Manifest entry for a single mod in a full backup.
#[derive(Debug, Serialize, Deserialize)]
struct ManifestMod {
    forge_mod_id: i64,
    forge_version_id: i64,
    name: String,
    slug: Option<String>,
    version: String,
    file_paths: Vec<String>,
}

/// Manifest entry for a single addon in a full backup.
#[derive(Debug, Serialize, Deserialize)]
struct ManifestAddon {
    forge_addon_id: i64,
    parent_forge_mod_id: i64,
    forge_version_id: i64,
    name: String,
    slug: Option<String>,
    version: String,
    mod_version_constraint: Option<String>,
    file_paths: Vec<String>,
}

/// Manifest written into every full backup, recording which mods and files
/// existed at backup time so we can reconstruct the DB state on restore.
#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    mods: Vec<ManifestMod>,
    #[serde(default)]
    addons: Vec<ManifestAddon>,
}

/// Generate a backup ID from the current UTC timestamp.
/// Format: `YYYYMMDDTHHMMSS` (15 characters).
pub fn generate_backup_id() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

/// Generate a unique backup ID within `base_dir`, appending a numeric suffix
/// if the timestamp-based directory already exists.
fn unique_backup_id(base_dir: &Path) -> String {
    let id = generate_backup_id();
    if !base_dir.join(&id).exists() {
        return id;
    }
    for suffix in 1..100 {
        let candidate = format!("{id}_{suffix}");
        if !base_dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{id}_{}", rand::rng().random::<u16>())
}

/// Resolve the backup directory path from the SPT dir and config.
///
/// Validates that the resolved path is within `spt_dir` to prevent path
/// traversal via crafted `backup_dir` config values.
pub fn resolve_backup_dir(spt_dir: &Path, config: &crate::config::Config) -> Result<PathBuf> {
    let backup_dir = &config.backup.backup_dir;
    if Path::new(backup_dir).is_absolute() {
        anyhow::bail!("backup_dir must be a relative path (got absolute: {backup_dir})");
    }
    for component in Path::new(backup_dir).components() {
        if matches!(component, std::path::Component::ParentDir) {
            anyhow::bail!("backup_dir must not contain '..' components");
        }
    }
    Ok(spt_dir.join(backup_dir))
}

/// Copy a list of tracked files from `base_dir` into a backup `dest` directory,
/// preserving relative paths. Returns the total size of successfully copied files.
fn copy_files_to_backup(files: &[InstalledFile], base_dir: &Path, dest: &Path) -> Result<i64> {
    let mut total_size: i64 = 0;
    for file in files {
        let src = base_dir.join(&file.file_path);
        let dst = dest.join(&file.file_path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if src.exists() {
            std::fs::copy(&src, &dst)
                .with_context(|| format!("failed to copy {} to backup", src.display()))?;
            total_size += file.file_size.unwrap_or(0);
        } else {
            tracing::warn!(path = %file.file_path, "backup: source file missing, skipping");
        }
    }
    Ok(total_size)
}

/// Copy a list of tracked files into a backup directory under a `subdir` prefix,
/// collecting the relative paths of files that were actually copied.
/// Returns `(total_size, copied_file_paths)`.
fn copy_files_for_manifest(
    files: &[InstalledFile],
    base_dir: &Path,
    dest_subdir: &Path,
) -> Result<(i64, Vec<String>)> {
    let mut total_size: i64 = 0;
    let mut file_paths: Vec<String> = Vec::new();
    for file in files {
        let src = base_dir.join(&file.file_path);
        let dst = dest_subdir.join(&file.file_path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if src.exists() {
            std::fs::copy(&src, &dst)?;
            total_size += file.file_size.unwrap_or(0);
            file_paths.push(file.file_path.clone());
        }
    }
    Ok((total_size, file_paths))
}

/// Restore files from a backup manifest entry into `spt_dir`, computing hashes
/// and recording each file via the provided `record_file` callback.
fn restore_manifest_files(
    rel_paths: &[String],
    src_dir: &Path,
    spt_dir: &Path,
    mut record_file: impl FnMut(&str, &str, i64) -> Result<()>,
) -> Result<()> {
    for rel_path in rel_paths {
        let src = src_dir.join(rel_path);
        if !src.exists() {
            tracing::warn!(path = %rel_path, "manifest file missing from backup, skipping");
            continue;
        }
        let dst = spt_dir.join(rel_path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&src, &dst)?;

        let content = std::fs::read(&dst)?;
        let hash = crate::spt::mods::compute_hash_public(&content);
        let size = content.len() as i64;
        record_file(rel_path, &hash, size)?;
    }
    Ok(())
}

/// Back up a single mod's files to the backup directory.
///
/// Copies all files tracked in the database for the given mod, records the backup
/// in the DB, and enforces per-mod retention limits.
///
/// Returns the database ID of the new backup record.
pub fn backup_mod(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
    trigger: &str,
) -> Result<i64> {
    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod {mod_db_id} not found"))?;

    let files = db.get_files_for_mod(mod_db_id)?;

    // Also get addon files for this mod
    let addons = db.list_addons_for_mod(mod_db_id)?;

    let backup_dir = resolve_backup_dir(spt_dir, config)?;
    let mod_backup_dir = backup_dir
        .join("mods")
        .join(mod_info.forge_mod_id.to_string());
    std::fs::create_dir_all(&mod_backup_dir)
        .with_context(|| format!("failed to create backup dir: {}", mod_backup_dir.display()))?;

    let bid = unique_backup_id(&mod_backup_dir);
    let dest = mod_backup_dir.join(&bid);

    let base_dir = crate::ops::resolve_mod_root(spt_dir, mod_info.disabled);
    let mut total_size = copy_files_to_backup(&files, &base_dir, &dest)?;
    // For addon files, only use stash if the addon itself is disabled.
    // Disabling a parent mod does NOT move addon files — they stay at
    // canonical paths unless the addon is independently disabled.
    for addon in &addons {
        let addon_files = db.get_files_for_addon(addon.id)?;
        let addon_base = crate::ops::resolve_mod_root(spt_dir, addon.disabled);
        total_size += copy_files_to_backup(&addon_files, &addon_base, &dest)?;
    }

    let backup_path = format!(
        "{}/mods/{}/{}",
        config.backup.backup_dir, mod_info.forge_mod_id, bid
    );

    let backup_db_id = db.insert_backup(
        "mod",
        trigger,
        &bid,
        Some(mod_db_id),
        Some(mod_info.forge_mod_id),
        Some(mod_info.forge_version_id),
        Some(&mod_info.name),
        mod_info.slug.as_deref(),
        Some(&mod_info.version),
        &backup_path,
        Some(total_size),
    )?;

    let addon_file_count: usize = addons
        .iter()
        .map(|a| db.get_files_for_addon(a.id).unwrap_or_default().len())
        .sum();
    tracing::info!(
        mod_db_id,
        backup_id = %bid,
        file_count = files.len() + addon_file_count,
        total_size,
        "mod backed up"
    );

    enforce_retention_mod(db, spt_dir, config, mod_info.forge_mod_id)?;

    Ok(backup_db_id)
}

/// Back up all installed mods, addons, player profiles, and the Quartermaster config file.
///
/// Creates a single "full" backup containing:
/// 1. All files for every installed mod (under `mods/`)
/// 2. All files for every installed addon (under `mods/`)
/// 3. All `.json` profile files from `user/profiles/` (under `profiles/`)
/// 4. The `quartermaster.toml` config file
///
/// Returns the database ID of the new backup record.
pub fn backup_full(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
) -> Result<i64> {
    let backup_dir = resolve_backup_dir(spt_dir, config)?;
    let full_dir = backup_dir.join("full");
    std::fs::create_dir_all(&full_dir)?;
    let bid = unique_backup_id(&full_dir);
    let dest = full_dir.join(&bid);
    std::fs::create_dir_all(&dest)?;

    let mut total_size: i64 = 0;
    let mut manifest_mods: Vec<ManifestMod> = Vec::new();
    let mut manifest_addons: Vec<ManifestAddon> = Vec::new();

    // 1. Copy all mod files and build manifest
    let mods_dest = dest.join("mods");
    let mods = db.list_mods()?;
    for m in &mods {
        let files = db.get_files_for_mod(m.id)?;
        let base = crate::ops::resolve_mod_root(spt_dir, m.disabled);
        let (size, file_paths) = copy_files_for_manifest(&files, &base, &mods_dest)?;
        total_size += size;
        manifest_mods.push(ManifestMod {
            forge_mod_id: m.forge_mod_id,
            forge_version_id: m.forge_version_id,
            name: m.name.clone(),
            slug: m.slug.clone(),
            version: m.version.clone(),
            file_paths,
        });
    }

    // 2. Copy all addon files and build manifest
    let all_addons = db.list_addons()?;
    for addon in &all_addons {
        // Look up the parent mod to get its forge_mod_id
        let parent_mod = db
            .get_mod(addon.parent_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("addon parent mod {} not found", addon.parent_mod_id))?;

        let addon_files = db.get_files_for_addon(addon.id)?;
        let addon_base = crate::ops::resolve_mod_root(spt_dir, addon.disabled);
        let (size, file_paths) = copy_files_for_manifest(&addon_files, &addon_base, &mods_dest)?;
        total_size += size;
        manifest_addons.push(ManifestAddon {
            forge_addon_id: addon.forge_addon_id,
            parent_forge_mod_id: parent_mod.forge_mod_id,
            forge_version_id: addon.forge_version_id,
            name: addon.name.clone(),
            slug: addon.slug.clone(),
            version: addon.version.clone(),
            mod_version_constraint: addon.mod_version_constraint.clone(),
            file_paths,
        });
    }

    // Write manifest so restore can reconstruct DB state for removed mods/addons
    let manifest = BackupManifest {
        mods: manifest_mods,
        addons: manifest_addons,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("failed to serialize backup manifest")?;
    std::fs::write(dest.join("manifest.json"), manifest_json)
        .context("failed to write backup manifest")?;

    // 3. Copy profiles
    let profiles_src = spt_dir.join("user/profiles");
    if profiles_src.is_dir() {
        let profiles_dst = dest.join("profiles");
        std::fs::create_dir_all(&profiles_dst)?;
        for entry in std::fs::read_dir(&profiles_src)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let dst = profiles_dst.join(entry.file_name());
                std::fs::copy(&path, &dst)?;
                total_size += std::fs::metadata(&path)
                    .map(|m| m.len() as i64)
                    .unwrap_or(0);
            }
        }
    }

    // 4. Copy config
    let config_src = spt_dir.join("quartermaster.toml");
    if config_src.exists() {
        std::fs::copy(&config_src, dest.join("quartermaster.toml"))?;
    }

    let backup_path = format!("{}/full/{}", config.backup.backup_dir, bid);
    let backup_db_id = db.insert_backup(
        "full",
        "manual",
        &bid,
        None,
        None,
        None,
        None,
        None,
        None,
        &backup_path,
        Some(total_size),
    )?;

    tracing::info!(backup_id = %bid, mod_count = mods.len(), total_size, "full backup created");

    enforce_retention_full(db, spt_dir, config)?;

    Ok(backup_db_id)
}

/// Hook for automatic mod backup before updates/removals.
///
/// When `config.backup.auto_backup` is true, attempts to back up the mod.
/// If the backup fails and `config.backup.require_backup` is true, the error
/// propagates (blocking the operation). Otherwise, the error is logged and
/// swallowed so the operation can proceed.
pub fn auto_backup_mod(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
    trigger: &str,
) -> Result<()> {
    if !config.backup.auto_backup {
        return Ok(());
    }
    match backup_mod(db, spt_dir, config, mod_db_id, trigger) {
        Ok(_) => Ok(()),
        Err(e) => {
            if config.backup.require_backup {
                Err(e).with_context(|| "auto-backup failed and require_backup is enabled")
            } else {
                tracing::warn!(mod_db_id, err = %e, "auto-backup failed, continuing");
                Ok(())
            }
        }
    }
}

/// Restore a single mod from a mod-type backup.
///
/// Looks up the backup record, locates the backup files on disk, deletes the
/// mod's current files (if the mod still exists), copies the backup files back
/// into the SPT directory, re-records them in the database, and marks the
/// backup as restored.
///
/// If the mod was previously removed, re-inserts it from the backup metadata.
pub fn restore_mod_backup(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    backup_db_id: i64,
) -> Result<()> {
    let backup = db
        .get_backup(backup_db_id)?
        .ok_or_else(|| anyhow::anyhow!("backup {backup_db_id} not found"))?;

    if backup.backup_type != "mod" {
        anyhow::bail!("backup {} is a full backup, not a mod backup", backup_db_id);
    }

    let forge_mod_id = backup
        .forge_mod_id
        .ok_or_else(|| anyhow::anyhow!("mod backup missing forge_mod_id"))?;

    let backup_files_dir = spt_dir.join(&backup.backup_path);
    if !backup_files_dir.exists() {
        anyhow::bail!("backup directory missing: {}", backup_files_dir.display());
    }

    // Wrap all DB mutations in a single transaction
    let tx = db.begin_transaction()?;

    // Find or re-create the mod row
    let mod_db_id = match db.get_mod_by_forge_id(forge_mod_id)? {
        Some(existing) => {
            // Delete current files from disk and DB
            let current_files = db.get_files_for_mod(existing.id)?;
            let paths: Vec<String> = current_files.into_iter().map(|f| f.file_path).collect();
            let delete_root = crate::ops::resolve_mod_root(spt_dir, existing.disabled);
            crate::spt::mods::delete_mod_files(&delete_root, &paths)?;
            db.delete_files_for_mod(existing.id)?;
            existing.id
        }
        None => {
            // Re-insert mod from backup metadata
            let name = backup
                .mod_name
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("backup missing mod_name"))?;
            let version_id = backup
                .forge_version_id
                .ok_or_else(|| anyhow::anyhow!("backup missing forge_version_id"))?;
            let version = backup
                .mod_version
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("backup missing mod_version"))?;
            db.insert_mod(
                forge_mod_id,
                version_id,
                name,
                backup.mod_slug.as_deref(),
                version,
            )?
        }
    };

    // Copy backup files to spt_dir and record them
    let mut restored_count = 0u64;
    copy_backup_tree_and_record(
        db,
        spt_dir,
        &backup_files_dir,
        &backup_files_dir,
        mod_db_id,
        &mut restored_count,
    )?;

    // Update mod version to the backup's version
    if let (Some(version_id), Some(version)) =
        (backup.forge_version_id, backup.mod_version.as_deref())
    {
        db.update_mod(mod_db_id, version_id, version)?;
    }

    // Restored files are placed at canonical paths (enabled state)
    db.set_mod_disabled(mod_db_id, false)?;

    db.set_backup_restored(backup_db_id)?;
    tx.commit()?;

    tracing::info!(
        backup_db_id,
        mod_db_id,
        restored_count,
        "mod restored from backup"
    );

    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config after restore");
    }

    if let Some(ref ms_config) = config.modsync {
        if let Err(e) = crate::modsync::ensure_mod_layout(spt_dir, ms_config, db, mod_db_id) {
            tracing::warn!(err = %e, "failed to ensure mod layout after restore");
        }
    }

    Ok(())
}

/// Restore all mods, profiles, and config from a full backup.
///
/// Uses the backup manifest to reconstruct the DB state, including mods that
/// were removed after the backup was created. Deletes all current mod files,
/// copies everything back from the backup directory, and re-records file
/// metadata. Also restores player profiles and the `quartermaster.toml`
/// config file if present in the backup.
pub fn restore_full_backup(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    backup_db_id: i64,
) -> Result<()> {
    let backup = db
        .get_backup(backup_db_id)?
        .ok_or_else(|| anyhow::anyhow!("backup {backup_db_id} not found"))?;

    if backup.backup_type != "full" {
        anyhow::bail!("backup {} is not a full backup", backup_db_id);
    }

    let backup_dir = spt_dir.join(&backup.backup_path);
    if !backup_dir.exists() {
        anyhow::bail!("backup directory missing: {}", backup_dir.display());
    }

    // Wrap all DB mutations in a transaction
    let tx = db.begin_transaction()?;

    // Restore mod files
    let mods_dir = backup_dir.join("mods");
    if mods_dir.is_dir() {
        // Delete all current mod files and DB records
        let all_mods = db.list_mods()?;
        for m in &all_mods {
            let files = db.get_files_for_mod(m.id)?;
            let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
            let delete_root = crate::ops::resolve_mod_root(spt_dir, m.disabled);
            crate::spt::mods::delete_mod_files(&delete_root, &paths)?;
            db.delete_files_for_mod(m.id)?;
            db.delete_mod(m.id)?;
        }

        // Read manifest to know which mods and addons existed at backup time
        let manifest_path = backup_dir.join("manifest.json");
        let manifest: BackupManifest = if manifest_path.exists() {
            let data = std::fs::read_to_string(&manifest_path)
                .context("failed to read backup manifest")?;
            serde_json::from_str(&data).context("failed to parse backup manifest")?
        } else {
            // Legacy backup without manifest — fall back to copying the
            // mods/ tree without DB file records. This is lossy but avoids
            // a hard failure on old backups.
            tracing::warn!("full backup has no manifest.json — file-to-mod mapping will be lost");
            copy_backup_subtree(&mods_dir, spt_dir)?;
            BackupManifest {
                mods: vec![],
                addons: vec![],
            }
        };

        // For each mod in the manifest: re-insert the mod row, copy files,
        // and record file metadata.
        for mm in &manifest.mods {
            let mod_db_id = db.insert_mod(
                mm.forge_mod_id,
                mm.forge_version_id,
                &mm.name,
                mm.slug.as_deref(),
                &mm.version,
            )?;

            restore_manifest_files(
                &mm.file_paths,
                &mods_dir,
                spt_dir,
                |rel_path, hash, size| {
                    db.insert_file(mod_db_id, rel_path, Some(hash), Some(size))?;
                    Ok(())
                },
            )?;
        }

        // Build a map from forge_mod_id to mod_db_id for addon restoration
        let mut mod_id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for mm in &manifest.mods {
            if let Ok(Some(m)) = db.get_mod_by_forge_id(mm.forge_mod_id) {
                mod_id_map.insert(mm.forge_mod_id, m.id);
            }
        }

        // For each addon in the manifest: re-insert the addon row, copy files,
        // and record file metadata.
        for ma in &manifest.addons {
            // Look up the parent mod's current DB id using the forge_mod_id
            let parent_mod_db_id = match mod_id_map.get(&ma.parent_forge_mod_id) {
                Some(&id) => id,
                None => {
                    tracing::warn!(
                        addon_name = %ma.name,
                        parent_forge_mod_id = ma.parent_forge_mod_id,
                        "parent mod not found for addon, skipping"
                    );
                    continue;
                }
            };

            let addon_db_id = db.insert_addon(
                ma.forge_addon_id,
                parent_mod_db_id,
                ma.forge_version_id,
                &ma.name,
                ma.slug.as_deref(),
                &ma.version,
                ma.mod_version_constraint.as_deref(),
            )?;

            restore_manifest_files(
                &ma.file_paths,
                &mods_dir,
                spt_dir,
                |rel_path, hash, size| {
                    db.insert_addon_file(addon_db_id, rel_path, Some(hash), Some(size))?;
                    Ok(())
                },
            )?;
        }
    }

    // Restore profiles
    let profiles_dir = backup_dir.join("profiles");
    if profiles_dir.is_dir() {
        let profiles_dst = spt_dir.join("user/profiles");
        std::fs::create_dir_all(&profiles_dst)?;
        for entry in std::fs::read_dir(&profiles_dir)? {
            let entry = entry?;
            let dst = profiles_dst.join(entry.file_name());
            std::fs::copy(entry.path(), &dst)?;
        }
    }

    // Restore config
    let config_src = backup_dir.join("quartermaster.toml");
    if config_src.exists() {
        let config_dst = spt_dir.join("quartermaster.toml");
        std::fs::copy(&config_src, &config_dst)?;
        tracing::warn!("quartermaster.toml restored — restart the web server to reload config");
    }

    db.set_backup_restored(backup_db_id)?;
    tx.commit()?;

    tracing::info!(backup_db_id, "full backup restored");

    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config after full restore");
    }

    if let Some(ref ms_config) = config.modsync {
        if let Err(e) = crate::modsync::ensure_all_mod_layouts(spt_dir, ms_config, db) {
            tracing::warn!(err = %e, "failed to ensure mod layouts after full restore");
        }
    }

    Ok(())
}

/// Recursively walk a backup directory tree, copying each file to `dst_root`
/// while preserving relative paths. Symlinks are skipped. For each copied file
/// the `on_file` callback is invoked with the relative path and destination path.
fn walk_backup_tree(
    current_dir: &Path,
    src_root: &Path,
    dst_root: &Path,
    on_file: &mut dyn FnMut(&Path, &Path) -> Result<()>,
) -> Result<()> {
    for entry in std::fs::read_dir(current_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            tracing::debug!(path = %entry.path().display(), "skipping symlink during backup restore");
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_backup_tree(&path, src_root, dst_root, on_file)?;
        } else {
            let relative = path
                .strip_prefix(src_root)
                .context("failed to compute relative path")?;
            let dst = dst_root.join(relative);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &dst)?;
            on_file(relative, &dst)?;
        }
    }
    Ok(())
}

/// Copy an entire backup subtree into dst_root, preserving relative paths.
/// Used as a fallback for legacy backups that lack a manifest.
fn copy_backup_subtree(src_root: &Path, dst_root: &Path) -> Result<()> {
    walk_backup_tree(src_root, src_root, dst_root, &mut |_, _| Ok(()))
}

/// Recursively copy files from a backup directory tree into the SPT directory,
/// recording each file in the database.
fn copy_backup_tree_and_record(
    db: &crate::db::Database,
    spt_dir: &Path,
    current_dir: &Path,
    backup_root: &Path,
    mod_db_id: i64,
    count: &mut u64,
) -> Result<()> {
    walk_backup_tree(current_dir, backup_root, spt_dir, &mut |relative, dst| {
        let content = std::fs::read(dst)?;
        let hash = crate::spt::mods::compute_hash_public(&content);
        let size = content.len() as i64;
        let rel_str = relative.to_string_lossy();
        db.insert_file(mod_db_id, &rel_str, Some(&hash), Some(size))?;
        *count += 1;
        Ok(())
    })
}

/// Prune backups that exceed the retention limit.
///
/// `count_fn` returns the current number of backups, `oldest_fn` returns the
/// oldest backup record. The caller decides which category of backups to query
/// (per-mod or full).
fn enforce_retention(
    db: &crate::db::Database,
    spt_dir: &Path,
    max_backups: u32,
    mut count_fn: impl FnMut() -> Result<i64>,
    mut oldest_fn: impl FnMut() -> Result<Option<crate::db::backups::BackupRecord>>,
) -> Result<()> {
    if max_backups == 0 {
        return Ok(());
    }
    while count_fn()? > max_backups as i64 {
        if let Some(oldest) = oldest_fn()? {
            let dir = spt_dir.join(&oldest.backup_path);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).ok();
            }
            db.delete_backup(oldest.id)?;
            tracing::debug!(backup_id = %oldest.backup_id, "pruned old backup");
        } else {
            break;
        }
    }
    Ok(())
}

/// Prune per-mod backups that exceed the retention limit.
fn enforce_retention_mod(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    forge_mod_id: i64,
) -> Result<()> {
    enforce_retention(
        db,
        spt_dir,
        config.backup.max_backups,
        || Ok(db.count_backups_for_mod(forge_mod_id)?),
        || Ok(db.oldest_backup_for_mod(forge_mod_id)?),
    )
}

/// Prune full backups that exceed the retention limit.
fn enforce_retention_full(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
) -> Result<()> {
    enforce_retention(
        db,
        spt_dir,
        config.backup.max_backups,
        || Ok(db.count_full_backups()?),
        || Ok(db.oldest_full_backup()?),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Create a standard test environment: a temp SPT dir with one mod
    /// ("TestMod") containing a single `package.json`, an in-memory DB with
    /// that mod tracked, and a default config.
    fn test_backup_env() -> (
        tempfile::TempDir,
        crate::db::Database,
        i64,
        crate::config::Config,
    ) {
        let spt_dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            b"{}",
        )
        .unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/package.json",
            Some("abc"),
            Some(2),
        )
        .unwrap();

        let config = crate::config::Config::default();
        (spt_dir, db, mod_id, config)
    }

    #[test]
    fn backup_id_format() {
        let id = generate_backup_id();
        // Format: YYYYMMDDTHHMMSS — 15 chars
        assert_eq!(id.len(), 15);
        assert_eq!(&id[8..9], "T");
    }

    #[test]
    fn backup_mod_copies_files_and_records() {
        let (spt_dir, db, mod_id, config) = test_backup_env();
        let backup_id = backup_mod(&db, spt_dir.path(), &config, mod_id, "manual").unwrap();

        let backup = db.get_backup(backup_id).unwrap().unwrap();
        assert_eq!(backup.backup_type, "mod");
        assert_eq!(backup.trigger, "manual");
        assert_eq!(backup.forge_mod_id, Some(100));

        let backup_dir = spt_dir.path().join(&backup.backup_path);
        assert!(backup_dir
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
    }

    #[test]
    fn backup_mod_enforces_retention() {
        let (spt_dir, db, mod_id, mut config) = test_backup_env();
        config.backup.max_backups = 2;

        // Create 3 backups — first should be pruned
        backup_mod(&db, spt_dir.path(), &config, mod_id, "manual").unwrap();
        backup_mod(&db, spt_dir.path(), &config, mod_id, "manual").unwrap();
        backup_mod(&db, spt_dir.path(), &config, mod_id, "manual").unwrap();

        let backups = db.list_backups_for_mod(100).unwrap();
        assert_eq!(backups.len(), 2);
    }

    #[test]
    fn backup_full_copies_mods_profiles_config() {
        let (spt_dir, db, _mod_id, config) = test_backup_env();
        // Add profiles and config for full backup test
        std::fs::create_dir_all(spt_dir.path().join("user/profiles")).unwrap();
        std::fs::write(
            spt_dir.path().join("user/profiles/abc123.json"),
            b"{\"info\":{}}",
        )
        .unwrap();
        std::fs::write(
            spt_dir.path().join("quartermaster.toml"),
            b"[backup]\nauto_backup = true\n",
        )
        .unwrap();

        let backup_id = backup_full(&db, spt_dir.path(), &config).unwrap();

        let backup = db.get_backup(backup_id).unwrap().unwrap();
        assert_eq!(backup.backup_type, "full");
        let backup_dir = spt_dir.path().join(&backup.backup_path);
        assert!(backup_dir
            .join("mods/SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(backup_dir.join("profiles/abc123.json").exists());
        assert!(backup_dir.join("quartermaster.toml").exists());

        // Manifest should be written
        let manifest_path = backup_dir.join("manifest.json");
        assert!(manifest_path.exists());
        let manifest: BackupManifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.mods.len(), 1);
        assert_eq!(manifest.mods[0].forge_mod_id, 100);
        assert_eq!(manifest.mods[0].name, "TestMod");
        assert_eq!(
            manifest.mods[0].file_paths,
            vec!["SPT/user/mods/TestMod/package.json"]
        );
    }

    #[test]
    fn auto_backup_skips_when_disabled() {
        let spt_dir = tempfile::TempDir::new().unwrap();
        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        let mut config = crate::config::Config::default();
        config.backup.auto_backup = false;

        auto_backup_mod(&db, spt_dir.path(), &config, mod_id, "auto_update").unwrap();
        assert_eq!(db.list_backups_for_mod(100).unwrap().len(), 0);
    }

    #[test]
    fn auto_backup_require_backup_propagates_error() {
        let spt_dir = tempfile::TempDir::new().unwrap();
        let db = crate::db::Database::open_in_memory().unwrap();
        // mod_id 999 doesn't exist — will error
        let mut config = crate::config::Config::default();
        config.backup.require_backup = true;

        let result = auto_backup_mod(&db, spt_dir.path(), &config, 999, "auto_update");
        assert!(result.is_err());
    }

    #[test]
    fn auto_backup_swallows_error_when_not_required() {
        let spt_dir = tempfile::TempDir::new().unwrap();
        let db = crate::db::Database::open_in_memory().unwrap();
        let mut config = crate::config::Config::default();
        config.backup.require_backup = false;

        let result = auto_backup_mod(&db, spt_dir.path(), &config, 999, "auto_update");
        assert!(result.is_ok());
    }

    #[test]
    fn restore_mod_backup_replaces_files() {
        let (spt_dir, db, mod_id, config) = test_backup_env();
        let backup_db_id = backup_mod(&db, spt_dir.path(), &config, mod_id, "manual").unwrap();

        // Simulate an update — overwrite the file
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            b"{\"v\":\"2\"}",
        )
        .unwrap();
        db.update_mod(mod_id, 300, "2.0.0").unwrap();

        // Restore
        restore_mod_backup(&db, spt_dir.path(), &config, backup_db_id).unwrap();

        // File should be restored to original content from backup (not the updated "v":"2")
        let content =
            std::fs::read_to_string(spt_dir.path().join("SPT/user/mods/TestMod/package.json"))
                .unwrap();
        assert_eq!(
            content, "{}",
            "file should be restored to original backup content"
        );
        assert!(
            !content.contains("\"v\":\"2\""),
            "updated content should be gone"
        );

        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert_eq!(m.version, "1.0.0");

        let backup = db.get_backup(backup_db_id).unwrap().unwrap();
        assert!(backup.restored_at.is_some());
    }

    #[test]
    fn restore_mod_backup_reinstalls_removed_mod() {
        let (spt_dir, db, mod_id, config) = test_backup_env();
        let backup_db_id = backup_mod(&db, spt_dir.path(), &config, mod_id, "auto_remove").unwrap();

        // Simulate removal
        crate::spt::mods::delete_mod_files(
            spt_dir.path(),
            &["SPT/user/mods/TestMod/package.json".to_string()],
        )
        .unwrap();
        db.delete_mod(mod_id).unwrap();
        assert!(db.get_mod(mod_id).unwrap().is_none());

        // Restore
        restore_mod_backup(&db, spt_dir.path(), &config, backup_db_id).unwrap();

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        // Mod should be re-inserted
        let m = db.get_mod_by_forge_id(100).unwrap().unwrap();
        assert_eq!(m.name, "TestMod");
        assert_eq!(m.version, "1.0.0");
    }

    #[test]
    fn backup_mod_finds_disabled_files_in_stash() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let config = crate::config::Config::default();

        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();

        // Create file in stash (as if disabled)
        let stash_path = spt_dir.join("quartermaster/disabled/SPT/user/mods/TestMod/package.json");
        std::fs::create_dir_all(stash_path.parent().unwrap()).unwrap();
        std::fs::write(&stash_path, b"{}").unwrap();

        // DB stores canonical path
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/package.json",
            Some("abc"),
            Some(2),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        let backup_id = backup_mod(&db, spt_dir, &config, mod_id, "manual").unwrap();
        let backup = db.get_backup(backup_id).unwrap().unwrap();
        assert!(
            backup.backup_size.unwrap_or(0) > 0,
            "backup should have found files in stash"
        );
    }

    #[test]
    fn restore_full_backup_restores_removed_mods() {
        let spt_dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/ModA")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/ModA/package.json"),
            b"{\"a\":1}",
        )
        .unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/ModB")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/ModB/package.json"),
            b"{\"b\":1}",
        )
        .unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_a = db
            .insert_mod(100, 200, "ModA", Some("mod-a"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_a,
            "SPT/user/mods/ModA/package.json",
            Some("aaa"),
            Some(7),
        )
        .unwrap();
        let mod_b = db
            .insert_mod(101, 201, "ModB", Some("mod-b"), "2.0.0")
            .unwrap();
        db.insert_file(
            mod_b,
            "SPT/user/mods/ModB/package.json",
            Some("bbb"),
            Some(7),
        )
        .unwrap();

        let config = crate::config::Config::default();
        let backup_db_id = backup_full(&db, spt_dir.path(), &config).unwrap();

        // Simulate removing ModB after backup
        crate::spt::mods::delete_mod_files(
            spt_dir.path(),
            &["SPT/user/mods/ModB/package.json".to_string()],
        )
        .unwrap();
        db.delete_files_for_mod(mod_b).unwrap();
        db.delete_mod(mod_b).unwrap();
        assert!(db.get_mod(mod_b).unwrap().is_none());

        // Restore full backup — ModB should come back
        restore_full_backup(&db, spt_dir.path(), &config, backup_db_id).unwrap();

        // Both mods should exist
        let restored_a = db.get_mod_by_forge_id(100).unwrap().unwrap();
        assert_eq!(restored_a.name, "ModA");
        let restored_b = db.get_mod_by_forge_id(101).unwrap().unwrap();
        assert_eq!(restored_b.name, "ModB");

        // Files should be on disk
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/ModA/package.json")
            .exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/ModB/package.json")
            .exists());

        // File records should exist
        let a_files = db.get_files_for_mod(restored_a.id).unwrap();
        assert_eq!(a_files.len(), 1);
        let b_files = db.get_files_for_mod(restored_b.id).unwrap();
        assert_eq!(b_files.len(), 1);
    }
}
