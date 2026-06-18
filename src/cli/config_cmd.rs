use anyhow::{bail, Result};

use crate::config::Config;
use crate::spt::detect::detect_spt_dir;

use super::{Cli, ConfigAction};

pub fn run(action: &Option<ConfigAction>, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));

    match action {
        None => show_config(&config_path),
        Some(ConfigAction::Get { key }) => get_config(&config_path, key),
        Some(ConfigAction::Set { key, value }) => set_config(&config_path, key, value),
    }
}

fn show_config(config_path: &std::path::Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let toml_str = toml::to_string_pretty(&config)?;
    println!("{toml_str}");
    Ok(())
}

fn get_config(config_path: &std::path::Path, key: &str) -> Result<()> {
    let config = Config::load(config_path)?;
    let value = match key {
        "spt_dir" => config
            .spt_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        "forge_token" => config.forge_token.unwrap_or_default(),
        "queue_changes" => config.queue_changes.to_string(),
        "auto_drain_on_lifecycle" => config.auto_drain_on_lifecycle.to_string(),
        "session_secret" => config.session_secret,
        "server_container" => config.server_container.unwrap_or_default(),
        "server_host" => config.server_host.unwrap_or_default(),
        "server_port" => config
            .server_port
            .map(|p| p.to_string())
            .unwrap_or_default(),
        "web_bind" => config.web_bind,
        "web_port" => config.web_port.to_string(),
        _ => bail!("unknown config key: '{key}'"),
    };
    println!("{value}");
    Ok(())
}

fn set_config(config_path: &std::path::Path, key: &str, value: &str) -> Result<()> {
    let mut config = Config::load(config_path)?;
    match key {
        "spt_dir" => config.spt_dir = Some(std::path::PathBuf::from(value)),
        "forge_token" => config.forge_token = Some(value.to_string()),
        "queue_changes" => {
            config.queue_changes = value
                .parse()
                .map_err(|_| anyhow::anyhow!("expected 'true' or 'false'"))?
        }
        "auto_drain_on_lifecycle" => {
            config.auto_drain_on_lifecycle = value
                .parse()
                .map_err(|_| anyhow::anyhow!("expected 'true' or 'false'"))?
        }
        "session_secret" => config.session_secret = value.to_string(),
        "server_container" => config.server_container = Some(value.to_string()),
        "server_host" => config.server_host = Some(value.to_string()),
        "server_port" => {
            config.server_port = Some(
                value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("expected a port number"))?,
            )
        }
        "web_bind" => config.web_bind = value.to_string(),
        "web_port" => {
            config.web_port = value
                .parse()
                .map_err(|_| anyhow::anyhow!("expected a port number"))?
        }
        _ => bail!("unknown config key: '{key}'"),
    }
    config.save(config_path)?;
    println!("Set {key} = {value}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        set_config(&config_path, "web_port", "3000").unwrap();
        let reloaded = Config::load(&config_path).unwrap();
        assert_eq!(reloaded.web_port, 3000);
    }

    #[test]
    fn set_boolean_values() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        set_config(&config_path, "queue_changes", "false").unwrap();
        let reloaded = Config::load(&config_path).unwrap();
        assert!(!reloaded.queue_changes);

        set_config(&config_path, "auto_drain_on_lifecycle", "true").unwrap();
        let reloaded = Config::load(&config_path).unwrap();
        assert!(reloaded.auto_drain_on_lifecycle);
    }

    #[test]
    fn set_unknown_key_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        assert!(set_config(&config_path, "nonexistent_key", "value").is_err());
    }

    #[test]
    fn set_invalid_port_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        assert!(set_config(&config_path, "web_port", "not_a_number").is_err());
    }

    #[test]
    fn set_invalid_boolean_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        assert!(set_config(&config_path, "queue_changes", "maybe").is_err());
    }
}
