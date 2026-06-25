use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::RngExt;
use serde::{Deserialize, Serialize};

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

/// Manifest written into every full backup, recording which mods and files
/// existed at backup time so we can reconstruct the DB state on restore.
#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    mods: Vec<ManifestMod>,
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
    let resolved = spt_dir.join(&config.backup.backup_dir);
    let canonical_spt = spt_dir
        .canonicalize()
        .unwrap_or_else(|_| spt_dir.to_path_buf());
    let canonical_backup = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
    if !canonical_backup.starts_with(&canonical_spt) {
        anyhow::bail!("backup_dir must resolve within spt_dir");
    }
    Ok(resolved)
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
    let backup_dir = resolve_backup_dir(spt_dir, config)?;
    let mod_backup_dir = backup_dir
        .join("mods")
        .join(mod_info.forge_mod_id.to_string());
    std::fs::create_dir_all(&mod_backup_dir)
        .with_context(|| format!("failed to create backup dir: {}", mod_backup_dir.display()))?;

    let bid = unique_backup_id(&mod_backup_dir);
    let dest = mod_backup_dir.join(&bid);

    let mut total_size: i64 = 0;
    for file in &files {
        let src = spt_dir.join(&file.file_path);
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

    tracing::info!(
        mod_db_id,
        backup_id = %bid,
        file_count = files.len(),
        total_size,
        "mod backed up"
    );

    enforce_retention_mod(db, spt_dir, config, mod_info.forge_mod_id)?;

    Ok(backup_db_id)
}

/// Back up all installed mods, player profiles, and the Quartermaster config file.
///
/// Creates a single "full" backup containing:
/// 1. All files for every installed mod (under `mods/`)
/// 2. All `.json` profile files from `user/profiles/` (under `profiles/`)
/// 3. The `quartermaster.toml` config file
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

    let mut total_size: i64 = 0;
    let mut manifest_mods: Vec<ManifestMod> = Vec::new();

    // 1. Copy all mod files and build manifest
    let mods = db.list_mods()?;
    for m in &mods {
        let files = db.get_files_for_mod(m.id)?;
        let mut file_paths: Vec<String> = Vec::new();
        for file in &files {
            let src = spt_dir.join(&file.file_path);
            let dst = dest.join("mods").join(&file.file_path);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if src.exists() {
                std::fs::copy(&src, &dst)?;
                total_size += file.file_size.unwrap_or(0);
                file_paths.push(file.file_path.clone());
            }
        }
        manifest_mods.push(ManifestMod {
            forge_mod_id: m.forge_mod_id,
            forge_version_id: m.forge_version_id,
            name: m.name.clone(),
            slug: m.slug.clone(),
            version: m.version.clone(),
            file_paths,
        });
    }

    // Write manifest so restore can reconstruct DB state for removed mods
    let manifest = BackupManifest {
        mods: manifest_mods,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("failed to serialize backup manifest")?;
    std::fs::write(dest.join("manifest.json"), manifest_json)
        .context("failed to write backup manifest")?;

    // 2. Copy profiles
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

    // 3. Copy config
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
                tracing::warn!(mod_db_id, error = %e, "auto-backup failed, continuing");
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
            crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
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

    db.set_backup_restored(backup_db_id)?;
    tx.commit()?;

    tracing::info!(
        backup_db_id,
        mod_db_id,
        restored_count,
        "mod restored from backup"
    );

    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(error = %e, "failed to regenerate NarcoNet config after restore");
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
            crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
            db.delete_files_for_mod(m.id)?;
            db.delete_mod(m.id)?;
        }

        // Read manifest to know which mods existed at backup time
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
            BackupManifest { mods: vec![] }
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

            for rel_path in &mm.file_paths {
                let src = mods_dir.join(rel_path);
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
                db.insert_file(mod_db_id, rel_path, Some(&hash), Some(size))?;
            }
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
        tracing::warn!(error = %e, "failed to regenerate NarcoNet config after full restore");
    }

    Ok(())
}

/// Copy an entire backup subtree into spt_dir, preserving relative paths.
/// Used as a fallback for legacy backups that lack a manifest.
fn copy_backup_subtree(src_root: &Path, dst_root: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src_root)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(src_root)
            .context("failed to compute relative path")?;
        let dst = dst_root.join(relative);
        if path.is_dir() {
            std::fs::create_dir_all(&dst)?;
            copy_backup_subtree(&path, dst_root)?;
        } else {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &dst)?;
        }
    }
    Ok(())
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
    for entry in std::fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            copy_backup_tree_and_record(db, spt_dir, &path, backup_root, mod_db_id, count)?;
        } else {
            let relative = path.strip_prefix(backup_root)?;
            let dst = spt_dir.join(relative);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &dst)?;

            let content = std::fs::read(&dst)?;
            let hash = crate::spt::mods::compute_hash_public(&content);
            let size = content.len() as i64;
            let rel_str = relative.to_string_lossy();
            db.insert_file(mod_db_id, &rel_str, Some(&hash), Some(size))?;
            *count += 1;
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
    if config.backup.max_backups == 0 {
        return Ok(());
    }
    while db.count_backups_for_mod(forge_mod_id)? > config.backup.max_backups as i64 {
        if let Some(oldest) = db.oldest_backup_for_mod(forge_mod_id)? {
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

/// Prune full backups that exceed the retention limit.
fn enforce_retention_full(
    db: &crate::db::Database,
    spt_dir: &Path,
    config: &crate::config::Config,
) -> Result<()> {
    if config.backup.max_backups == 0 {
        return Ok(());
    }
    while db.count_full_backups()? > config.backup.max_backups as i64 {
        if let Some(oldest) = db.oldest_full_backup()? {
            let dir = spt_dir.join(&oldest.backup_path);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).ok();
            }
            db.delete_backup(oldest.id)?;
        } else {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn backup_id_format() {
        let id = generate_backup_id();
        // Format: YYYYMMDDTHHMMSS — 15 chars
        assert_eq!(id.len(), 15);
        assert_eq!(&id[8..9], "T");
    }

    #[test]
    fn backup_mod_copies_files_and_records() {
        let spt_dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            b"{}",
        )
        .unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/package.json",
            Some("abc123"),
            Some(2),
        )
        .unwrap();

        let config = crate::config::Config::default();
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

        let mut config = crate::config::Config::default();
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
        let spt_dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            b"{}",
        )
        .unwrap();
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
        let spt_dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            b"{\"v\":\"1\"}",
        )
        .unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/package.json",
            Some("abc"),
            Some(12),
        )
        .unwrap();

        let config = crate::config::Config::default();
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

        let content =
            std::fs::read_to_string(spt_dir.path().join("SPT/user/mods/TestMod/package.json"))
                .unwrap();
        assert!(content.contains("\"v\":\"1\""));

        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert_eq!(m.version, "1.0.0");

        let backup = db.get_backup(backup_db_id).unwrap().unwrap();
        assert!(backup.restored_at.is_some());
    }

    #[test]
    fn restore_mod_backup_reinstalls_removed_mod() {
        let spt_dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(spt_dir.path().join("SPT/user/mods/TestMod")).unwrap();
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            b"{}",
        )
        .unwrap();

        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/package.json",
            Some("abc"),
            Some(2),
        )
        .unwrap();

        let config = crate::config::Config::default();
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
