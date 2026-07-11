use std::collections::HashSet;

use anyhow::Result;

use crate::config::Config;
use crate::container::ContainerManager;
use crate::dirs::QumaDirs;

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
}
