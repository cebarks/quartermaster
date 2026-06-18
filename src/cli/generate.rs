use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::spt::detect::detect_spt_dir;

use super::{Cli, GenerateTarget};

pub fn run(target: &GenerateTarget, cli: &Cli) -> Result<()> {
    match target {
        GenerateTarget::Systemd { install } => generate_systemd(*install, cli),
    }
}

fn generate_systemd_unit(spt_dir: &Path, config_path: &Path, config: &Config) -> String {
    let quma_path =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/quma"));

    format!(
        r#"[Unit]
Description=Quartermaster (quma) Web UI
After=network.target

[Service]
Type=simple
WorkingDirectory={working_dir}
ExecStart={exec} serve --bind {bind} --port {port} --spt-dir {spt_dir} --config {config}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
        working_dir = spt_dir.display(),
        exec = quma_path.display(),
        bind = config.web_bind,
        port = config.web_port,
        spt_dir = spt_dir.display(),
        config = config_path.display(),
    )
}

fn generate_systemd(install: bool, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let config = Config::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    let unit = generate_systemd_unit(&spt_dir, &config_path, &config);

    if install {
        let service_path = Path::new("/etc/systemd/system/quartermaster.service");
        std::fs::write(service_path, &unit).with_context(|| {
            format!(
                "failed to write {} — are you running as root?",
                service_path.display()
            )
        })?;
        println!("Wrote {}", service_path.display());

        let status = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()
            .context("failed to run systemctl daemon-reload")?;
        if !status.success() {
            bail!("systemctl daemon-reload failed");
        }

        let status = std::process::Command::new("systemctl")
            .args(["enable", "quartermaster.service"])
            .status()
            .context("failed to enable quartermaster.service")?;
        if !status.success() {
            bail!("systemctl enable failed");
        }

        println!("Service enabled. Start with: systemctl start quartermaster");
    } else {
        print!("{unit}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_contains_required_fields() {
        let config = Config {
            web_bind: "0.0.0.0".to_string(),
            web_port: 9190,
            ..Config::default()
        };

        let unit = generate_systemd_unit(
            Path::new("/opt/spt"),
            Path::new("/opt/spt/quartermaster.toml"),
            &config,
        );

        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("WorkingDirectory=/opt/spt"));
        assert!(unit.contains("--bind 0.0.0.0"));
        assert!(unit.contains("--port 9190"));
        assert!(unit.contains("--spt-dir /opt/spt"));
        assert!(unit.contains("--config /opt/spt/quartermaster.toml"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("After=network.target"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn systemd_unit_uses_custom_bind_port() {
        let config = Config {
            web_bind: "127.0.0.1".to_string(),
            web_port: 8080,
            ..Config::default()
        };

        let unit = generate_systemd_unit(
            Path::new("/srv/spt"),
            Path::new("/srv/spt/quartermaster.toml"),
            &config,
        );

        assert!(unit.contains("--bind 127.0.0.1"));
        assert!(unit.contains("--port 8080"));
        assert!(unit.contains("WorkingDirectory=/srv/spt"));
    }
}
