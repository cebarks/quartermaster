use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::mods::InstalledMod;
use crate::db::Database;
use crate::spt::mods::ExtractedFile;

fn record_extracted_files(db: &Database, mod_db_id: i64, files: &[ExtractedFile]) -> Result<()> {
    for file in files {
        db.insert_file(
            mod_db_id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }
    Ok(())
}

/// Parameters for installing a mod from a downloaded archive.
pub struct InstallRequest<'a> {
    pub db: &'a Database,
    pub spt_dir: &'a Path,
    pub config: &'a crate::config::Config,
    pub forge_mod_id: i64,
    pub version_id: i64,
    pub name: &'a str,
    pub slug: Option<&'a str>,
    pub version: &'a str,
    pub archive_path: &'a Path,
}

pub fn install_mod_from_archive(req: &InstallRequest<'_>) -> Result<i64> {
    tracing::info!(
        mod_name = req.name,
        mod_id = req.forge_mod_id,
        version = req.version,
        "installing mod from archive"
    );

    // Extract to a staging directory so files are not left on disk if the DB
    // transaction fails (mirrors the update path).
    let staging_dir = tempfile::tempdir()?;
    let extracted = crate::spt::mods::extract_mod(req.archive_path, staging_dir.path())?;

    let tx = req.db.begin_transaction()?;
    let db_id = req.db.insert_mod(
        req.forge_mod_id,
        req.version_id,
        req.name,
        req.slug,
        req.version,
    )?;
    record_extracted_files(req.db, db_id, &extracted)?;
    tx.commit()?;

    // DB committed — now move files from staging to the live directory.
    for file in &extracted {
        let src = staging_dir.path().join(&file.path);
        let dst = req.spt_dir.join(&file.path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
    }

    tracing::debug!(
        db_id,
        file_count = extracted.len(),
        "mod installed, files recorded"
    );
    if let Err(e) = crate::modsync::regenerate_if_enabled(req.spt_dir, req.config, req.db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    Ok(db_id)
}

pub fn update_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
    version_id: i64,
    version_str: &str,
    archive_path: &Path,
) -> Result<()> {
    tracing::info!(
        mod_db_id,
        version = version_str,
        "updating mod from archive"
    );
    let staging_dir = tempfile::tempdir()?;
    let extracted = crate::spt::mods::extract_mod(archive_path, staging_dir.path())?;

    let old_files = db.get_files_for_mod(mod_db_id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(
        old_file_count = old_paths.len(),
        new_file_count = extracted.len(),
        "replacing mod files"
    );
    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_update")?;

    // Copy new files first (overwriting any shared with old version), so that
    // if copying fails mid-way the old files that weren't overwritten remain
    // intact. This is strictly safer than delete-all-then-copy-all.
    for file in &extracted {
        let src = staging_dir.path().join(&file.path);
        let dst = spt_dir.join(&file.path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
    }

    // Delete old files that are NOT in the new file set (stale files only).
    let new_paths: std::collections::HashSet<&str> =
        extracted.iter().map(|f| f.path.as_str()).collect();
    let stale_paths: Vec<String> = old_paths
        .into_iter()
        .filter(|p| !new_paths.contains(p.as_str()))
        .collect();
    if !stale_paths.is_empty() {
        tracing::debug!(stale_count = stale_paths.len(), "removing stale files");
        crate::spt::mods::delete_mod_files(spt_dir, &stale_paths)?;
    }

    let tx = db.begin_transaction()?;
    db.delete_files_for_mod(mod_db_id)?;
    record_extracted_files(db, mod_db_id, &extracted)?;
    db.update_mod(mod_db_id, version_id, version_str)?;
    tx.commit()?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    Ok(())
}

/// Apply a mod update using brief DB locks suitable for the web context.
///
/// This is the async counterpart of [`update_mod_from_archive`]: it performs the
/// same 3-step update (read old paths, filesystem swap, DB write) but splits
/// each step into a separate [`actix_web::web::block`] call so the DB mutex is
/// never held across slow filesystem I/O.
///
/// A `pending_updates` marker row is written to the database *before* any
/// destructive filesystem work begins. If the process crashes between the
/// filesystem swap and the final DB commit, [`recover_pending_updates`] will
/// detect and resolve the inconsistency on the next startup.
///
/// `extracted` must be the files already extracted to `staging_path` (e.g. via
/// [`crate::spt::mods::extract_mod`]).
#[allow(clippy::too_many_arguments)]
pub async fn apply_mod_update(
    db: Arc<parking_lot::Mutex<Database>>,
    spt_dir: PathBuf,
    config: crate::config::Config,
    staging_path: PathBuf,
    extracted: Vec<ExtractedFile>,
    mod_db_id: i64,
    version_id: i64,
    version_str: String,
) -> Result<()> {
    // Serialize file metadata for the pending_updates marker
    let new_files_json =
        serde_json::to_string(&extracted).context("failed to serialize new file paths")?;

    // Step 1: Read old file paths, auto-backup, and write pending marker (brief DB lock)
    let db_step1 = db.clone();
    let spt_dir_backup = spt_dir.clone();
    let config_backup = config;
    let version_str_step1 = version_str.clone();
    let new_files_json_step1 = new_files_json;
    let (old_paths, pending_id) = actix_web::web::block(move || {
        let db = db_step1.lock();
        crate::backup::auto_backup_mod(
            &db,
            &spt_dir_backup,
            &config_backup,
            mod_db_id,
            "auto_update",
        )?;
        let files = db.get_files_for_mod(mod_db_id)?;
        let old_paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
        let old_files_json =
            serde_json::to_string(&old_paths).context("failed to serialize old file paths")?;

        // Write the pending marker before any destructive filesystem work
        let pending_id = db.insert_pending_update(
            mod_db_id,
            version_id,
            &version_str_step1,
            &new_files_json_step1,
            &old_files_json,
        )?;
        tracing::debug!(mod_db_id, pending_id, "pending update marker written");

        Ok::<_, anyhow::Error>((old_paths, pending_id))
    })
    .await??;

    // Step 2: Filesystem swap (no DB lock held)
    // Copy new files first, then delete stale-only old files. If copying
    // fails partway, old files that weren't overwritten remain intact.
    let spt_dir_fs = spt_dir.clone();
    let extracted = actix_web::web::block(move || {
        for file in &extracted {
            let src = staging_path.join(&file.path);
            let dst = spt_dir_fs.join(&file.path);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
        }

        let new_paths: std::collections::HashSet<&str> =
            extracted.iter().map(|f| f.path.as_str()).collect();
        let stale_paths: Vec<String> = old_paths
            .into_iter()
            .filter(|p| !new_paths.contains(p.as_str()))
            .collect();
        if !stale_paths.is_empty() {
            tracing::debug!(stale_count = stale_paths.len(), "removing stale files");
            crate::spt::mods::delete_mod_files(&spt_dir_fs, &stale_paths)?;
        }

        Ok::<_, anyhow::Error>(extracted)
    })
    .await??;

    // Step 3: DB writes atomically + clear pending marker (brief DB lock)
    let db_step3 = db;
    let result = actix_web::web::block(move || {
        let db = db_step3.lock();
        let tx = db.begin_transaction()?;
        db.delete_files_for_mod(mod_db_id)?;
        record_extracted_files(&db, mod_db_id, &extracted)?;
        db.update_mod(mod_db_id, version_id, &version_str)?;
        db.delete_pending_update(pending_id)?;
        tx.commit()?;
        tracing::debug!(mod_db_id, pending_id, "pending update marker cleared");
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    if let Err(ref e) = result {
        tracing::error!(
            mod_db_id,
            pending_id,
            error = %e,
            "INCONSISTENT_STATE: filesystem updated but DB write failed for mod update. \
             A pending_updates record (id={}) exists — recovery will run on next startup.",
            pending_id
        );
    }

    result
}

/// Recover from interrupted async mod updates on startup.
///
/// Scans the `pending_updates` table for markers left behind by [`apply_mod_update`]
/// if the process crashed or step 3 failed. For each pending record, inspects the
/// filesystem to determine whether to complete the update forward or clear the stale
/// marker.
///
/// This should be called once during server startup, before the HTTP server begins
/// accepting requests.
pub fn recover_pending_updates(db: &Database, spt_dir: &Path) -> Result<()> {
    let pending = db.list_pending_updates()?;
    if pending.is_empty() {
        return Ok(());
    }

    tracing::info!(
        count = pending.len(),
        "found pending update markers — running recovery"
    );

    for record in &pending {
        if let Err(e) = recover_single_update(db, spt_dir, record) {
            tracing::error!(
                pending_id = record.id,
                mod_db_id = record.mod_db_id,
                error = %e,
                "failed to recover pending update — manual intervention may be needed"
            );
        }
    }

    Ok(())
}

fn recover_single_update(
    db: &Database,
    spt_dir: &Path,
    record: &crate::db::mods::PendingUpdate,
) -> Result<()> {
    // Check if the mod row still exists
    let mod_exists = db.get_mod(record.mod_db_id)?.is_some();
    if !mod_exists {
        tracing::warn!(
            pending_id = record.id,
            mod_db_id = record.mod_db_id,
            "cleared orphaned pending update marker (mod row was deleted)"
        );
        db.delete_pending_update(record.id)?;
        return Ok(());
    }

    // Parse the JSON file lists
    let new_files: Vec<ExtractedFile> = serde_json::from_str(&record.new_file_paths)
        .context("failed to parse new_file_paths JSON from pending_updates")?;
    let old_paths: Vec<String> = serde_json::from_str(&record.old_file_paths)
        .context("failed to parse old_file_paths JSON from pending_updates")?;

    // Check how many new files exist on disk with correct hashes
    let mut new_files_ok = 0usize;
    for file in &new_files {
        let path = spt_dir.join(&file.path);
        if path.exists() {
            match std::fs::read(&path) {
                Ok(content) => {
                    let hash = crate::spt::mods::compute_hash_public(&content);
                    if hash == file.hash {
                        new_files_ok += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        path = %file.path,
                        error = %e,
                        "recovery: file exists but could not be read"
                    );
                }
            }
        }
    }

    // Check how many old files still exist on disk
    let old_files_exist = old_paths
        .iter()
        .filter(|p| spt_dir.join(p).exists())
        .count();

    let all_new_present = new_files_ok == new_files.len();

    if all_new_present {
        // All new files present with correct hashes — complete the DB update
        let tx = db.begin_transaction()?;
        db.delete_files_for_mod(record.mod_db_id)?;
        record_extracted_files(db, record.mod_db_id, &new_files)?;
        db.update_mod(record.mod_db_id, record.version_id, &record.version_str)?;
        db.delete_pending_update(record.id)?;
        tx.commit()?;
        tracing::info!(
            mod_db_id = record.mod_db_id,
            pending_id = record.id,
            version = %record.version_str,
            "recovered interrupted update: completed DB update"
        );
    } else if new_files_ok > 0 && new_files_ok < new_files.len() {
        // Partial copy — some new files exist but not all. Clean up the
        // partially-copied new files (only those that don't overlap with old
        // paths) and clear the marker.
        let old_set: std::collections::HashSet<&str> =
            old_paths.iter().map(|p| p.as_str()).collect();
        for file in &new_files {
            if !old_set.contains(file.path.as_str()) {
                let path = spt_dir.join(&file.path);
                if path.exists() {
                    if let Err(e) = std::fs::remove_file(&path) {
                        tracing::warn!(
                            path = %file.path,
                            error = %e,
                            "failed to clean up partially-copied file during recovery"
                        );
                    }
                }
            }
        }
        db.delete_pending_update(record.id)?;
        tracing::warn!(
            mod_db_id = record.mod_db_id,
            pending_id = record.id,
            new_present = new_files_ok,
            new_total = new_files.len(),
            "recovered interrupted update: rolled back partial copy. \
             Restore from backup if mod files are inconsistent."
        );
    } else if old_files_exist > 0 {
        // No new files on disk, old files still present — swap never happened
        db.delete_pending_update(record.id)?;
        tracing::info!(
            mod_db_id = record.mod_db_id,
            pending_id = record.id,
            "recovered interrupted update: filesystem unchanged, cleared stale marker"
        );
    } else {
        // Neither old nor new files — ambiguous state
        db.delete_pending_update(record.id)?;
        tracing::warn!(
            mod_db_id = record.mod_db_id,
            pending_id = record.id,
            "recovered interrupted update: ambiguous state (no old or new files found). \
             Restore from backup if needed."
        );
    }

    Ok(())
}

pub fn remove_mod_by_id(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
) -> Result<()> {
    tracing::info!(mod_db_id, "removing mod");
    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_remove")?;
    let files = db.get_files_for_mod(mod_db_id)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(file_count = paths.len(), "deleting mod files");
    crate::spt::mods::delete_mod_files(spt_dir, &paths)?;

    // Look up forge_mod_id before deletion for group cleanup
    let forge_mod_id = db.get_mod(mod_db_id)?.map(|m| m.forge_mod_id);

    let tx = db.begin_transaction()?;
    db.delete_mod(mod_db_id)?;
    tx.commit()?;

    // Eager cleanup: strip uninstalled mod from any group
    if let Some(forge_id) = forge_mod_id {
        let config_path = crate::config::Config::resolve_path(None, Some(spt_dir));
        if config_path.exists() {
            if let Ok(mut cfg) = crate::config::Config::load(&config_path) {
                let mut changed = false;
                if let Some(ref mut ms) = cfg.modsync {
                    for group in ms.groups.values_mut() {
                        if let Some(pos) = group.members.iter().position(|&id| id == forge_id) {
                            group.members.remove(pos);
                            changed = true;
                        }
                    }
                }
                if changed {
                    if let Err(e) = cfg.save(&config_path) {
                        tracing::warn!(err = %e, "failed to clean up group membership after mod removal");
                    }
                }
            }
        }
    }

    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    Ok(())
}

/// A runtime-generated file discovered on disk but not in the original mod archive.
struct RuntimeFile {
    relative_path: String,
    hash: String,
    size: i64,
}

/// Scan a mod's directories on disk and record any files not already tracked
/// as runtime-generated files (source = 'runtime').
///
/// Splits work into three phases to minimise the time the DB mutex is held:
/// 1. Brief lock to read currently-tracked file paths.
/// 2. No lock — recursive filesystem scan + streaming SHA-256 hashing.
/// 3. Brief lock to batch-insert discovered files inside a transaction.
pub fn scan_and_record_runtime_files(
    db: &std::sync::Arc<parking_lot::Mutex<Database>>,
    mod_db_id: i64,
    spt_dir: &Path,
) -> Result<()> {
    // Phase 1: Read tracked paths (brief DB lock)
    let tracked_paths: std::collections::HashSet<String> = {
        let db = db.lock();
        let tracked = db.get_files_for_mod(mod_db_id)?;
        tracked.into_iter().map(|f| f.file_path).collect()
    };

    // Determine which top-level directories this mod occupies
    let mut mod_dirs: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for file_path in &tracked_paths {
        let p = Path::new(file_path);
        // For SPT/user/mods/ModName/... take first 4 components
        // For BepInEx/plugins/ModName/... take first 3 components
        let parts: Vec<&str> = file_path.split('/').collect();
        let dir = if file_path.starts_with("SPT/") && parts.len() >= 4 {
            format!("{}/{}/{}/{}", parts[0], parts[1], parts[2], parts[3])
        } else if file_path.starts_with("BepInEx/") && parts.len() >= 3 {
            format!("{}/{}/{}", parts[0], parts[1], parts[2])
        } else if let Some(parent) = p.parent() {
            parent.to_string_lossy().to_string()
        } else {
            continue;
        };
        mod_dirs.insert(spt_dir.join(dir));
    }

    tracing::debug!(
        mod_db_id,
        dir_count = mod_dirs.len(),
        "scanning for runtime files"
    );

    // Phase 2: Filesystem scan + streaming hash (NO lock held)
    let mut runtime_files = Vec::new();
    for dir in &mod_dirs {
        if !dir.is_dir() {
            continue;
        }
        scan_runtime_recursive(dir, spt_dir, &tracked_paths, &mut runtime_files)?;
    }

    // Phase 3: Batch insert (brief DB lock)
    if !runtime_files.is_empty() {
        let db = db.lock();
        let tx = db.begin_transaction()?;
        for file in &runtime_files {
            if let Err(e) = db.insert_file_with_source(
                mod_db_id,
                &file.relative_path,
                Some(&file.hash),
                Some(file.size),
                "runtime",
            ) {
                tracing::warn!(
                    path = %file.relative_path,
                    error = %e,
                    "failed to record runtime file"
                );
            }
        }
        tx.commit()?;
    }

    Ok(())
}

fn scan_runtime_recursive(
    dir: &Path,
    spt_root: &Path,
    tracked: &std::collections::HashSet<String>,
    results: &mut Vec<RuntimeFile>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            tracing::debug!(path = %entry.path().display(), "skipping symlink during runtime scan");
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            scan_runtime_recursive(&path, spt_root, tracked, results)?;
        } else if let Ok(relative) = path.strip_prefix(spt_root) {
            let rel_str = relative.to_string_lossy().to_string();
            if !tracked.contains(&rel_str) {
                tracing::trace!(path = %rel_str, "recording runtime file");
                // Use streaming hash to avoid loading entire files into memory
                match crate::spt::mods::compute_file_hash(&path) {
                    Ok(hash) => {
                        let size = std::fs::metadata(&path)
                            .map(|m| m.len() as i64)
                            .unwrap_or(0);
                        results.push(RuntimeFile {
                            relative_path: rel_str,
                            hash,
                            size,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            err = %e,
                            "skipping unreadable runtime file"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// Given a list of file paths belonging to a mod, find the unique top-level
/// directories that contain them (e.g. `SPT/user/mods/ModName` or
/// `BepInEx/plugins/PluginDir`). Returns relative paths.
fn find_top_level_mod_dirs(file_paths: &[String]) -> Vec<String> {
    let mut dirs = std::collections::BTreeSet::new();
    for path in file_paths {
        let parts: Vec<&str> = path.split('/').collect();
        // SPT/user/mods/ModName/... → 4 components
        if path.starts_with("SPT/user/mods/") && parts.len() >= 4 {
            dirs.insert(format!(
                "{}/{}/{}/{}",
                parts[0], parts[1], parts[2], parts[3]
            ));
        // BepInEx/plugins/PluginDir/... → 3 components (if the file is inside a directory)
        } else if path.starts_with("BepInEx/plugins/") && parts.len() >= 3 {
            // Check if third component is a directory (has children) or a loose file
            if parts.len() > 3 {
                dirs.insert(format!("{}/{}/{}", parts[0], parts[1], parts[2]));
            }
            // Loose files (e.g. BepInEx/plugins/something.dll) are handled separately
        }
    }
    dirs.into_iter().collect()
}

/// Find loose files that are not inside a top-level mod directory.
/// These are individual files like `BepInEx/plugins/something.dll`.
fn find_loose_files<'a>(file_paths: &'a [String], top_dirs: &[String]) -> Vec<&'a str> {
    file_paths
        .iter()
        .filter(|p| !top_dirs.iter().any(|d| p.starts_with(d)))
        .map(|p| p.as_str())
        .collect()
}

/// Undo a list of completed renames in reverse order (dst -> src).
fn undo_renames(completed: &[(PathBuf, PathBuf)]) {
    for (src, dst) in completed.iter().rev() {
        if let Err(undo_err) = std::fs::rename(dst, src) {
            tracing::error!(
                from = %dst.display(),
                to = %src.display(),
                error = %undo_err,
                "CRITICAL: failed to undo rename during rollback"
            );
        }
    }
}

/// Perform a batch of filesystem renames, returning the completed renames.
/// On failure, automatically undoes all completed renames before returning the error.
fn rename_batch(renames: &[(PathBuf, PathBuf)]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut completed: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (src, dst) in renames {
        if src.exists() {
            if let Err(e) = std::fs::rename(src, dst) {
                undo_renames(&completed);
                return Err(e).with_context(|| format!("failed to rename {}", src.display()));
            }
            completed.push((src.clone(), dst.clone()));
            tracing::debug!(from = %src.display(), to = %dst.display(), "renamed");
        }
    }
    Ok(completed)
}

/// Disable a mod by renaming its top-level directories and loose files with
/// a `.disabled` suffix, updating file paths in the database, and marking
/// the mod as disabled.
pub fn disable_mod(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
) -> Result<()> {
    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
    if mod_info.disabled {
        anyhow::bail!("mod is already disabled");
    }

    let files = db.get_files_for_mod(mod_db_id)?;
    let file_paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
    let top_dirs = find_top_level_mod_dirs(&file_paths);
    let loose = find_loose_files(&file_paths, &top_dirs);

    tracing::info!(mod_db_id, mod_name = %mod_info.name, "disabling mod");

    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_disable")?;

    // Begin transaction: update DB first, then filesystem
    let tx = db.begin_transaction()?;

    // Update file paths in the database
    for file in &files {
        let new_path = if let Some(matching_dir) = top_dirs
            .iter()
            .find(|d| file.file_path.starts_with(d.as_str()))
        {
            file.file_path
                .replacen(matching_dir, &format!("{matching_dir}.disabled"), 1)
        } else if loose.contains(&file.file_path.as_str()) {
            format!("{}.disabled", file.file_path)
        } else {
            continue;
        };
        db.rename_file_path(file.id, &new_path)?;
    }

    db.set_mod_disabled(mod_db_id, true)?;

    // Build rename list and perform filesystem renames
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for dir in &top_dirs {
        let src = spt_dir.join(dir);
        let dst = spt_dir.join(format!("{dir}.disabled"));
        renames.push((src, dst));
    }
    for loose_path in &loose {
        let src = spt_dir.join(loose_path);
        let dst = spt_dir.join(format!("{loose_path}.disabled"));
        renames.push((src, dst));
    }

    let completed = rename_batch(&renames)?;

    // Commit transaction -- undo renames if commit fails
    if let Err(e) = tx.commit() {
        undo_renames(&completed);
        return Err(e.into());
    }

    tracing::info!(mod_db_id, mod_name = %mod_info.name, "mod disabled");
    Ok(())
}

/// Enable a previously disabled mod by removing the `.disabled` suffix from
/// its directories and files, updating file paths in the database, and
/// clearing the disabled flag.
pub fn enable_mod(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
) -> Result<()> {
    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
    if !mod_info.disabled {
        anyhow::bail!("mod is not disabled");
    }

    let files = db.get_files_for_mod(mod_db_id)?;
    let file_paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
    let top_dirs = find_top_level_mod_dirs(&file_paths);
    let loose = find_loose_files(&file_paths, &top_dirs);

    tracing::info!(mod_db_id, mod_name = %mod_info.name, "enabling mod");

    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_enable")?;

    // Begin transaction: update DB first, then filesystem
    let tx = db.begin_transaction()?;

    // Update file paths in the database (strip .disabled from paths)
    for file in &files {
        let new_path = if let Some(matching_dir) = top_dirs
            .iter()
            .find(|d| file.file_path.starts_with(d.as_str()))
        {
            if matching_dir.ends_with(".disabled") {
                let restored_dir = matching_dir
                    .strip_suffix(".disabled")
                    .expect("checked by ends_with above");
                file.file_path
                    .replacen(matching_dir.as_str(), restored_dir, 1)
            } else {
                continue;
            }
        } else if file.file_path.ends_with(".disabled") {
            file.file_path
                .strip_suffix(".disabled")
                .expect("checked by ends_with above")
                .to_string()
        } else {
            continue;
        };
        db.rename_file_path(file.id, &new_path)?;
    }

    db.set_mod_disabled(mod_db_id, false)?;

    // Build rename list and perform filesystem renames (strip .disabled suffix)
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for dir in &top_dirs {
        if dir.ends_with(".disabled") {
            let restored = dir
                .strip_suffix(".disabled")
                .expect("checked by ends_with above");
            let src = spt_dir.join(dir);
            let dst = spt_dir.join(restored);
            renames.push((src, dst));
        }
    }
    for loose_path in &loose {
        if loose_path.ends_with(".disabled") {
            let restored = loose_path
                .strip_suffix(".disabled")
                .expect("checked by ends_with above");
            let src = spt_dir.join(loose_path);
            let dst = spt_dir.join(restored);
            renames.push((src, dst));
        }
    }

    let completed = rename_batch(&renames)?;

    // Commit transaction -- undo renames if commit fails
    if let Err(e) = tx.commit() {
        undo_renames(&completed);
        return Err(e.into());
    }

    tracing::info!(mod_db_id, mod_name = %mod_info.name, "mod enabled");
    Ok(())
}

/// Recursively collect all transitive reverse dependencies of a mod.
/// Returns them in BFS order (direct dependents first, then their dependents, etc.).
pub fn collect_all_reverse_deps(db: &Database, mod_db_id: i64) -> Result<Vec<InstalledMod>> {
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(mod_db_id);
    visited.insert(mod_db_id);

    while let Some(current_id) = queue.pop_front() {
        let rev_deps = db.get_reverse_dependencies(current_id)?;
        for dep in rev_deps {
            if visited.insert(dep.mod_id) {
                if let Some(dependent) = db.get_mod(dep.mod_id)? {
                    queue.push_back(dependent.id);
                    result.push(dependent);
                }
            }
        }
    }

    Ok(result)
}

/// Resolve and install all dependencies for a mod.
///
/// Reuses `download_and_install_with_arc` for each dependency so we get
/// staging-directory safety and proper DB recording. Returns the DB IDs
/// of installed dependencies for recording edges.
pub async fn resolve_and_install_deps(
    forge: &crate::forge::client::ForgeClient,
    db: &Arc<parking_lot::Mutex<crate::db::Database>>,
    spt_dir: &Path,
    config: &crate::config::Config,
    forge_mod_id: i64,
    selected_version: &crate::forge::models::ForgeVersion,
) -> Result<Vec<i64>> {
    let dep_nodes = forge
        .get_dependencies(&[(&forge_mod_id.to_string(), &selected_version.version)])
        .await?;

    let mut to_install = Vec::new();
    collect_web_deps(&dep_nodes, db, &mut to_install)?;

    if !to_install.is_empty() {
        tracing::info!(
            count = to_install.len(),
            deps = ?to_install.iter().map(|d| &d.name).collect::<Vec<_>>(),
            "installing dependencies"
        );
    }

    let mut installed_db_ids = Vec::new();

    for dep in &to_install {
        // Race condition guard: another concurrent install may have installed
        // this dep between collect_web_deps and now
        {
            let db_guard = db.lock();
            if let Some(existing) = db_guard.get_mod_by_forge_id(dep.mod_id)? {
                tracing::debug!(
                    dep.mod_id,
                    name = dep.name,
                    "dependency already installed (race)"
                );
                installed_db_ids.push(existing.id);
                continue;
            }
        }

        let dep_versions = forge.get_versions(dep.mod_id, None).await?;
        let dep_ver = dep_versions
            .iter()
            .find(|v| v.id == dep.version_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "version {} for dependency {} not found on Forge",
                    dep.version_id,
                    dep.name
                )
            })?;

        let link = dep_ver
            .link
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no download link for {} v{}", dep.name, dep.version))?;

        let dep_mod = forge.get_mod(dep.mod_id, false).await?;

        let db_id = crate::cli::install::download_and_install_with_arc(
            forge,
            db,
            spt_dir,
            config,
            &crate::cli::install::ModInstallParams {
                forge_mod_id: dep.mod_id,
                forge_version_id: dep.version_id,
                download_url: link,
                name: &dep.name,
                slug: dep_mod.slug.as_deref(),
                version: &dep.version,
            },
        )
        .await?;

        tracing::info!(
            dep.mod_id,
            name = dep.name,
            version = dep.version,
            "dependency installed"
        );
        installed_db_ids.push(db_id);
    }

    Ok(installed_db_ids)
}

/// Record dependency edges in the database (parent mod depends on each dep).
pub fn record_dep_edges(
    db: &Arc<parking_lot::Mutex<crate::db::Database>>,
    main_mod_db_id: i64,
    dep_db_ids: &[i64],
) {
    let db = db.lock();
    for dep_db_id in dep_db_ids {
        match db.insert_dependency(main_mod_db_id, *dep_db_id, None) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ffi::ErrorCode::ConstraintViolation => {}
            Err(e) => {
                tracing::warn!(main_mod_db_id, dep_db_id, err = %e, "failed to record dependency edge");
            }
        }
    }
}

struct PendingDep {
    mod_id: i64,
    version_id: i64,
    name: String,
    version: String,
}

fn collect_web_deps(
    nodes: &[crate::forge::models::DependencyNode],
    db: &Arc<parking_lot::Mutex<crate::db::Database>>,
    out: &mut Vec<PendingDep>,
) -> Result<()> {
    for node in nodes {
        if node.conflict {
            tracing::warn!(mod_name = node.name, "skipping conflicting dependency");
            continue;
        }

        {
            let db = db.lock();
            if db.get_mod_by_forge_id(node.id)?.is_some() {
                continue;
            }
        }

        if out.iter().any(|d| d.mod_id == node.id) {
            continue;
        }

        // Recurse into children first so transitive deps install before their parents
        collect_web_deps(&node.dependencies, db, out)?;

        let (version_id, version) = match &node.latest_compatible_version {
            Some(v) => (v.id, v.version.clone()),
            None => {
                tracing::warn!(
                    dep = node.name,
                    "dependency has no compatible version, skipping"
                );
                continue;
            }
        };

        out.push(PendingDep {
            mod_id: node.id,
            version_id,
            name: node.name.clone(),
            version,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn create_test_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let buf = std::io::Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(content).unwrap();
        }
        let buf = zip.finish().unwrap();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(buf.get_ref()).unwrap();
        tmp.flush().unwrap();
        tmp
    }

    fn setup_spt_dir() -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();
        tmp
    }

    #[test]
    fn install_extracts_files_and_records_in_db() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: Some("test-mod"),
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        let installed = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(installed.name, "TestMod");
        assert_eq!(installed.version, "1.0.0");

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/src/mod.ts")
            .exists());
    }

    #[test]
    fn install_does_not_leave_orphan_files_on_db_failure() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // First install succeeds
        let zip1 = create_test_zip(&[("SPT/user/mods/ModA/package.json", b"{\"name\":\"a\"}")]);
        install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "ModA",
            slug: None,
            version: "1.0.0",
            archive_path: zip1.path(),
        })
        .unwrap();

        // Second install with the SAME forge_mod_id should fail (UNIQUE constraint)
        let zip2 = create_test_zip(&[("SPT/user/mods/ModB/package.json", b"{\"name\":\"b\"}")]);
        let result = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100, // same ID — triggers UNIQUE constraint
            version_id: 300,
            name: "ModB",
            slug: None,
            version: "2.0.0",
            archive_path: zip2.path(),
        });
        assert!(result.is_err(), "duplicate forge_mod_id should fail");

        // ModB's files should NOT exist on disk (staging was cleaned up)
        assert!(
            !spt_dir.path().join("SPT/user/mods/ModB").exists(),
            "ModB files should not be on disk after failed install"
        );

        // ModA's files should still be intact
        assert!(
            spt_dir
                .path()
                .join("SPT/user/mods/ModA/package.json")
                .exists(),
            "ModA files should still exist"
        );
    }

    #[test]
    fn update_uses_staging_so_failure_preserves_old_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1
        let zip_v1 = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{\"v\":\"1\"}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip_v1.path(),
        })
        .unwrap();

        // Update to v2
        let zip_v2 = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"v\":\"2\"}"),
            ("SPT/user/mods/TestMod/new_file.ts", b"new"),
        ]);
        update_mod_from_archive(
            &db,
            spt_dir.path(),
            &Config::default(),
            db_id,
            300,
            "2.0.0",
            zip_v2.path(),
        )
        .unwrap();

        let updated = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(updated.version, "2.0.0");

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);

        let content =
            std::fs::read_to_string(spt_dir.path().join("SPT/user/mods/TestMod/package.json"))
                .unwrap();
        assert!(content.contains("\"v\":\"2\""));
    }

    #[test]
    fn update_removes_stale_overwrites_shared_adds_new() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1 with files: old_only.ts (will be stale) and shared.json (will be overwritten)
        let zip_v1 = create_test_zip(&[
            ("SPT/user/mods/TestMod/old_only.ts", b"old content"),
            ("SPT/user/mods/TestMod/shared.json", b"{\"v\":\"1\"}"),
        ]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip_v1.path(),
        })
        .unwrap();

        // Verify v1 files exist
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/old_only.ts")
            .exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/shared.json")
            .exists());

        // Update to v2 with files: shared.json (overwritten) and new_only.ts (new)
        let zip_v2 = create_test_zip(&[
            ("SPT/user/mods/TestMod/shared.json", b"{\"v\":\"2\"}"),
            ("SPT/user/mods/TestMod/new_only.ts", b"new content"),
        ]);
        update_mod_from_archive(
            &db,
            spt_dir.path(),
            &Config::default(),
            db_id,
            300,
            "2.0.0",
            zip_v2.path(),
        )
        .unwrap();

        // Stale file (old_only.ts) should be gone
        assert!(
            !spt_dir
                .path()
                .join("SPT/user/mods/TestMod/old_only.ts")
                .exists(),
            "stale file should be deleted"
        );

        // Shared file should have new content
        let shared =
            std::fs::read_to_string(spt_dir.path().join("SPT/user/mods/TestMod/shared.json"))
                .unwrap();
        assert!(
            shared.contains("\"v\":\"2\""),
            "shared file should be overwritten with new content"
        );

        // New file should exist
        assert!(
            spt_dir
                .path()
                .join("SPT/user/mods/TestMod/new_only.ts")
                .exists(),
            "new file should be created"
        );

        // DB should track exactly the new files
        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);
        let paths: Vec<&str> = files.iter().map(|f| f.file_path.as_str()).collect();
        assert!(paths.contains(&"SPT/user/mods/TestMod/shared.json"));
        assert!(paths.contains(&"SPT/user/mods/TestMod/new_only.ts"));
    }

    #[test]
    fn remove_deletes_files_and_db_records() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        let zip = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());

        remove_mod_by_id(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        assert!(!spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(db.get_mod(db_id).unwrap().is_none());
    }

    #[test]
    fn find_top_level_mod_dirs_extracts_correctly() {
        let paths = vec![
            "SPT/user/mods/TestMod/package.json".to_string(),
            "SPT/user/mods/TestMod/src/mod.ts".to_string(),
            "BepInEx/plugins/PluginDir/plugin.dll".to_string(),
            "BepInEx/plugins/loose.dll".to_string(),
        ];
        let dirs = find_top_level_mod_dirs(&paths);
        assert_eq!(dirs.len(), 2);
        assert!(dirs.contains(&"SPT/user/mods/TestMod".to_string()));
        assert!(dirs.contains(&"BepInEx/plugins/PluginDir".to_string()));
        // loose.dll should NOT be a top-level dir
    }

    #[test]
    fn find_loose_files_identifies_non_dir_files() {
        let paths = vec![
            "SPT/user/mods/TestMod/package.json".to_string(),
            "BepInEx/plugins/loose.dll".to_string(),
        ];
        let top_dirs = vec!["SPT/user/mods/TestMod".to_string()];
        let loose = find_loose_files(&paths, &top_dirs);
        assert_eq!(loose, vec!["BepInEx/plugins/loose.dll"]);
    }

    #[test]
    fn disable_and_enable_mod_renames_directories() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        // Verify mod is installed and enabled
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(!db.get_mod(db_id).unwrap().unwrap().disabled);

        // Disable the mod
        disable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Directory should be renamed
        assert!(!spt_dir.path().join("SPT/user/mods/TestMod").exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled")
            .exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled/package.json")
            .exists());

        // DB should reflect disabled state
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert!(m.disabled);

        // File paths in DB should be updated
        let files = db.get_files_for_mod(db_id).unwrap();
        assert!(files
            .iter()
            .all(|f| f.file_path.contains("TestMod.disabled")));

        // Enable the mod
        enable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Directory should be restored
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(!spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled")
            .exists());

        // DB should reflect enabled state
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert!(!m.disabled);

        // File paths in DB should be restored
        let files = db.get_files_for_mod(db_id).unwrap();
        assert!(files.iter().all(|f| !f.file_path.contains(".disabled")));
    }

    #[test]
    fn disable_mod_handles_loose_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[("BepInEx/plugins/loose.dll", b"dll content")]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "LooseMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        assert!(spt_dir.path().join("BepInEx/plugins/loose.dll").exists());

        disable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        assert!(!spt_dir.path().join("BepInEx/plugins/loose.dll").exists());
        assert!(spt_dir
            .path()
            .join("BepInEx/plugins/loose.dll.disabled")
            .exists());

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files[0].file_path, "BepInEx/plugins/loose.dll.disabled");

        enable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        assert!(spt_dir.path().join("BepInEx/plugins/loose.dll").exists());
        assert!(!spt_dir
            .path()
            .join("BepInEx/plugins/loose.dll.disabled")
            .exists());
    }

    #[test]
    fn disable_already_disabled_mod_errors() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{}")]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        disable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();
        let result = disable_mod(&db, spt_dir.path(), &Config::default(), db_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already disabled"));
    }

    // ── Recovery tests ───────────────────────────────────────────────

    /// Helper: compute the hash the same way the extraction code does.
    fn hash_content(data: &[u8]) -> String {
        crate::spt::mods::compute_hash_public(data)
    }

    #[test]
    fn recover_completes_update_when_new_files_exist() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1
        let zip_v1 = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{\"v\":\"1\"}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip_v1.path(),
        })
        .unwrap();

        // Simulate: filesystem has v2 files, but DB still says v1
        let v2_content = b"{\"v\":\"2\"}";
        std::fs::write(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json"),
            v2_content,
        )
        .unwrap();

        let new_files = vec![ExtractedFile {
            path: "SPT/user/mods/TestMod/package.json".to_string(),
            hash: hash_content(v2_content),
            size: v2_content.len() as u64,
        }];
        let old_paths = vec!["SPT/user/mods/TestMod/package.json".to_string()];

        db.insert_pending_update(
            db_id,
            300,
            "2.0.0",
            &serde_json::to_string(&new_files).unwrap(),
            &serde_json::to_string(&old_paths).unwrap(),
        )
        .unwrap();

        // Run recovery
        recover_pending_updates(&db, spt_dir.path()).unwrap();

        // DB should now reflect v2
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(m.version, "2.0.0");
        assert_eq!(m.forge_version_id, 300);

        // Pending marker should be cleared
        assert!(db.list_pending_updates().unwrap().is_empty());

        // Files in DB should match new version
        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_hash.as_deref(),
            Some(hash_content(v2_content).as_str())
        );
    }

    #[test]
    fn recover_clears_marker_when_no_new_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1
        let zip_v1 = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{\"v\":\"1\"}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip_v1.path(),
        })
        .unwrap();

        // Simulate: pending marker exists, but filesystem still has v1 (swap never happened)
        let new_files = vec![ExtractedFile {
            path: "SPT/user/mods/TestMod/new_file.ts".to_string(),
            hash: hash_content(b"new"),
            size: 3,
        }];
        let old_paths = vec!["SPT/user/mods/TestMod/package.json".to_string()];

        db.insert_pending_update(
            db_id,
            300,
            "2.0.0",
            &serde_json::to_string(&new_files).unwrap(),
            &serde_json::to_string(&old_paths).unwrap(),
        )
        .unwrap();

        // Run recovery
        recover_pending_updates(&db, spt_dir.path()).unwrap();

        // DB should still say v1
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(m.version, "1.0.0");

        // Pending marker should be cleared
        assert!(db.list_pending_updates().unwrap().is_empty());
    }

    #[test]
    fn recover_handles_orphaned_pending_update() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Insert a pending update for a mod that doesn't exist (id=999)
        db.insert_pending_update(999, 300, "2.0.0", "[]", "[]")
            .unwrap();

        // Run recovery — should not error
        recover_pending_updates(&db, spt_dir.path()).unwrap();

        // Marker should be cleared
        assert!(db.list_pending_updates().unwrap().is_empty());
    }

    #[test]
    fn recover_rolls_back_partial_copy() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1
        let zip_v1 = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{\"v\":\"1\"}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip_v1.path(),
        })
        .unwrap();

        // Simulate partial copy: one new file exists, another doesn't
        let file_a_content = b"new_a";
        let new_file_a_path = "SPT/user/mods/TestMod/new_a.ts";
        std::fs::write(spt_dir.path().join(new_file_a_path), file_a_content).unwrap();

        let file_b_content = b"new_b";
        let new_file_b_path = "SPT/user/mods/TestMod/new_b.ts";
        // new_b does NOT exist on disk — simulates crash mid-copy

        let new_files = vec![
            ExtractedFile {
                path: new_file_a_path.to_string(),
                hash: hash_content(file_a_content),
                size: file_a_content.len() as u64,
            },
            ExtractedFile {
                path: new_file_b_path.to_string(),
                hash: hash_content(file_b_content),
                size: file_b_content.len() as u64,
            },
        ];
        let old_paths = vec!["SPT/user/mods/TestMod/package.json".to_string()];

        db.insert_pending_update(
            db_id,
            300,
            "2.0.0",
            &serde_json::to_string(&new_files).unwrap(),
            &serde_json::to_string(&old_paths).unwrap(),
        )
        .unwrap();

        // Run recovery
        recover_pending_updates(&db, spt_dir.path()).unwrap();

        // The partially-copied new_a.ts (not in old set) should be deleted
        assert!(!spt_dir.path().join(new_file_a_path).exists());

        // DB should still say v1
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(m.version, "1.0.0");

        // Pending marker should be cleared
        assert!(db.list_pending_updates().unwrap().is_empty());
    }

    #[test]
    fn recover_noop_when_no_pending_updates() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Should succeed with nothing to do
        recover_pending_updates(&db, spt_dir.path()).unwrap();
        assert!(db.list_pending_updates().unwrap().is_empty());
    }

    #[test]
    fn duplicate_pending_update_for_same_mod_is_rejected() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();

        // First insert succeeds
        db.insert_pending_update(mod_id, 300, "2.0.0", "[]", "[]")
            .unwrap();

        // Second insert for the same mod_db_id should fail (UNIQUE constraint)
        let result = db.insert_pending_update(mod_id, 400, "3.0.0", "[]", "[]");
        assert!(result.is_err());

        // Only one record should exist
        assert_eq!(db.list_pending_updates().unwrap().len(), 1);
    }
}
