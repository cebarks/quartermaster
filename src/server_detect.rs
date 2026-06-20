use std::path::Path;

use anyhow::{bail, Result};

use crate::config::Config;
use crate::container::ContainerManager;
use crate::spt::detect::read_http_config;
use crate::spt::server::SptClient;

/// Check whether the SPT server is currently running.
///
/// Priority:
/// 1. If `server_container` is configured, check Podman container state
/// 2. Otherwise, attempt to ping the SPT server
/// 3. If ping fails (connection refused, timeout), assume server is stopped
pub async fn is_server_running(
    config: &Config,
    spt_dir: &Path,
    container_mgr: Option<&ContainerManager>,
) -> Result<bool> {
    if let Some(ref container) = config.server_container {
        if let Some(mgr) = container_mgr {
            return mgr.is_running(container).await;
        }
        bail!("Podman socket not available — cannot check container status");
    }

    let (host, port) = resolve_server_addr(config, spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let ping = spt_client.ping().await?;
    Ok(ping.ok)
}

/// Resolve the SPT server address from config, falling back to http.json, then defaults.
pub fn resolve_server_addr(config: &Config, spt_dir: &Path) -> (String, u16) {
    // Read http.json once, reuse for both host and port fallback
    let http_config = read_http_config(spt_dir);

    let host = config
        .server_host
        .clone()
        .or_else(|| http_config.as_ref().map(|(h, _)| h.clone()))
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let port = config
        .server_port
        .or_else(|| http_config.as_ref().map(|(_, p)| *p))
        .unwrap_or(6969);

    (host, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_addr_from_config() {
        let mut config = Config::default();
        config.server_host = Some("10.0.0.5".to_string());
        config.server_port = Some(7070);

        let (host, port) = resolve_server_addr(&config, Path::new("/nonexistent"));
        assert_eq!(host, "10.0.0.5");
        assert_eq!(port, 7070);
    }

    #[test]
    fn resolve_addr_defaults_without_http_json() {
        let config = Config::default();
        let (host, port) = resolve_server_addr(&config, Path::new("/nonexistent"));
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 6969);
    }

    #[test]
    fn resolve_addr_from_http_json() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = tmp.path();
        let configs_dir = spt.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("http.json"),
            r#"{"ip": "0.0.0.0", "port": 6970}"#,
        )
        .unwrap();

        let config = Config::default();
        let (host, port) = resolve_server_addr(&config, spt);
        assert_eq!(host, "0.0.0.0");
        assert_eq!(port, 6970);
    }

    #[test]
    fn resolve_addr_config_overrides_http_json() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = tmp.path();
        let configs_dir = spt.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("http.json"),
            r#"{"ip": "0.0.0.0", "port": 6970}"#,
        )
        .unwrap();

        let mut config = Config::default();
        config.server_host = Some("custom-host".to_string());
        // port not set in config — should fall back to http.json

        let (host, port) = resolve_server_addr(&config, spt);
        assert_eq!(host, "custom-host");
        assert_eq!(port, 6970);
    }
}
