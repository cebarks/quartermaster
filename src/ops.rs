use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::mods::InstalledMod;
use crate::db::Database;
use crate::headless_sync::{sync_client_files_to_headless, SyncOp};
use crate::spt::mods::ExtractedFile;

/// Create a staging tempdir on the same filesystem as `spt_dir` so that
/// `rename()` works instead of falling back to a byte-by-byte copy.
pub fn staging_tempdir(spt_dir: &Path) -> Result<tempfile::TempDir> {
    let staging_root = spt_dir.join("quartermaster/.staging");
    std::fs::create_dir_all(&staging_root)
        .with_context(|| format!("failed to create staging dir: {}", staging_root.display()))?;
    tempfile::tempdir_in(&staging_root).context("failed to create staging tempdir")
}

/// Remove leftover staging tempdirs from a previous crash.
/// Safe to call on startup — nothing should be actively staging yet.
pub fn cleanup_staging(spt_dir: &Path) {
    let staging_root = spt_dir.join("quartermaster/.staging");
    if let Ok(entries) = std::fs::read_dir(&staging_root) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Check if a mod is in a modsync group with `exclude_headless = true`.
pub fn is_excluded_from_headless(config: &crate::config::Config, forge_mod_id: i64) -> bool {
    config
        .modsync
        .as_ref()
        .map(|ms| {
            ms.groups
                .values()
                .any(|g| g.exclude_headless && g.members.contains(&forge_mod_id))
        })
        .unwrap_or(false)
}

/// Best-effort sync of client-side files to the headless install directory.
/// No-op if headless is not configured or the mod is in an exclude_headless group.
fn maybe_sync_headless(
    config: &crate::config::Config,
    spt_dir: &Path,
    db: &Database,
    mod_db_id: i64,
    op: SyncOp,
) {
    let install_dir = match config.headless.as_ref().map(|h| &h.install_dir) {
        Some(dir) => dir,
        None => return,
    };

    if let Some(forge_mod_id) = db.get_mod(mod_db_id).ok().flatten().map(|m| m.forge_mod_id) {
        if is_excluded_from_headless(config, forge_mod_id) {
            tracing::debug!(
                forge_mod_id,
                "headless sync: mod in exclude_headless group, skipping"
            );
            return;
        }
    }

    let files: Vec<String> = match db.get_files_for_mod(mod_db_id) {
        Ok(f) => f.into_iter().map(|f| f.file_path).collect(),
        Err(e) => {
            tracing::warn!(mod_db_id, err = %e, "headless sync: failed to read file list");
            return;
        }
    };

    if let Err(e) = sync_client_files_to_headless(spt_dir, install_dir, &files, op) {
        tracing::warn!(mod_db_id, err = %e, "headless sync failed");
    }
}

/// Sync with a pre-read file list. Used when the file list is already available
/// or when the DB record is about to be deleted (remove paths).
fn maybe_sync_headless_with_files(
    config: &crate::config::Config,
    spt_dir: &Path,
    files: &[String],
    op: SyncOp,
) {
    let install_dir = match config.headless.as_ref().map(|h| &h.install_dir) {
        Some(dir) => dir,
        None => return,
    };
    if let Err(e) = sync_client_files_to_headless(spt_dir, install_dir, files, op) {
        tracing::warn!(err = %e, "headless sync failed");
    }
}

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

fn record_extracted_addon_files(
    db: &Database,
    addon_db_id: i64,
    files: &[ExtractedFile],
) -> Result<()> {
    for file in files {
        db.insert_addon_file(
            addon_db_id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }
    Ok(())
}

/// Move extracted files from a staging directory to the live SPT directory.
///
/// For each file, attempts `rename` first (fast, same-filesystem move), falling
/// back to `copy` if the staging dir is on a different mount.
fn move_staged_files(staging_dir: &Path, spt_dir: &Path, files: &[ExtractedFile]) -> Result<()> {
    for file in files {
        let src = staging_dir.join(&file.path);
        let dst = spt_dir.join(&file.path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
    }
    Ok(())
}

/// Compute which files from `old_paths` are not present in `new_files`, delete
/// them from `spt_dir`, and log the count.
fn remove_stale_files(
    spt_dir: &Path,
    old_paths: Vec<String>,
    new_files: &[ExtractedFile],
) -> Result<()> {
    let new_paths: std::collections::HashSet<&str> =
        new_files.iter().map(|f| f.path.as_str()).collect();
    let stale_paths: Vec<String> = old_paths
        .into_iter()
        .filter(|p| !new_paths.contains(p.as_str()))
        .collect();
    if !stale_paths.is_empty() {
        tracing::debug!(stale_count = stale_paths.len(), "removing stale files");
        crate::spt::mods::delete_mod_files(spt_dir, &stale_paths)?;
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

/// Parameters for installing an addon from a downloaded archive.
#[allow(dead_code)] // Used in Task 5
pub struct InstallAddonRequest<'a> {
    pub db: &'a Database,
    pub spt_dir: &'a Path,
    pub config: &'a crate::config::Config,
    pub forge_addon_id: i64,
    pub parent_mod_id: i64,
    pub version_id: i64,
    pub name: &'a str,
    pub slug: Option<&'a str>,
    pub version: &'a str,
    pub mod_version_constraint: Option<&'a str>,
    pub archive_path: &'a Path,
}

pub fn install_mod_from_archive(req: &InstallRequest<'_>) -> Result<i64> {
    tracing::info!(
        mod_name = req.name,
        mod_id = req.forge_mod_id,
        version = req.version,
        "installing mod from archive"
    );

    // Extract to a staging directory on the same filesystem as spt_dir so
    // rename() works instead of cross-device copy.
    let staging_dir = staging_tempdir(req.spt_dir)?;
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
    move_staged_files(staging_dir.path(), req.spt_dir, &extracted)?;

    tracing::debug!(
        db_id,
        file_count = extracted.len(),
        "mod installed, files recorded"
    );
    if let Err(e) = crate::modsync::regenerate_if_enabled(req.spt_dir, req.config, req.db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    if let Some(ref ms_config) = req.config.modsync {
        if let Err(e) = crate::modsync::ensure_mod_layout(req.spt_dir, ms_config, req.db, db_id) {
            tracing::warn!(err = %e, "failed to ensure mod layout after install");
        }
    }
    maybe_sync_headless(req.config, req.spt_dir, req.db, db_id, SyncOp::Install);
    Ok(db_id)
}

#[allow(dead_code)] // Used in Task 5
pub fn install_addon_from_archive(req: &InstallAddonRequest<'_>) -> Result<i64> {
    tracing::info!(
        addon_name = req.name,
        addon_id = req.forge_addon_id,
        parent_mod_id = req.parent_mod_id,
        version = req.version,
        "installing addon from archive"
    );

    let staging_dir = staging_tempdir(req.spt_dir)?;
    let extracted = crate::spt::mods::extract_mod(req.archive_path, staging_dir.path())?;

    let tx = req.db.begin_transaction()?;
    let db_id = req.db.insert_addon(
        req.forge_addon_id,
        req.parent_mod_id,
        req.version_id,
        req.name,
        req.slug,
        req.version,
        req.mod_version_constraint,
    )?;
    record_extracted_addon_files(req.db, db_id, &extracted)?;
    tx.commit()?;

    // DB committed — now move files from staging to the live directory.
    move_staged_files(staging_dir.path(), req.spt_dir, &extracted)?;

    tracing::debug!(
        db_id,
        file_count = extracted.len(),
        "addon installed, files recorded"
    );
    if let Err(e) = crate::modsync::regenerate_if_enabled(req.spt_dir, req.config, req.db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    // Addons inherit parent mod's exclude_headless status
    {
        let parent_forge_id = req
            .db
            .get_mod(req.parent_mod_id)
            .ok()
            .flatten()
            .map(|m| m.forge_mod_id);
        let excluded = parent_forge_id
            .map(|id| is_excluded_from_headless(req.config, id))
            .unwrap_or(false);
        if !excluded {
            let files: Vec<String> = req
                .db
                .get_files_for_addon(db_id)
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.file_path)
                .collect();
            maybe_sync_headless_with_files(req.config, req.spt_dir, &files, SyncOp::Install);
        }
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
    let staging_dir = staging_tempdir(spt_dir)?;
    let extracted = crate::spt::mods::extract_mod(archive_path, staging_dir.path())?;

    let old_files = db.get_files_for_mod(mod_db_id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(
        old_file_count = old_paths.len(),
        new_file_count = extracted.len(),
        "replacing mod files"
    );
    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_update")?;

    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod not found for update"))?;
    let effective_root = resolve_mod_root(spt_dir, mod_info.disabled);

    // Compute stale paths for headless sync before remove_stale_files consumes old_paths
    let new_paths_set: std::collections::HashSet<&str> =
        extracted.iter().map(|f| f.path.as_str()).collect();
    let stale_paths_for_headless: Vec<String> = old_paths
        .iter()
        .filter(|p| !new_paths_set.contains(p.as_str()))
        .cloned()
        .collect();

    // Copy new files first (overwriting any shared with old version), so that
    // if copying fails mid-way the old files that weren't overwritten remain
    // intact. This is strictly safer than delete-all-then-copy-all.
    move_staged_files(staging_dir.path(), &effective_root, &extracted)?;
    remove_stale_files(&effective_root, old_paths, &extracted)?;

    let tx = db.begin_transaction()?;
    db.delete_files_for_mod(mod_db_id)?;
    record_extracted_files(db, mod_db_id, &extracted)?;
    db.update_mod(mod_db_id, version_id, version_str)?;
    tx.commit()?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    if let Some(ref ms_config) = config.modsync {
        if let Err(e) = crate::modsync::ensure_mod_layout(spt_dir, ms_config, db, mod_db_id) {
            tracing::warn!(err = %e, "failed to ensure mod layout after update");
        }
    }
    // Remove stale files from headless, then copy new files
    maybe_sync_headless_with_files(config, spt_dir, &stale_paths_for_headless, SyncOp::Remove);
    maybe_sync_headless(config, spt_dir, db, mod_db_id, SyncOp::Install);
    Ok(())
}

#[allow(dead_code)] // Used in Task 5
#[allow(clippy::too_many_arguments)]
pub fn update_addon_from_archive(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    addon_db_id: i64,
    version_id: i64,
    version_str: &str,
    mod_version_constraint: Option<&str>,
    archive_path: &Path,
) -> Result<()> {
    tracing::info!(
        addon_db_id,
        version = version_str,
        "updating addon from archive"
    );
    let staging_dir = staging_tempdir(spt_dir)?;
    let extracted = crate::spt::mods::extract_mod(archive_path, staging_dir.path())?;

    let old_files = db.get_files_for_addon(addon_db_id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(
        old_file_count = old_paths.len(),
        new_file_count = extracted.len(),
        "replacing addon files"
    );

    // Get addon to back up parent mod
    let addon = db
        .get_addon(addon_db_id)?
        .ok_or_else(|| anyhow::anyhow!("addon not found"))?;
    crate::backup::auto_backup_mod(
        db,
        spt_dir,
        config,
        addon.parent_mod_id,
        "auto_update_addon",
    )?;

    let effective_root = resolve_mod_root(spt_dir, addon.disabled);

    // Compute stale paths for headless sync before remove_stale_files consumes old_paths
    let new_paths_set: std::collections::HashSet<&str> =
        extracted.iter().map(|f| f.path.as_str()).collect();
    let stale_addon_paths: Vec<String> = old_paths
        .iter()
        .filter(|p| !new_paths_set.contains(p.as_str()))
        .cloned()
        .collect();

    // Copy new files first (overwriting any shared with old version), so that
    // if copying fails mid-way the old files that weren't overwritten remain
    // intact. This is strictly safer than delete-all-then-copy-all.
    move_staged_files(staging_dir.path(), &effective_root, &extracted)?;
    remove_stale_files(&effective_root, old_paths, &extracted)?;

    let tx = db.begin_transaction()?;
    db.delete_files_for_addon(addon_db_id)?;
    record_extracted_addon_files(db, addon_db_id, &extracted)?;
    db.update_addon(addon_db_id, version_id, version_str, mod_version_constraint)?;
    tx.commit()?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
    }
    // Addons inherit parent mod's exclude_headless status
    {
        let parent_forge_id = db
            .get_mod(addon.parent_mod_id)
            .ok()
            .flatten()
            .map(|m| m.forge_mod_id);
        let excluded = parent_forge_id
            .map(|id| is_excluded_from_headless(config, id))
            .unwrap_or(false);
        if !excluded {
            maybe_sync_headless_with_files(config, spt_dir, &stale_addon_paths, SyncOp::Remove);
            let new_files: Vec<String> = db
                .get_files_for_addon(addon_db_id)
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.file_path)
                .collect();
            maybe_sync_headless_with_files(config, spt_dir, &new_files, SyncOp::Install);
        }
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

    // Clone db, config, spt_dir for headless sync step (step 4) before they're moved
    let db_sync = db.clone();
    let config_sync = config.clone();
    let spt_dir_sync = spt_dir.clone();

    // Step 1: Read old file paths, auto-backup, and write pending marker (brief DB lock)
    let db_step1 = db.clone();
    let spt_dir_backup = spt_dir.clone();
    let config_backup = config;
    let version_str_step1 = version_str.clone();
    let new_files_json_step1 = new_files_json;
    let (old_paths, pending_id, is_disabled) = actix_web::web::block(move || {
        let db = db_step1.lock();
        crate::backup::auto_backup_mod(
            &db,
            &spt_dir_backup,
            &config_backup,
            mod_db_id,
            "auto_update",
        )?;
        let mod_info = db
            .get_mod(mod_db_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for update"))?;
        let is_disabled = mod_info.disabled;
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

        Ok::<_, anyhow::Error>((old_paths, pending_id, is_disabled))
    })
    .await??;

    // Step 2: Filesystem swap (no DB lock held)
    // Copy new files first, then delete stale-only old files. If copying
    // fails partway, old files that weren't overwritten remain intact.
    let spt_dir_fs = spt_dir.clone();
    let old_paths_fs = old_paths.clone();
    let (extracted, stale_paths) = actix_web::web::block(move || {
        let effective_root = resolve_mod_root(&spt_dir_fs, is_disabled);
        move_staged_files(&staging_path, &effective_root, &extracted)?;
        // Compute stale paths before remove_stale_files consumes old_paths_fs
        let new_paths_set: std::collections::HashSet<&str> =
            extracted.iter().map(|f| f.path.as_str()).collect();
        let stale: Vec<String> = old_paths_fs
            .into_iter()
            .filter(|p| !new_paths_set.contains(p.as_str()))
            .collect();
        remove_stale_files(&effective_root, old_paths, &extracted)?;
        Ok::<_, anyhow::Error>((extracted, stale))
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

    // Step 4: Headless sync (best-effort, brief DB lock)
    if result.is_ok() && config_sync.headless.is_some() {
        let _ = actix_web::web::block(move || {
            let db = db_sync.lock();
            // Remove stale files, then install new files
            maybe_sync_headless_with_files(
                &config_sync,
                &spt_dir_sync,
                &stale_paths,
                SyncOp::Remove,
            );
            maybe_sync_headless(&config_sync, &spt_dir_sync, &db, mod_db_id, SyncOp::Install);
            Ok::<_, anyhow::Error>(())
        })
        .await;
    }

    result
}

/// Apply an addon update using brief DB locks suitable for the web context.
///
/// This is the async counterpart of [`update_addon_from_archive`]: it performs the
/// same 3-step update (read old paths, filesystem swap, DB write) but splits
/// each step into a separate [`actix_web::web::block`] call so the DB mutex is
/// never held across slow filesystem I/O.
///
/// A `pending_updates` marker row is written to the database *before* any
/// destructive filesystem work begins. If the process crashes between the
/// filesystem swap and the final DB commit, [`recover_pending_updates`] will
/// detect and resolve the inconsistency on the next startup.
///
/// `extracted` must be the files already extracted to `staging_path`.
#[allow(dead_code)] // Used in Task 7
#[allow(clippy::too_many_arguments)]
pub async fn apply_addon_update(
    db: Arc<parking_lot::Mutex<Database>>,
    spt_dir: PathBuf,
    config: crate::config::Config,
    staging_path: PathBuf,
    extracted: Vec<ExtractedFile>,
    addon_db_id: i64,
    version_id: i64,
    version_str: String,
    mod_version_constraint: Option<String>,
    forge_addon_id: i64,
) -> Result<()> {
    // Serialize file metadata for the pending_updates marker
    let new_files_json =
        serde_json::to_string(&extracted).context("failed to serialize new file paths")?;

    // Clone db, config, spt_dir for headless sync step (step 4) before they're moved
    let db_sync = db.clone();
    let config_sync = config.clone();
    let spt_dir_sync = spt_dir.clone();

    // Step 1: Read old file paths, auto-backup, and write pending marker (brief DB lock)
    let db_step1 = db.clone();
    let spt_dir_backup = spt_dir.clone();
    let config_backup = config;
    let version_str_step1 = version_str.clone();
    let new_files_json_step1 = new_files_json;
    let (old_paths, pending_id, _parent_mod_id, is_disabled) = actix_web::web::block(move || {
        let db = db_step1.lock();
        let addon = db
            .get_addon(addon_db_id)?
            .ok_or_else(|| anyhow::anyhow!("addon not found"))?;
        let is_disabled = addon.disabled;
        crate::backup::auto_backup_mod(
            &db,
            &spt_dir_backup,
            &config_backup,
            addon.parent_mod_id,
            "auto_update_addon",
        )?;
        let files = db.get_files_for_addon(addon_db_id)?;
        let old_paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
        let old_files_json =
            serde_json::to_string(&old_paths).context("failed to serialize old file paths")?;

        // Write the pending marker before any destructive filesystem work
        let pending_id = db.insert_pending_addon_update(
            addon_db_id,
            version_id,
            &version_str_step1,
            &new_files_json_step1,
            &old_files_json,
            forge_addon_id,
        )?;
        tracing::debug!(
            addon_db_id,
            pending_id,
            "pending addon update marker written"
        );

        Ok::<_, anyhow::Error>((old_paths, pending_id, addon.parent_mod_id, is_disabled))
    })
    .await??;

    // Step 2: Filesystem swap (no DB lock held)
    // Copy new files first, then delete stale-only old files. If copying
    // fails partway, old files that weren't overwritten remain intact.
    let spt_dir_fs = spt_dir.clone();
    let old_paths_fs = old_paths.clone();
    let (extracted, stale_paths) = actix_web::web::block(move || {
        let effective_root = resolve_mod_root(&spt_dir_fs, is_disabled);
        move_staged_files(&staging_path, &effective_root, &extracted)?;
        // Compute stale paths before remove_stale_files consumes old_paths_fs
        let new_paths_set: std::collections::HashSet<&str> =
            extracted.iter().map(|f| f.path.as_str()).collect();
        let stale: Vec<String> = old_paths_fs
            .into_iter()
            .filter(|p| !new_paths_set.contains(p.as_str()))
            .collect();
        remove_stale_files(&effective_root, old_paths, &extracted)?;
        Ok::<_, anyhow::Error>((extracted, stale))
    })
    .await??;

    // Step 3: DB writes atomically + clear pending marker (brief DB lock)
    let db_step3 = db;
    let result = actix_web::web::block(move || {
        let db = db_step3.lock();
        let tx = db.begin_transaction()?;
        db.delete_files_for_addon(addon_db_id)?;
        record_extracted_addon_files(&db, addon_db_id, &extracted)?;
        db.update_addon(
            addon_db_id,
            version_id,
            &version_str,
            mod_version_constraint.as_deref(),
        )?;
        db.delete_pending_update(pending_id)?;
        tx.commit()?;
        tracing::debug!(
            addon_db_id,
            pending_id,
            "pending addon update marker cleared"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    if let Err(ref e) = result {
        tracing::error!(
            addon_db_id,
            pending_id,
            error = %e,
            "INCONSISTENT_STATE: filesystem updated but DB write failed for addon update. \
             A pending_updates record (id={}) exists — recovery will run on next startup.",
            pending_id
        );
    }

    // Step 4: Headless sync (best-effort, brief DB lock)
    if result.is_ok() && config_sync.headless.is_some() {
        let _ = actix_web::web::block(move || {
            let db = db_sync.lock();
            // Look up parent mod to check exclude_headless
            let excluded = db
                .get_addon(addon_db_id)
                .ok()
                .flatten()
                .and_then(|a| db.get_mod(a.parent_mod_id).ok().flatten())
                .map(|m| is_excluded_from_headless(&config_sync, m.forge_mod_id))
                .unwrap_or(false);
            if !excluded {
                // Remove stale files, then install new files
                maybe_sync_headless_with_files(
                    &config_sync,
                    &spt_dir_sync,
                    &stale_paths,
                    SyncOp::Remove,
                );
                let files: Vec<String> = db
                    .get_files_for_addon(addon_db_id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| f.file_path)
                    .collect();
                maybe_sync_headless_with_files(
                    &config_sync,
                    &spt_dir_sync,
                    &files,
                    SyncOp::Install,
                );
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
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

/// Count how many files from `new_files` exist on disk under `spt_dir` with
/// correct SHA256 hashes. Used by recovery to determine update completion state.
fn count_verified_files(spt_dir: &Path, new_files: &[ExtractedFile]) -> usize {
    let mut verified = 0usize;
    for file in new_files {
        let path = spt_dir.join(&file.path);
        if path.exists() {
            match crate::spt::mods::compute_file_hash(&path) {
                Ok(hash) => {
                    if hash == file.hash {
                        verified += 1;
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
    verified
}

/// Remove partially-copied new files that don't overlap with old files.
/// Used during recovery rollback of interrupted updates.
fn cleanup_partial_copy(spt_dir: &Path, new_files: &[ExtractedFile], old_paths: &[String]) {
    let old_set: std::collections::HashSet<&str> = old_paths.iter().map(|p| p.as_str()).collect();
    for file in new_files {
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
}

/// Result of assessing disk state during recovery of an interrupted update.
enum RecoveryOutcome {
    /// All new files present with correct hashes -- complete the DB update forward.
    AllNewPresent,
    /// Some (but not all) new files present -- partial copy needs rollback.
    PartialCopy { present: usize, total: usize },
    /// No new files, old files still present -- swap never happened.
    OldFilesIntact,
    /// Neither old nor new files found -- ambiguous state.
    Ambiguous,
}

/// Assess what state disk is in for a pending update recovery.
fn assess_recovery_state(
    spt_dir: &Path,
    new_files: &[ExtractedFile],
    old_paths: &[String],
) -> RecoveryOutcome {
    let new_files_ok = count_verified_files(spt_dir, new_files);
    let old_files_exist = old_paths
        .iter()
        .filter(|p| spt_dir.join(p).exists())
        .count();

    if new_files_ok == new_files.len() {
        RecoveryOutcome::AllNewPresent
    } else if new_files_ok > 0 && new_files_ok < new_files.len() {
        RecoveryOutcome::PartialCopy {
            present: new_files_ok,
            total: new_files.len(),
        }
    } else if old_files_exist > 0 {
        RecoveryOutcome::OldFilesIntact
    } else {
        RecoveryOutcome::Ambiguous
    }
}

fn recover_single_update(
    db: &Database,
    spt_dir: &Path,
    record: &crate::db::mods::PendingUpdate,
) -> Result<()> {
    // Route to addon or mod recovery based on item_type
    if record.item_type == "addon" {
        return recover_single_addon_update(db, spt_dir, record);
    }

    // Check if the mod row still exists
    let mod_info = db.get_mod(record.mod_db_id)?;
    if mod_info.is_none() {
        tracing::warn!(
            pending_id = record.id,
            mod_db_id = record.mod_db_id,
            "cleared orphaned pending update marker (mod row was deleted)"
        );
        db.delete_pending_update(record.id)?;
        return Ok(());
    }

    // Determine effective root: stash if disabled, canonical otherwise
    let effective_root = resolve_mod_root(spt_dir, mod_info.as_ref().is_some_and(|m| m.disabled));

    let new_files: Vec<ExtractedFile> = serde_json::from_str(&record.new_file_paths)
        .context("failed to parse new_file_paths JSON from pending_updates")?;
    let old_paths: Vec<String> = serde_json::from_str(&record.old_file_paths)
        .context("failed to parse old_file_paths JSON from pending_updates")?;

    match assess_recovery_state(&effective_root, &new_files, &old_paths) {
        RecoveryOutcome::AllNewPresent => {
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
        }
        RecoveryOutcome::PartialCopy { present, total } => {
            cleanup_partial_copy(&effective_root, &new_files, &old_paths);
            db.delete_pending_update(record.id)?;
            tracing::warn!(
                mod_db_id = record.mod_db_id,
                pending_id = record.id,
                new_present = present,
                new_total = total,
                "recovered interrupted update: rolled back partial copy. \
                 Restore from backup if mod files are inconsistent."
            );
        }
        RecoveryOutcome::OldFilesIntact => {
            db.delete_pending_update(record.id)?;
            tracing::info!(
                mod_db_id = record.mod_db_id,
                pending_id = record.id,
                "recovered interrupted update: filesystem unchanged, cleared stale marker"
            );
        }
        RecoveryOutcome::Ambiguous => {
            db.delete_pending_update(record.id)?;
            tracing::warn!(
                mod_db_id = record.mod_db_id,
                pending_id = record.id,
                "recovered interrupted update: ambiguous state (no old or new files found). \
                 Restore from backup if needed."
            );
        }
    }

    Ok(())
}

fn recover_single_addon_update(
    db: &Database,
    spt_dir: &Path,
    record: &crate::db::mods::PendingUpdate,
) -> Result<()> {
    let addon_info = db.get_addon(record.mod_db_id)?;
    if addon_info.is_none() {
        tracing::warn!(
            pending_id = record.id,
            addon_db_id = record.mod_db_id,
            "cleared orphaned pending addon update marker (addon row was deleted)"
        );
        db.delete_pending_update(record.id)?;
        return Ok(());
    }

    // Determine effective root: stash if disabled, canonical otherwise
    let effective_root = resolve_mod_root(spt_dir, addon_info.as_ref().is_some_and(|a| a.disabled));

    let new_files: Vec<ExtractedFile> = serde_json::from_str(&record.new_file_paths)
        .context("failed to parse new_file_paths JSON from pending_updates")?;
    let old_paths: Vec<String> = serde_json::from_str(&record.old_file_paths)
        .context("failed to parse old_file_paths JSON from pending_updates")?;

    match assess_recovery_state(&effective_root, &new_files, &old_paths) {
        RecoveryOutcome::AllNewPresent => {
            let tx = db.begin_transaction()?;
            db.delete_files_for_addon(record.mod_db_id)?;
            record_extracted_addon_files(db, record.mod_db_id, &new_files)?;
            // For addons, we don't have mod_version_constraint in the pending record, so pass None
            db.update_addon(
                record.mod_db_id,
                record.version_id,
                &record.version_str,
                None,
            )?;
            db.delete_pending_update(record.id)?;
            tx.commit()?;
            tracing::info!(
                addon_db_id = record.mod_db_id,
                pending_id = record.id,
                version = %record.version_str,
                "recovered interrupted addon update: completed DB update"
            );
        }
        RecoveryOutcome::PartialCopy { present, total } => {
            cleanup_partial_copy(&effective_root, &new_files, &old_paths);
            db.delete_pending_update(record.id)?;
            tracing::warn!(
                addon_db_id = record.mod_db_id,
                pending_id = record.id,
                new_present = present,
                new_total = total,
                "recovered interrupted addon update: rolled back partial copy. \
                 Restore from backup if addon files are inconsistent."
            );
        }
        RecoveryOutcome::OldFilesIntact => {
            db.delete_pending_update(record.id)?;
            tracing::info!(
                addon_db_id = record.mod_db_id,
                pending_id = record.id,
                "recovered interrupted addon update: filesystem unchanged, cleared stale marker"
            );
        }
        RecoveryOutcome::Ambiguous => {
            db.delete_pending_update(record.id)?;
            tracing::warn!(
                addon_db_id = record.mod_db_id,
                pending_id = record.id,
                "recovered interrupted addon update: ambiguous state (no old or new files found). \
                 Restore from backup if needed."
            );
        }
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

    // Remove child addons first, suppressing per-addon modsync regen
    let child_addons = db.list_addons_for_mod(mod_db_id)?;
    for addon in &child_addons {
        tracing::info!(addon_name = %addon.name, "removing child addon before parent mod removal");
        remove_addon_by_id(db, spt_dir, config, addon.id, true /* skip_modsync */)?;
    }

    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_remove")?;
    let mod_info_for_disable = db.get_mod(mod_db_id)?;
    let is_disabled = mod_info_for_disable.is_some_and(|m| m.disabled);
    let files = db.get_files_for_mod(mod_db_id)?;
    let file_paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(file_count = file_paths.len(), "deleting mod files");
    let delete_root = resolve_mod_root(spt_dir, is_disabled);
    crate::spt::mods::delete_mod_files(&delete_root, &file_paths)?;

    // Clean up empty quma-* group directories after mod removal
    for path in &file_paths {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 4
            && parts[0] == "BepInEx"
            && parts[1] == "plugins"
            && parts[2].starts_with("quma-")
        {
            let group_dir = spt_dir.join(format!("{}/{}/{}", parts[0], parts[1], parts[2]));
            if group_dir.is_dir() {
                if let Ok(mut entries) = std::fs::read_dir(&group_dir) {
                    if entries.next().is_none() {
                        let _ = std::fs::remove_dir(&group_dir);
                    }
                }
            }
        }
    }

    // Look up forge_mod_id before deletion for group cleanup and headless sync
    let forge_mod_id = db.get_mod(mod_db_id)?.map(|m| m.forge_mod_id);

    // Remove client files from headless before DB delete loses the file list
    if let Some(forge_id) = forge_mod_id {
        if !is_excluded_from_headless(config, forge_id) {
            maybe_sync_headless_with_files(config, spt_dir, &file_paths, SyncOp::Remove);
        }
    }

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

/// Remove an addon by its database ID.
///
/// This function backs up the addon files, deletes them from disk, and removes
/// the database record. When `skip_modsync` is true, the modsync regeneration
/// is suppressed — used during parent mod cascade removal to avoid redundant
/// regenerations.
pub fn remove_addon_by_id(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    addon_db_id: i64,
    skip_modsync: bool,
) -> Result<()> {
    let addon = db
        .get_addon(addon_db_id)?
        .ok_or_else(|| anyhow::anyhow!("addon not found"))?;
    tracing::info!(addon_db_id, addon_name = %addon.name, "removing addon");

    // Back up addon files (via parent mod backup for simplicity)
    crate::backup::auto_backup_mod(
        db,
        spt_dir,
        config,
        addon.parent_mod_id,
        "auto_remove_addon",
    )?;

    let files = db.get_files_for_addon(addon_db_id)?;
    let file_paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(file_count = file_paths.len(), "deleting addon files");
    let delete_root = resolve_mod_root(spt_dir, addon.disabled);
    crate::spt::mods::delete_mod_files(&delete_root, &file_paths)?;

    // Remove client files from headless before DB delete loses the file list
    {
        let parent_forge_id = db
            .get_mod(addon.parent_mod_id)
            .ok()
            .flatten()
            .map(|m| m.forge_mod_id);
        let excluded = parent_forge_id
            .map(|id| is_excluded_from_headless(config, id))
            .unwrap_or(false);
        if !excluded {
            maybe_sync_headless_with_files(config, spt_dir, &file_paths, SyncOp::Remove);
        }
    }

    let tx = db.begin_transaction()?;
    db.delete_addon(addon_db_id)?; // CASCADE deletes file records
    tx.commit()?;

    if !skip_modsync {
        if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
            tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
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

/// Root of the disabled-mod stash: `<spt_dir>/quartermaster/disabled/`.
pub fn disabled_stash_dir(spt_dir: &Path) -> PathBuf {
    spt_dir.join("quartermaster/disabled")
}

/// Resolve the filesystem root for a mod/addon based on disabled state.
/// Enabled: files at `spt_dir/<path>`. Disabled: files at `spt_dir/quartermaster/disabled/<path>`.
pub fn resolve_mod_root(spt_dir: &Path, disabled: bool) -> PathBuf {
    if disabled {
        disabled_stash_dir(spt_dir)
    } else {
        spt_dir.to_path_buf()
    }
}

/// One-time migration: convert mods disabled under the old `.disabled` suffix
/// scheme to the new stash-directory scheme. Idempotent — safe to call on
/// every startup.
pub fn migrate_disabled_to_stash(db: &Database, spt_dir: &Path) -> Result<()> {
    let all_mods = db.list_mods()?;
    let disabled_mods: Vec<_> = all_mods.into_iter().filter(|m| m.disabled).collect();
    if disabled_mods.is_empty() {
        return Ok(());
    }

    let stash = disabled_stash_dir(spt_dir);

    for m in &disabled_mods {
        let files = db.get_files_for_mod(m.id)?;
        let has_old_paths = files.iter().any(|f| f.file_path.contains(".disabled"));
        if !has_old_paths {
            continue;
        }

        tracing::info!(mod_id = m.id, mod_name = %m.name, "migrating disabled mod from .disabled scheme to stash");

        let tx = db.begin_transaction()?;
        for file in &files {
            if !file.file_path.contains(".disabled") {
                continue;
            }

            // Compute canonical path by stripping .disabled from the path
            let canonical = file
                .file_path
                .replace(".disabled/", "/")
                .replace(".disabled", "");

            // Update DB to canonical path
            db.rename_file_path(file.id, &canonical)?;

            // Move file on disk: old location → stash
            let old_on_disk = spt_dir.join(&file.file_path);
            let new_in_stash = stash.join(&canonical);

            if new_in_stash.exists() {
                // Already migrated on disk
                continue;
            }

            if old_on_disk.exists() {
                if let Some(parent) = new_in_stash.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&old_on_disk, &new_in_stash)
                    .with_context(|| format!("failed to migrate {}", old_on_disk.display()))?;
            } else {
                tracing::warn!(
                    path = %file.file_path,
                    "migration: old-scheme file missing on disk, skipping"
                );
            }
        }
        tx.commit()?;

        // Clean up empty old .disabled directories
        let old_dirs: std::collections::HashSet<String> = files
            .iter()
            .filter_map(|f| {
                let path = &f.file_path;
                path.find(".disabled")
                    .map(|pos| path[..pos + ".disabled".len()].to_string())
            })
            .collect();
        for dir in &old_dirs {
            let dir_path = spt_dir.join(dir);
            if dir_path.is_dir() {
                if let Ok(mut entries) = std::fs::read_dir(&dir_path) {
                    if entries.next().is_none() {
                        let _ = std::fs::remove_dir(&dir_path);
                    }
                }
            }
        }
    }

    // Also migrate disabled addons
    let all_addons = db.list_addons()?;
    let disabled_addons: Vec<_> = all_addons.into_iter().filter(|a| a.disabled).collect();
    for addon in &disabled_addons {
        let files = db.get_files_for_addon(addon.id)?;
        let has_old_paths = files.iter().any(|f| f.file_path.contains(".disabled"));
        if !has_old_paths {
            continue;
        }

        tracing::info!(addon_id = addon.id, addon_name = %addon.name, "migrating disabled addon from .disabled scheme to stash");

        let tx = db.begin_transaction()?;
        for file in &files {
            if !file.file_path.contains(".disabled") {
                continue;
            }

            let canonical = file
                .file_path
                .replace(".disabled/", "/")
                .replace(".disabled", "");

            db.rename_file_path(file.id, &canonical)?;

            let old_on_disk = spt_dir.join(&file.file_path);
            let new_in_stash = stash.join(&canonical);

            if new_in_stash.exists() {
                continue;
            }

            if old_on_disk.exists() {
                if let Some(parent) = new_in_stash.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&old_on_disk, &new_in_stash)
                    .with_context(|| format!("failed to migrate {}", old_on_disk.display()))?;
            } else {
                tracing::warn!(
                    path = %file.file_path,
                    "migration: old-scheme addon file missing on disk, skipping"
                );
            }
        }
        tx.commit()?;

        // Clean up empty old .disabled directories
        let old_dirs: std::collections::HashSet<String> = files
            .iter()
            .filter_map(|f| {
                let path = &f.file_path;
                path.find(".disabled")
                    .map(|pos| path[..pos + ".disabled".len()].to_string())
            })
            .collect();
        for dir in &old_dirs {
            let dir_path = spt_dir.join(dir);
            if dir_path.is_dir() {
                if let Ok(mut entries) = std::fs::read_dir(&dir_path) {
                    if entries.next().is_none() {
                        let _ = std::fs::remove_dir(&dir_path);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Split top-level directories into exclusive (safe to move as a whole) and
/// shared (other mods also have files under them — must move individual files).
/// Excludes the mod's own disabled addon files from the overlap check, but
/// enabled addon files count as overlap (to avoid orphaning them).
fn filter_shared_dirs(
    db: &Database,
    mod_db_id: i64,
    top_dirs: &[String],
) -> (Vec<String>, Vec<String>) {
    // Collect addons belonging to this mod so we can check their disabled state
    let own_addons = db.list_addons_for_mod(mod_db_id).unwrap_or_default();
    let own_addon_ids: std::collections::HashSet<i64> = own_addons.iter().map(|a| a.id).collect();

    let all_files = db.get_all_enabled_mod_files().unwrap_or_default();
    let other_files: Vec<&str> = all_files
        .iter()
        .filter(|f| {
            // Exclude this mod's own files
            if f.mod_id == Some(mod_db_id) && f.addon_id.is_none() {
                return false;
            }
            // Only exclude disabled own addons — enabled ones count as overlap
            if let Some(aid) = f.addon_id {
                if own_addon_ids.contains(&aid) {
                    let addon_disabled = own_addons.iter().any(|a| a.id == aid && a.disabled);
                    if addon_disabled {
                        return false;
                    }
                    // Enabled own addon files count as overlap
                }
            }
            true
        })
        .map(|f| f.file_path.as_str())
        .collect();

    let mut exclusive = Vec::new();
    let mut shared = Vec::new();
    for dir in top_dirs {
        let has_overlap = other_files.iter().any(|p| p.starts_with(dir.as_str()));
        if has_overlap {
            shared.push(dir.clone());
        } else {
            exclusive.push(dir.clone());
        }
    }
    (exclusive, shared)
}

/// Remove empty directories in the stash, walking upward from moved paths.
/// Stops at (never removes) the `quartermaster/disabled/` root.
fn cleanup_empty_stash_dirs(spt_dir: &Path, paths: &[String]) {
    let stash_root = disabled_stash_dir(spt_dir);
    for rel_path in paths {
        let mut dir = stash_root.join(rel_path);
        // Start from the parent of the moved item
        while let Some(parent) = dir.parent().map(|p| p.to_path_buf()) {
            if parent == stash_root || !parent.starts_with(&stash_root) {
                break;
            }
            match std::fs::read_dir(&parent) {
                Ok(mut entries) => {
                    if entries.next().is_none() {
                        let _ = std::fs::remove_dir(&parent);
                    } else {
                        break;
                    }
                }
                Err(_) => break,
            }
            dir = parent;
        }
    }
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

/// Disable a mod by moving its files to the stash directory at
/// `<spt_dir>/quartermaster/disabled/`, preserving relative paths.
/// DB file paths are not modified — they always store the canonical location.
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

    // Backup runs before the move — files are still at canonical paths
    crate::backup::auto_backup_mod(db, spt_dir, config, mod_db_id, "auto_disable")?;

    // Detect shared directories — move individual files instead of whole dirs
    let (exclusive_dirs, shared_dirs) = filter_shared_dirs(db, mod_db_id, &top_dirs);

    // For shared dirs, collect individual files to move
    let shared_files: Vec<&str> = files
        .iter()
        .filter(|f| {
            shared_dirs
                .iter()
                .any(|d| f.file_path.starts_with(d.as_str()))
        })
        .map(|f| f.file_path.as_str())
        .collect();

    // Build rename list: exclusive dirs as whole dirs, shared as individual files
    let stash = disabled_stash_dir(spt_dir);
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for dir in &exclusive_dirs {
        renames.push((spt_dir.join(dir), stash.join(dir)));
    }
    for loose_path in &loose {
        renames.push((spt_dir.join(loose_path), stash.join(loose_path)));
    }
    for file_path in &shared_files {
        renames.push((spt_dir.join(file_path), stash.join(file_path)));
    }

    // Handle stash collisions: remove stale stash entries before moving
    for (_, dst) in &renames {
        if dst.exists() {
            tracing::warn!(path = %dst.display(), "removing stale stash entry before disable");
            if dst.is_dir() {
                std::fs::remove_dir_all(dst)?;
            } else {
                std::fs::remove_file(dst)?;
            }
        }
    }

    // Create parent directories in stash
    for (_, dst) in &renames {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let tx = db.begin_transaction()?;
    db.set_mod_disabled(mod_db_id, true)?;

    let completed = rename_batch(&renames)?;

    if let Err(e) = tx.commit() {
        undo_renames(&completed);
        return Err(e.into());
    }

    tracing::info!(mod_db_id, mod_name = %mod_info.name, "mod disabled");
    maybe_sync_headless(config, spt_dir, db, mod_db_id, SyncOp::Remove);
    Ok(())
}

/// Enable a previously disabled mod by moving its files from the stash back
/// to their canonical location. DB file paths are already canonical.
///
/// Handles both whole-directory moves (exclusive dirs) and individual file
/// moves (shared dirs that were disabled per-file). Tries whole-dir rename
/// first; if the stash dir doesn't exist (per-file disable), falls back to
/// moving individual files.
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

    let stash = disabled_stash_dir(spt_dir);

    // Build renames: try whole-dir for top_dirs, fall back to per-file
    // if the stash doesn't have a whole directory (was disabled per-file)
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for dir in &top_dirs {
        let stash_dir = stash.join(dir);
        if stash_dir.is_dir() {
            renames.push((stash_dir, spt_dir.join(dir)));
        } else {
            // Per-file fallback: move individual files that belong to this dir
            for file in &files {
                if file.file_path.starts_with(dir.as_str()) {
                    let src = stash.join(&file.file_path);
                    if src.exists() {
                        renames.push((src, spt_dir.join(&file.file_path)));
                    }
                }
            }
        }
    }
    for loose_path in &loose {
        renames.push((stash.join(loose_path), spt_dir.join(loose_path)));
    }

    // Create parent directories at canonical locations
    for (_, dst) in &renames {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let tx = db.begin_transaction()?;
    db.set_mod_disabled(mod_db_id, false)?;

    let completed = rename_batch(&renames)?;

    if let Err(e) = tx.commit() {
        undo_renames(&completed);
        return Err(e.into());
    }

    // Clean up empty stash directories
    let all_paths: Vec<String> = top_dirs
        .iter()
        .cloned()
        .chain(loose.iter().map(|s| s.to_string()))
        .collect();
    cleanup_empty_stash_dirs(spt_dir, &all_paths);

    tracing::info!(mod_db_id, mod_name = %mod_info.name, "mod enabled");

    if let Some(ref ms_config) = config.modsync {
        if let Err(e) = crate::modsync::ensure_mod_layout(spt_dir, ms_config, db, mod_db_id) {
            tracing::warn!(err = %e, "failed to ensure mod layout after enable");
        }
    }
    maybe_sync_headless(config, spt_dir, db, mod_db_id, SyncOp::Install);
    Ok(())
}

/// Disable an addon by moving its files to the stash directory.
/// DB file paths are not modified.
pub fn disable_addon(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    addon_db_id: i64,
) -> Result<()> {
    let addon_info = db
        .get_addon(addon_db_id)?
        .ok_or_else(|| anyhow::anyhow!("addon not found"))?;
    if addon_info.disabled {
        anyhow::bail!("addon is already disabled");
    }

    let files = db.get_files_for_addon(addon_db_id)?;

    tracing::info!(addon_db_id, addon_name = %addon_info.name, "disabling addon");

    crate::backup::auto_backup_mod(
        db,
        spt_dir,
        config,
        addon_info.parent_mod_id,
        "auto_disable_addon",
    )?;

    // For addons, always move individual files since the parent mod likely
    // shares the same top-level directory
    let stash = disabled_stash_dir(spt_dir);
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in &files {
        let src = spt_dir.join(&file.file_path);
        let dst = stash.join(&file.file_path);
        renames.push((src, dst));
    }

    // Handle stash collisions: remove stale stash entries before moving
    for (_, dst) in &renames {
        if dst.exists() {
            tracing::warn!(path = %dst.display(), "removing stale stash entry before addon disable");
            if dst.is_dir() {
                std::fs::remove_dir_all(dst)?;
            } else {
                std::fs::remove_file(dst)?;
            }
        }
    }

    for (_, dst) in &renames {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let tx = db.begin_transaction()?;
    db.set_addon_disabled(addon_db_id, true)?;

    let completed = rename_batch(&renames)?;

    if let Err(e) = tx.commit() {
        undo_renames(&completed);
        return Err(e.into());
    }

    tracing::info!(addon_db_id, addon_name = %addon_info.name, "addon disabled");
    {
        let parent_forge_id = db
            .get_mod(addon_info.parent_mod_id)
            .ok()
            .flatten()
            .map(|m| m.forge_mod_id);
        let excluded = parent_forge_id
            .map(|id| is_excluded_from_headless(config, id))
            .unwrap_or(false);
        if !excluded {
            let file_paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
            maybe_sync_headless_with_files(config, spt_dir, &file_paths, SyncOp::Remove);
        }
    }
    Ok(())
}

/// Enable a previously disabled addon by moving files from stash back to canonical location.
pub fn enable_addon(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    addon_db_id: i64,
) -> Result<()> {
    let addon_info = db
        .get_addon(addon_db_id)?
        .ok_or_else(|| anyhow::anyhow!("addon not found"))?;
    if !addon_info.disabled {
        anyhow::bail!("addon is not disabled");
    }

    let files = db.get_files_for_addon(addon_db_id)?;
    let file_paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();

    tracing::info!(addon_db_id, addon_name = %addon_info.name, "enabling addon");

    crate::backup::auto_backup_mod(
        db,
        spt_dir,
        config,
        addon_info.parent_mod_id,
        "auto_enable_addon",
    )?;

    let stash = disabled_stash_dir(spt_dir);
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in &files {
        let src = stash.join(&file.file_path);
        let dst = spt_dir.join(&file.file_path);
        renames.push((src, dst));
    }

    for (_, dst) in &renames {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let tx = db.begin_transaction()?;
    db.set_addon_disabled(addon_db_id, false)?;

    let completed = rename_batch(&renames)?;

    if let Err(e) = tx.commit() {
        undo_renames(&completed);
        return Err(e.into());
    }

    cleanup_empty_stash_dirs(spt_dir, &file_paths);

    tracing::info!(addon_db_id, addon_name = %addon_info.name, "addon enabled");
    {
        let parent_forge_id = db
            .get_mod(addon_info.parent_mod_id)
            .ok()
            .flatten()
            .map(|m| m.forge_mod_id);
        let excluded = parent_forge_id
            .map(|id| is_excluded_from_headless(config, id))
            .unwrap_or(false);
        if !excluded {
            maybe_sync_headless_with_files(config, spt_dir, &file_paths, SyncOp::Install);
        }
    }
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
                slug: dep.slug.as_deref(),
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
    slug: Option<String>,
}

// TODO(debt): no cycle guard — if the Forge API ever returns circular deps, this
// stack-overflows. Same issue in cli::install::collect_deps_to_install. Add a
// visited set or depth limit to both.
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
            slug: node.slug.clone(),
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
    use crate::spt::mods::tests::create_test_zip;
    use tempfile::TempDir;

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
    fn disabled_stash_dir_path() {
        let spt_dir = PathBuf::from("/spt");
        assert_eq!(
            disabled_stash_dir(&spt_dir),
            PathBuf::from("/spt/quartermaster/disabled")
        );
    }

    #[test]
    fn filter_shared_dirs_detects_overlap() {
        let db = Database::open_in_memory().unwrap();

        // Mod A owns BepInEx/plugins/SharedDir/a.dll
        let mod_a = db.insert_mod(100, 200, "ModA", None, "1.0.0").unwrap();
        db.insert_file(
            mod_a,
            "BepInEx/plugins/SharedDir/a.dll",
            Some("aaa"),
            Some(100),
        )
        .unwrap();

        // Mod B also has files under BepInEx/plugins/SharedDir/
        let mod_b = db.insert_mod(101, 201, "ModB", None, "1.0.0").unwrap();
        db.insert_file(
            mod_b,
            "BepInEx/plugins/SharedDir/b.dll",
            Some("bbb"),
            Some(100),
        )
        .unwrap();

        let top_dirs = vec!["BepInEx/plugins/SharedDir".to_string()];
        let (exclusive, shared) = filter_shared_dirs(&db, mod_a, &top_dirs);
        assert!(exclusive.is_empty());
        assert_eq!(shared, vec!["BepInEx/plugins/SharedDir"]);
    }

    #[test]
    fn filter_shared_dirs_exclusive_when_no_overlap() {
        let db = Database::open_in_memory().unwrap();

        let mod_a = db.insert_mod(100, 200, "ModA", None, "1.0.0").unwrap();
        db.insert_file(
            mod_a,
            "SPT/user/mods/ModA/package.json",
            Some("aaa"),
            Some(100),
        )
        .unwrap();

        let mod_b = db.insert_mod(101, 201, "ModB", None, "1.0.0").unwrap();
        db.insert_file(
            mod_b,
            "SPT/user/mods/ModB/package.json",
            Some("bbb"),
            Some(100),
        )
        .unwrap();

        let top_dirs = vec!["SPT/user/mods/ModA".to_string()];
        let (exclusive, shared) = filter_shared_dirs(&db, mod_a, &top_dirs);
        assert_eq!(exclusive, vec!["SPT/user/mods/ModA"]);
        assert!(shared.is_empty());
    }

    #[test]
    fn disable_and_enable_mod_moves_to_stash() {
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

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(!db.get_mod(db_id).unwrap().unwrap().disabled);

        // Disable
        disable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Files moved to stash
        assert!(!spt_dir.path().join("SPT/user/mods/TestMod").exists());
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/src/mod.ts")
            .exists());

        // DB flag set, but file paths unchanged (canonical)
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert!(m.disabled);
        let files = db.get_files_for_mod(db_id).unwrap();
        assert!(files.iter().all(|f| !f.file_path.contains("quartermaster")));
        assert!(files.iter().all(|f| !f.file_path.contains(".disabled")));

        // Enable
        enable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Files restored to canonical location
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(!spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod")
            .exists());

        let m = db.get_mod(db_id).unwrap().unwrap();
        assert!(!m.disabled);
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
            .join("quartermaster/disabled/BepInEx/plugins/loose.dll")
            .exists());

        // DB paths stay canonical
        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files[0].file_path, "BepInEx/plugins/loose.dll");

        enable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        assert!(spt_dir.path().join("BepInEx/plugins/loose.dll").exists());
        assert!(!spt_dir
            .path()
            .join("quartermaster/disabled/BepInEx/plugins/loose.dll")
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

    #[test]
    fn disable_mod_shared_dir_moves_only_own_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install mod A with files under BepInEx/plugins/SharedDir/
        let zip_a = create_test_zip(&[("BepInEx/plugins/SharedDir/a.dll", b"mod a")]);
        let id_a = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "ModA",
            slug: None,
            version: "1.0.0",
            archive_path: zip_a.path(),
        })
        .unwrap();

        // Install mod B with files under the same directory
        let zip_b = create_test_zip(&[("BepInEx/plugins/SharedDir/b.dll", b"mod b")]);
        let _id_b = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 101,
            version_id: 201,
            name: "ModB",
            slug: None,
            version: "1.0.0",
            archive_path: zip_b.path(),
        })
        .unwrap();

        // Disable mod A — should not move mod B's file
        disable_mod(&db, spt_dir.path(), &Config::default(), id_a).unwrap();

        // Mod A's file in stash
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/BepInEx/plugins/SharedDir/a.dll")
            .exists());
        // Mod B's file still at canonical location
        assert!(spt_dir
            .path()
            .join("BepInEx/plugins/SharedDir/b.dll")
            .exists());
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

    #[test]
    fn remove_disabled_mod_deletes_from_stash() {
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
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/package.json")
            .exists());

        remove_mod_by_id(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Files should be gone from stash
        assert!(!spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod")
            .exists());
        // DB record gone
        assert!(db.get_mod(db_id).unwrap().is_none());
    }

    #[test]
    fn migrate_disabled_to_stash_moves_old_scheme_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Simulate old-scheme disabled mod: .disabled suffix in DB paths and on disk
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod.disabled/package.json",
            Some("abc"),
            Some(2),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        // Create old-scheme files on disk
        let old_dir = spt_dir.path().join("SPT/user/mods/TestMod.disabled");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::write(old_dir.join("package.json"), b"{}").unwrap();

        migrate_disabled_to_stash(&db, spt_dir.path()).unwrap();

        // DB paths should be canonical (stripped .disabled)
        let files = db.get_files_for_mod(mod_id).unwrap();
        assert_eq!(files[0].file_path, "SPT/user/mods/TestMod/package.json");

        // Files should be in the stash
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/package.json")
            .exists());
        // Old location should be gone
        assert!(!spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled")
            .exists());
    }

    #[test]
    fn disable_mod_handles_stash_collision() {
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

        // Pre-create stale stash entry
        let stale = spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("old_file.txt"), b"stale").unwrap();

        // Disable should succeed despite collision
        disable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Stale file should be gone, current file should be in stash
        assert!(!spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/old_file.txt")
            .exists());
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/package.json")
            .exists());
    }

    #[test]
    fn update_disabled_mod_updates_in_stash() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
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

        disable_mod(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        // Update while disabled
        let zip_v2 = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"v\":\"2\"}"),
            ("SPT/user/mods/TestMod/new_file.ts", b"new"),
        ]);
        update_mod_from_archive(
            &db,
            spt_dir.path(),
            &Config::default(),
            db_id,
            201,
            "2.0.0",
            zip_v2.path(),
        )
        .unwrap();

        // Files should be in stash, not at canonical location
        assert!(!spt_dir.path().join("SPT/user/mods/TestMod").exists());
        let stash = spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod");
        assert_eq!(
            std::fs::read_to_string(stash.join("package.json")).unwrap(),
            "{\"v\":\"2\"}"
        );
        assert!(stash.join("new_file.ts").exists());

        // DB should reflect updated version
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(m.version, "2.0.0");
        assert!(m.disabled);
    }

    #[test]
    fn migrate_disabled_to_stash_noop_when_already_migrated() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Already-migrated mod: canonical DB paths, files in stash
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/TestMod/package.json",
            Some("abc"),
            Some(2),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        let stash_path = spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod");
        std::fs::create_dir_all(&stash_path).unwrap();
        std::fs::write(stash_path.join("package.json"), b"{}").unwrap();

        // Should be a no-op — no .disabled in DB paths
        migrate_disabled_to_stash(&db, spt_dir.path()).unwrap();

        // Everything still in place
        assert!(spt_dir
            .path()
            .join("quartermaster/disabled/SPT/user/mods/TestMod/package.json")
            .exists());
        let files = db.get_files_for_mod(mod_id).unwrap();
        assert_eq!(files[0].file_path, "SPT/user/mods/TestMod/package.json");
    }
}
