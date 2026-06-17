// Podman integration is used by server lifecycle commands (tasks 15+).
#![allow(dead_code)]

use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};

pub struct PodmanClient {
    container: String,
}

fn parse_status_output(output: &str) -> bool {
    output.trim().eq_ignore_ascii_case("running")
}

impl PodmanClient {
    pub fn new(container: &str) -> Self {
        Self {
            container: container.to_string(),
        }
    }

    pub async fn is_running(&self) -> Result<bool> {
        let output = tokio::process::Command::new("podman")
            .args(["inspect", "--format", "{{.State.Status}}", &self.container])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman inspect")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no such container") || stderr.contains("not found") {
                bail!(
                    "container '{}' not found — check server_container config",
                    self.container
                );
            }
            bail!("podman inspect failed: {}", stderr.trim());
        }

        let status = String::from_utf8_lossy(&output.stdout);
        Ok(parse_status_output(&status))
    }

    pub async fn start(&self) -> Result<()> {
        let output = tokio::process::Command::new("podman")
            .args(["start", &self.container])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman start")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("podman start failed: {}", stderr.trim());
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let output = tokio::process::Command::new("podman")
            .args(["stop", &self.container])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman stop")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
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

        let status = tokio::process::Command::new("podman")
            .args(&args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to run podman logs")?;

        if !status.success() {
            bail!("podman logs exited with {}", status);
        }
        Ok(())
    }

    pub async fn detect_spt_containers(spt_dir: &Path) -> Result<Vec<String>> {
        let output = tokio::process::Command::new("podman")
            .args(["ps", "-a", "--format", "{{.Names}}\t{{.Mounts}}"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to run podman ps")?;

        if !output.status.success() {
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
}
