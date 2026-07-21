use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::users::{InsertPendingOp, QueueAction};
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::forge::client::ForgeClient;

/// Determine whether a mod operation should be queued instead of applied immediately.
///
/// Returns true when: queue_changes is enabled, --force was NOT passed, and the server is running.
pub async fn should_queue(
    config: &Config,
    force: bool,
    dirs: &QumaDirs,
    container_mgr: Option<&ContainerManager>,
) -> Result<bool> {
    if !config.queue_changes || force {
        return Ok(false);
    }

    crate::server_detect::is_server_running(config, dirs, container_mgr).await
}

/// Clean up a queued archive file associated with a pending operation.
/// Ignores NotFound errors (file already removed), logs warnings for other errors.
pub fn cleanup_queued_archive(op: &crate::db::users::PendingOperation) {
    if let Some(ref path) = op.archive_path {
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path, err = %e, "failed to clean up queued archive");
            }
        }
    }
}

/// Remove orphaned archive files from the queue directory.
/// An archive is orphaned if no pending operation references it.
pub fn sweep_orphaned_archives(dirs: &QumaDirs, db: &crate::db::Database) {
    let queue_dir = dirs.queue_dir();
    if !queue_dir.exists() {
        return;
    }
    let pending = match db.list_pending_ops() {
        Ok(ops) => ops,
        Err(_) => return,
    };
    let known_paths: HashSet<String> = pending
        .iter()
        .filter_map(|op| op.archive_path.clone())
        .collect();

    if let Ok(entries) = std::fs::read_dir(&queue_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(path_str) = path.to_str() {
                if !known_paths.contains(path_str) {
                    tracing::debug!(path = path_str, "removing orphaned queued archive");
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

// ── Metadata helpers ─────────────────────────────────────────────────
// These helpers and staging functions are wired up in Tasks 3-4.

/// Build metadata JSON, merging a version string with optional extra metadata.
#[allow(dead_code)] // used in Task 3
pub(crate) fn build_metadata(version: &str, extra: Option<&str>) -> String {
    let mut map: serde_json::Map<String, serde_json::Value> = match extra {
        Some(m) => serde_json::from_str(m).unwrap_or_default(),
        None => serde_json::Map::new(),
    };
    map.insert(
        "version".to_string(),
        serde_json::Value::String(version.to_string()),
    );
    serde_json::Value::Object(map).to_string()
}

/// Build metadata for a dependency op. `queued_for` is an array of parent
/// forge_mod_ids that caused this dep to be queued. When cancelling a parent,
/// its ID is removed from the array; the dep is only cancelled when the array
/// is empty.
#[allow(dead_code)] // used in Task 3
pub(crate) fn build_dep_metadata(version: &str, parent_forge_mod_id: i64) -> String {
    serde_json::json!({
        "version": version,
        "queued_for": [parent_forge_mod_id],
    })
    .to_string()
}

/// Extract version string from pending op metadata JSON.
#[allow(dead_code)] // used in Task 5
pub fn extract_version_from_metadata(metadata: Option<&str>) -> Option<String> {
    metadata
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("version")?.as_str().map(String::from))
}

/// Detect archive extension from download URL. SPT Forge serves both .zip and .7z.
#[allow(dead_code)] // used in Task 3
pub(crate) fn archive_extension(download_url: &str) -> &'static str {
    if download_url.ends_with(".7z") {
        "7z"
    } else {
        "zip"
    }
}

// ── Staging types ────────────────────────────────────────────────────

#[allow(dead_code)] // used in Tasks 3-4
pub struct StageRequest<'a> {
    pub forge_mod_id: i64,
    pub version_id: i64,
    pub mod_name: &'a str,
    pub slug: Option<&'a str>,
    pub queued_by: Option<&'a str>,
    pub metadata: Option<&'a str>,
}

#[allow(dead_code)] // used in Tasks 3-4
pub struct StageResult {
    pub queued_count: usize, // total ops created (main + deps)
    pub dep_count: usize,
}

// ── Stage + queue: mods ──────────────────────────────────────────────

/// Download a Forge mod archive and all uninstalled dependencies to queue_dir,
/// then insert pending operations for each. If any download fails, all archives
/// from this call are cleaned up and the error is returned.
#[allow(dead_code)] // used in Tasks 3-4
pub async fn stage_and_queue_mod(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    req: &StageRequest<'_>,
) -> Result<StageResult> {
    let queue_dir = dirs.queue_dir();
    std::fs::create_dir_all(&queue_dir)?;

    // Track archives we download so we can clean up on failure
    let mut downloaded_archives: Vec<std::path::PathBuf> = Vec::new();

    let result = stage_mod_inner(forge, db, dirs, req, &mut downloaded_archives).await;

    if result.is_err() {
        for path in &downloaded_archives {
            let _ = std::fs::remove_file(path);
        }
    }

    result
}

/// Collected fields for a single pending operation ready to insert.
#[allow(dead_code)] // used in Tasks 3-4
struct StagedOp {
    action: QueueAction,
    forge_mod_id: Option<i64>,
    forge_version_id: Option<i64>,
    mod_name: String,
    metadata: String,
    item_type: &'static str,
    archive_path: std::path::PathBuf,
    source_url: String,
}

#[allow(dead_code)] // used in Tasks 3-4
async fn stage_mod_inner(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    req: &StageRequest<'_>,
    downloaded: &mut Vec<std::path::PathBuf>,
) -> Result<StageResult> {
    let queue_dir = dirs.queue_dir();

    // 1. Resolve version to get download URL
    let forge_mod = forge.get_mod(req.forge_mod_id, false).await?;
    let versions = forge.get_versions(req.forge_mod_id, None).await?;
    let version = versions
        .iter()
        .find(|v| v.id == req.version_id)
        .ok_or_else(|| {
            anyhow::anyhow!("version {} not found for {}", req.version_id, req.mod_name)
        })?;
    let download_url = version.link.as_deref().ok_or_else(|| {
        anyhow::anyhow!("no download link for {} v{}", req.mod_name, version.version)
    })?;

    // 2. Resolve dependencies
    let dep_nodes = forge
        .get_dependencies(&[(&req.forge_mod_id.to_string(), &version.version)])
        .await?;

    let mut deps_to_install = Vec::new();
    let mut skipped_conflicts = Vec::new();
    {
        let db_guard = db.lock();
        crate::cli::install::collect_deps_to_install(
            &dep_nodes,
            &db_guard,
            &mut deps_to_install,
            &mut skipped_conflicts,
        )?;
    }

    if !skipped_conflicts.is_empty() {
        tracing::warn!(
            count = skipped_conflicts.len(),
            names = ?skipped_conflicts,
            "skipped conflicting dependencies"
        );
    }

    // === DOWNLOAD PHASE ===
    // Download all archives first. If any download fails, clean up and return
    // error — DB is never touched.
    let mut staged_ops: Vec<StagedOp> = Vec::new();

    // 3. Download each uninstalled dep
    let mut visited: HashSet<i64> = HashSet::new();
    visited.insert(req.forge_mod_id);

    for dep in &deps_to_install {
        if !visited.insert(dep.mod_id) {
            continue;
        }

        // Check if already queued
        {
            let db_guard = db.lock();
            if db_guard.has_pending_op(dep.mod_id, QueueAction::Install)? {
                continue;
            }
        }

        // Resolve download URL for this dep
        let dep_versions = forge.get_versions(dep.mod_id, None).await?;
        let dep_version = dep_versions
            .iter()
            .find(|v| v.id == dep.version_id)
            .ok_or_else(|| {
                anyhow::anyhow!("version {} for dep {} not found", dep.version_id, dep.name)
            })?;
        let dep_url = dep_version.link.as_deref().ok_or_else(|| {
            anyhow::anyhow!("no download link for dep {} v{}", dep.name, dep.version)
        })?;

        // Download dep archive
        let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let dep_slug = forge
            .get_mod(dep.mod_id, false)
            .await?
            .slug
            .unwrap_or_else(|| dep.name.clone());
        let ext = archive_extension(dep_url);
        let dest = queue_dir.join(format!("{timestamp}-{dep_slug}.{ext}"));
        forge.download_file(dep_url, &dest).await?;
        downloaded.push(dest.clone());

        let dep_metadata = build_dep_metadata(&dep.version, req.forge_mod_id);

        staged_ops.push(StagedOp {
            action: QueueAction::Install,
            forge_mod_id: Some(dep.mod_id),
            forge_version_id: Some(dep.version_id),
            mod_name: dep.name.clone(),
            metadata: dep_metadata,
            item_type: "mod",
            archive_path: dest,
            source_url: dep_url.to_string(),
        });
    }

    // 4. Download main mod archive
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let slug = req
        .slug
        .unwrap_or_else(|| forge_mod.slug.as_deref().unwrap_or("mod"));
    let ext = archive_extension(download_url);
    let dest = queue_dir.join(format!("{timestamp}-{slug}.{ext}"));
    forge.download_file(download_url, &dest).await?;
    downloaded.push(dest.clone());

    let main_metadata = build_metadata(&version.version, req.metadata);

    staged_ops.push(StagedOp {
        action: QueueAction::Install,
        forge_mod_id: Some(req.forge_mod_id),
        forge_version_id: Some(req.version_id),
        mod_name: req.mod_name.to_string(),
        metadata: main_metadata,
        item_type: "mod",
        archive_path: dest,
        source_url: download_url.to_string(),
    });

    // === INSERT PHASE ===
    // All downloads succeeded. Batch-insert all ops in a single SQLite transaction.
    let dep_count = staged_ops.len() - 1; // last op is the main mod
    {
        let db_guard = db.lock();
        let tx = db_guard.conn().unchecked_transaction()?;
        for op in &staged_ops {
            db_guard.insert_pending_op(&InsertPendingOp {
                action: op.action,
                forge_mod_id: op.forge_mod_id,
                forge_version_id: op.forge_version_id,
                mod_name: &op.mod_name,
                metadata: Some(&op.metadata),
                queued_by: req.queued_by,
                item_type: op.item_type,
                forge_addon_id: None,
                archive_path: Some(op.archive_path.to_str().expect("valid UTF-8 path")),
                source: "forge",
                source_url: Some(&op.source_url),
            })?;
        }
        tx.commit()?;
    }

    Ok(StageResult {
        queued_count: staged_ops.len(),
        dep_count,
    })
}

// ── Stage + queue: updates ───────────────────────────────────────────

/// Stage a Forge mod update — downloads the new version archive and queues the update.
/// Dependencies are resolved for the new version: any uninstalled deps are downloaded
/// and queued as installs (deps before the update).
#[allow(dead_code)] // used in Tasks 3-4
pub async fn stage_and_queue_update(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    forge_mod_id: i64,
    version_id: i64,
    mod_name: &str,
    queued_by: Option<&str>,
) -> Result<StageResult> {
    let queue_dir = dirs.queue_dir();
    std::fs::create_dir_all(&queue_dir)?;

    let mut downloaded_archives: Vec<std::path::PathBuf> = Vec::new();

    let result = stage_update_inner(
        forge,
        db,
        dirs,
        forge_mod_id,
        version_id,
        mod_name,
        queued_by,
        &mut downloaded_archives,
    )
    .await;

    if result.is_err() {
        for path in &downloaded_archives {
            let _ = std::fs::remove_file(path);
        }
    }

    result
}

#[allow(dead_code, clippy::too_many_arguments)] // used in Tasks 3-4
async fn stage_update_inner(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    forge_mod_id: i64,
    version_id: i64,
    mod_name: &str,
    queued_by: Option<&str>,
    downloaded: &mut Vec<std::path::PathBuf>,
) -> Result<StageResult> {
    let queue_dir = dirs.queue_dir();

    // Resolve version
    let versions = forge.get_versions(forge_mod_id, None).await?;
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| anyhow::anyhow!("version {} not found", version_id))?;
    let download_url = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no download link for {} v{}", mod_name, version.version))?;

    // Resolve deps for the new version — queue installs for any uninstalled
    let dep_nodes = forge
        .get_dependencies(&[(&forge_mod_id.to_string(), &version.version)])
        .await?;

    let mut deps_to_install = Vec::new();
    let mut skipped = Vec::new();
    {
        let db_guard = db.lock();
        crate::cli::install::collect_deps_to_install(
            &dep_nodes,
            &db_guard,
            &mut deps_to_install,
            &mut skipped,
        )?;
    }

    // === DOWNLOAD PHASE ===
    let mut staged_ops: Vec<StagedOp> = Vec::new();

    let mut visited: HashSet<i64> = HashSet::new();
    visited.insert(forge_mod_id);

    for dep in &deps_to_install {
        if !visited.insert(dep.mod_id) {
            continue;
        }
        {
            let db_guard = db.lock();
            if db_guard.has_pending_op(dep.mod_id, QueueAction::Install)? {
                continue;
            }
        }
        let dep_versions = forge.get_versions(dep.mod_id, None).await?;
        let dep_ver = dep_versions
            .iter()
            .find(|v| v.id == dep.version_id)
            .ok_or_else(|| anyhow::anyhow!("dep version not found for {}", dep.name))?;
        let dep_url = dep_ver
            .link
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no download link for dep {}", dep.name))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let dep_mod_info = forge.get_mod(dep.mod_id, false).await?;
        let dep_slug = dep_mod_info.slug.unwrap_or_else(|| dep.name.clone());
        let ext = archive_extension(dep_url);
        let dest = queue_dir.join(format!("{timestamp}-{dep_slug}.{ext}"));
        forge.download_file(dep_url, &dest).await?;
        downloaded.push(dest.clone());

        let dep_metadata = build_dep_metadata(&dep.version, forge_mod_id);

        staged_ops.push(StagedOp {
            action: QueueAction::Install,
            forge_mod_id: Some(dep.mod_id),
            forge_version_id: Some(dep.version_id),
            mod_name: dep.name.clone(),
            metadata: dep_metadata,
            item_type: "mod",
            archive_path: dest,
            source_url: dep_url.to_string(),
        });
    }

    // Download main mod archive
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let mod_info = forge.get_mod(forge_mod_id, false).await?;
    let slug = mod_info.slug.unwrap_or_else(|| mod_name.to_string());
    let ext = archive_extension(download_url);
    let dest = queue_dir.join(format!("{timestamp}-{slug}.{ext}"));
    forge.download_file(download_url, &dest).await?;
    downloaded.push(dest.clone());

    let update_metadata = build_metadata(&version.version, None);

    staged_ops.push(StagedOp {
        action: QueueAction::Update,
        forge_mod_id: Some(forge_mod_id),
        forge_version_id: Some(version_id),
        mod_name: mod_name.to_string(),
        metadata: update_metadata,
        item_type: "mod",
        archive_path: dest,
        source_url: download_url.to_string(),
    });

    // === INSERT PHASE ===
    // All downloads succeeded. Batch-insert in a single SQLite transaction.
    let dep_count = staged_ops.len() - 1;
    {
        let db_guard = db.lock();
        let tx = db_guard.conn().unchecked_transaction()?;
        for op in &staged_ops {
            db_guard.insert_pending_op(&InsertPendingOp {
                action: op.action,
                forge_mod_id: op.forge_mod_id,
                forge_version_id: op.forge_version_id,
                mod_name: &op.mod_name,
                metadata: Some(&op.metadata),
                queued_by,
                item_type: op.item_type,
                forge_addon_id: None,
                archive_path: Some(op.archive_path.to_str().expect("valid UTF-8 path")),
                source: "forge",
                source_url: Some(&op.source_url),
            })?;
        }
        tx.commit()?;
    }

    Ok(StageResult {
        queued_count: staged_ops.len(),
        dep_count,
    })
}

// ── Stage + queue: addons ────────────────────────────────────────────

/// Stage a Forge addon install/update — downloads the archive and queues the operation.
/// Addons have no dependency resolution. Stores `parent_forge_mod_id` in metadata so
/// `apply_addon_install` can find the parent mod at apply time without a Forge API call.
#[allow(dead_code, clippy::too_many_arguments)] // used in Tasks 3-4
pub async fn stage_and_queue_addon(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    action: QueueAction,
    forge_addon_id: i64,
    version_id: i64,
    addon_name: &str,
    parent_forge_mod_id: i64,
    queued_by: Option<&str>,
) -> Result<()> {
    let queue_dir = dirs.queue_dir();
    std::fs::create_dir_all(&queue_dir)?;

    let addon = forge.get_addon(forge_addon_id, true).await?;
    let versions = addon.versions.unwrap_or_default();
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| {
            anyhow::anyhow!("version {} not found for addon {}", version_id, addon_name)
        })?;
    let download_url = version.link.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "no download link for addon {} v{}",
            addon_name,
            version.version
        )
    })?;

    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let slug = addon.slug.as_deref().unwrap_or("addon");
    let ext = archive_extension(download_url);
    let dest = queue_dir.join(format!("{timestamp}-{slug}.{ext}"));
    forge.download_file(download_url, &dest).await?;

    // Store parent_forge_mod_id and version in metadata so apply can find
    // the parent mod without a Forge API call
    let metadata = serde_json::json!({
        "version": version.version,
        "parent_forge_mod_id": parent_forge_mod_id,
    })
    .to_string();

    let db_guard = db.lock();
    db_guard.insert_pending_op(&InsertPendingOp {
        action,
        forge_mod_id: None,
        forge_version_id: Some(version_id),
        mod_name: addon_name,
        metadata: Some(&metadata),
        queued_by,
        item_type: "addon",
        forge_addon_id: Some(forge_addon_id),
        archive_path: Some(dest.to_str().expect("valid UTF-8 path")),
        source: "forge",
        source_url: Some(download_url),
    })?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn should_queue_disabled_in_config() {
        let mut config = Config::default();
        config.queue_changes = false;
        let dirs = QumaDirs::from_legacy(PathBuf::from("/nonexistent"));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(should_queue(&config, false, &dirs, None));
        assert!(!result.unwrap());
    }

    #[test]
    fn should_queue_force_overrides() {
        let config = Config::default();
        let dirs = QumaDirs::from_legacy(PathBuf::from("/nonexistent"));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(should_queue(&config, true, &dirs, None));
        assert!(!result.unwrap());
    }

    #[test]
    fn build_metadata_version_only() {
        let m = build_metadata("1.2.3", None);
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["version"], "1.2.3");
    }

    #[test]
    fn build_metadata_merges_extra() {
        let m = build_metadata("2.0.0", Some(r#"{"foo":"bar"}"#));
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["version"], "2.0.0");
        assert_eq!(v["foo"], "bar");
    }

    #[test]
    fn build_metadata_overwrites_version_in_extra() {
        let m = build_metadata("3.0.0", Some(r#"{"version":"old"}"#));
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["version"], "3.0.0");
    }

    #[test]
    fn build_dep_metadata_structure() {
        let m = build_dep_metadata("1.0.0", 42);
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["version"], "1.0.0");
        assert_eq!(v["queued_for"], serde_json::json!([42]));
    }

    #[test]
    fn extract_version_from_metadata_present() {
        let m = r#"{"version":"1.2.3","other":"stuff"}"#;
        assert_eq!(
            extract_version_from_metadata(Some(m)),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn extract_version_from_metadata_missing() {
        assert_eq!(extract_version_from_metadata(None), None);
        assert_eq!(
            extract_version_from_metadata(Some(r#"{"other":"stuff"}"#)),
            None
        );
    }

    #[test]
    fn extract_version_from_metadata_invalid_json() {
        assert_eq!(extract_version_from_metadata(Some("not json")), None);
    }

    #[test]
    fn archive_extension_detects_7z() {
        assert_eq!(archive_extension("https://example.com/mod.7z"), "7z");
    }

    #[test]
    fn archive_extension_defaults_to_zip() {
        assert_eq!(archive_extension("https://example.com/mod.zip"), "zip");
        assert_eq!(archive_extension("https://example.com/mod.tar.gz"), "zip");
        assert_eq!(archive_extension("https://example.com/mod"), "zip");
    }
}
