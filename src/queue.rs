use std::path::Path;

use anyhow::Result;

use crate::config::Config;
use crate::container::ContainerManager;

/// Determine whether a mod operation should be queued instead of applied immediately.
///
/// Returns true when: queue_changes is enabled, --force was NOT passed, and the server is running.
pub async fn should_queue(
    config: &Config,
    force: bool,
    spt_dir: &Path,
    container_mgr: Option<&ContainerManager>,
) -> Result<bool> {
    if !config.queue_changes || force {
        return Ok(false);
    }

    crate::server_detect::is_server_running(config, spt_dir, container_mgr).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_queue_disabled_in_config() {
        let mut config = Config::default();
        config.queue_changes = false;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(should_queue(
            &config,
            false,
            Path::new("/nonexistent"),
            None,
        ));
        assert!(!result.unwrap());
    }

    #[test]
    fn should_queue_force_overrides() {
        let config = Config::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(should_queue(&config, true, Path::new("/nonexistent"), None));
        assert!(!result.unwrap());
    }
}
