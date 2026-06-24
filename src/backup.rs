use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::RngExt;

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
pub fn resolve_backup_dir(spt_dir: &Path, config: &crate::config::Config) -> PathBuf {
    spt_dir.join(&config.backup.backup_dir)
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
    let backup_dir = resolve_backup_dir(spt_dir, config);
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
    let backup_dir = resolve_backup_dir(spt_dir, config);
    let full_dir = backup_dir.join("full");
    std::fs::create_dir_all(&full_dir)?;
    let bid = unique_backup_id(&full_dir);
    let dest = full_dir.join(&bid);

    let mut total_size: i64 = 0;

    // 1. Copy all mod files
    let mods = db.list_mods()?;
    for m in &mods {
        let files = db.get_files_for_mod(m.id)?;
        for file in &files {
            let src = spt_dir.join(&file.file_path);
            let dst = dest.join("mods").join(&file.file_path);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if src.exists() {
                std::fs::copy(&src, &dst)?;
                total_size += file.file_size.unwrap_or(0);
            }
        }
    }

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
}
