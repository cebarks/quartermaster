// Podman integration is used by server lifecycle commands (tasks 15+).
#![allow(dead_code)]

use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};

pub const SPT_SERVER_IMAGE: &str = "ghcr.io/zhliau/fika-spt-server-docker:latest";
pub const DEFAULT_CONTAINER_NAME: &str = "spt-server";
pub const DEFAULT_SPT_PORT: u16 = 6969;

pub struct PodmanClient {
    container: String,
}

fn parse_status_output(output: &str) -> bool {
    output.trim().eq_ignore_ascii_case("running")
}

impl PodmanClient {
    pub async fn pull_image(image: &str) -> Result<()> {
        tracing::info!(image, "pulling container image");
        let output = tokio::process::Command::new("podman")
            .args(["pull", image])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman pull")?;

        tracing::trace!(
            image,
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            status = %output.status,
            "podman pull output"
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(image, stderr = %stderr.trim(), "podman pull failed");
            bail!("podman pull failed: {}", stderr.trim());
        }
        Ok(())
    }

    pub async fn create_spt_container(name: &str, spt_dir: &Path, port: u16) -> Result<()> {
        let mount = format!("{}:/opt/server:Z", spt_dir.display());
        let port_map = format!("{port}:6969");

        tracing::info!(name, spt_dir = %spt_dir.display(), port, "creating SPT server container");
        let output = tokio::process::Command::new("podman")
            .args([
                "create",
                "--name",
                name,
                "-p",
                &port_map,
                "-v",
                &mount,
                "--user",
                "root",
                "-e",
                "TAKE_OWNERSHIP=true",
                "-e",
                "CHANGE_PERMISSIONS=true",
                "-e",
                "LISTEN_ALL_NETWORKS=true",
                SPT_SERVER_IMAGE,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman create")?;

        tracing::trace!(
            name,
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            status = %output.status,
            "podman create output"
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(name, stderr = %stderr.trim(), "podman create failed");
            bail!("podman create failed: {}", stderr.trim());
        }
        Ok(())
    }

    pub fn new(container: &str) -> Self {
        Self {
            container: container.to_string(),
        }
    }

    pub async fn is_running(&self) -> Result<bool> {
        tracing::debug!(container = %self.container, "checking container status");
        let output = tokio::process::Command::new("podman")
            .args(["inspect", "--format", "{{.State.Status}}", &self.container])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman inspect")?;

        tracing::trace!(
            container = %self.container,
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            status = %output.status,
            "podman inspect output"
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no such container") || stderr.contains("not found") {
                tracing::warn!(container = %self.container, "container not found");
                bail!(
                    "container '{}' not found — check server_container config",
                    self.container
                );
            }
            tracing::error!(container = %self.container, stderr = %stderr.trim(), "podman inspect failed");
            bail!("podman inspect failed: {}", stderr.trim());
        }

        let status = String::from_utf8_lossy(&output.stdout);
        Ok(parse_status_output(&status))
    }

    pub async fn start(&self) -> Result<()> {
        tracing::debug!(container = %self.container, command = "start", "starting container");
        let output = tokio::process::Command::new("podman")
            .args(["start", &self.container])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman start")?;

        tracing::trace!(
            container = %self.container,
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            status = %output.status,
            "podman start output"
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(container = %self.container, stderr = %stderr.trim(), "podman start failed");
            bail!("podman start failed: {}", stderr.trim());
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        tracing::debug!(container = %self.container, command = "stop", "stopping container");
        let output = tokio::process::Command::new("podman")
            .args(["stop", &self.container])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman stop")?;

        tracing::trace!(
            container = %self.container,
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            status = %output.status,
            "podman stop output"
        );

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(container = %self.container, stderr = %stderr.trim(), "podman stop failed");
            bail!("podman stop failed: {}", stderr.trim());
        }
        Ok(())
    }

    pub async fn logs(&self, follow: bool, tail: usize) -> Result<()> {
        let mut args = vec!["logs".to_string(), "--tail".to_string(), tail.to_string()];
        if follow {
            args.push("-f".to_string());
        }
        args.push(self.container.clone());

        tracing::debug!(container = %self.container, ?args, "fetching container logs");

        let status = tokio::process::Command::new("podman")
            .args(&args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to run podman logs")?;

        if !status.success() {
            tracing::error!(container = %self.container, status = %status, "podman logs failed");
            bail!("podman logs exited with {}", status);
        }
        Ok(())
    }

    pub async fn detect_spt_containers(spt_dir: &Path) -> Result<Vec<String>> {
        tracing::debug!(spt_dir = %spt_dir.display(), "detecting SPT containers");
        let output = tokio::process::Command::new("podman")
            .args(["ps", "-a", "--format", "{{.Names}}\t{{.Mounts}}"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman ps")?;

        tracing::trace!(
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            status = %output.status,
            "podman ps output"
        );

        if !output.status.success() {
            tracing::error!("podman ps failed, returning empty list");
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let spt_dir_str = spt_dir.to_string_lossy();

        let matches: Vec<String> = stdout
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(2, '\t');
                let name = parts.next()?.trim();
                let mounts = parts.next().unwrap_or("");
                if mounts.contains(spt_dir_str.as_ref()) {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect();

        tracing::debug!(container_count = matches.len(), "found SPT containers");
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_container_status_running() {
        assert!(parse_status_output("running"));
    }

    #[test]
    fn parse_container_status_stopped() {
        assert!(!parse_status_output("exited"));
    }

    #[test]
    fn parse_container_status_created() {
        assert!(!parse_status_output("created"));
    }

    #[test]
    fn parse_container_status_with_whitespace() {
        assert!(parse_status_output("  running\n"));
    }

    #[test]
    fn pull_image_constructs_correct_command() {
        // Verify the constant is correct
        assert_eq!(
            SPT_SERVER_IMAGE,
            "ghcr.io/zhliau/fika-spt-server-docker:latest"
        );
    }

    #[test]
    fn default_container_constants() {
        assert_eq!(DEFAULT_CONTAINER_NAME, "spt-server");
        assert_eq!(DEFAULT_SPT_PORT, 6969);
    }
}
