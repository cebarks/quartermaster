# Phase 3: Server & Queue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Podman container management, server-running detection, the change queue system, `quma apply` / `quma server` / `quma status` commands, and wire queue integration into the existing install/update/remove commands.

**Architecture:** `src/podman.rs` wraps Podman CLI for container state and lifecycle. `src/spt/server.rs` provides an `SptClient` that talks to the running SPT server via HTTPS (self-signed cert, `responsecompressed: 0` header). `src/server_detect.rs` combines Podman state + ping fallback into a single `is_server_running()` check. `src/queue.rs` ties server detection to the pending_operations DB table and provides queue/drain helpers. CLI commands in `src/cli/apply.rs`, `src/cli/server.rs`, and `src/cli/status.rs` use these layers. Finally, install/update/remove are retrofitted with queue-or-apply logic.

**Tech Stack:** Rust, `tokio::process::Command` (for Podman CLI), `reqwest` (with `danger_accept_invalid_certs` for SPT server), existing `rusqlite` DB layer.

**Spec:** `SPEC.md` in the project root is the authoritative reference. Read it before starting any task.

## Global Constraints

- SPT 4.0+ only, Linux only for v1
- SPT server uses HTTPS with a self-signed TLS certificate on port 6969 (default)
- SPT server responses are zlib-compressed by default — send `responsecompressed: 0` header for raw JSON
- Server detection: Podman container state first, `/launcher/ping` fallback
- All `rusqlite` operations use the existing `Database` type from `src/db/mod.rs`
- Existing `pending_operations` DB table and CRUD methods already exist in `src/db/users.rs`
- Binary name is `quma`, async runtime is `tokio`
- Work on a feature branch `feature/phase-3-server-queue` branched from `main`

---

## Task 13: Podman Integration & Server Detection

**Files:**
- Create: `src/podman.rs`
- Create: `src/spt/server.rs`
- Create: `src/server_detect.rs`
- Modify: `src/spt/mod.rs` (add `pub mod server;`)
- Modify: `src/main.rs` (add `mod podman; mod server_detect;`)

**Interfaces:**
- Consumes: `Config` from `src/config.rs` (fields: `server_container`, `server_host`, `server_port`)
- Consumes: `read_http_config(spt_dir)` from `src/spt/detect.rs`
- Produces:
  - `PodmanClient::new(container: &str) -> Self`
  - `PodmanClient::is_running(&self) -> Result<bool>`
  - `PodmanClient::start(&self) -> Result<()>`
  - `PodmanClient::stop(&self) -> Result<()>`
  - `PodmanClient::logs(&self, follow: bool, tail: usize) -> Result<()>` (streams to stdout)
  - `PodmanClient::detect_spt_containers(spt_dir: &Path) -> Result<Vec<String>>`
  - `SptClient::new(host: &str, port: u16) -> Result<Self>`
  - `SptClient::ping(&self) -> Result<PingResult>` where `PingResult { ok: bool, latency_ms: u64 }`
  - `SptClient::server_version(&self) -> Result<String>`
  - `SptClient::loaded_server_mods(&self) -> Result<HashMap<String, serde_json::Value>>`
  - `is_server_running(config: &Config, spt_dir: &Path) -> Result<bool>`

- [ ] **Step 1: Create feature branch**

```bash
git checkout -b feature/phase-3-server-queue main
```

- [ ] **Step 2: Write failing test — `PodmanClient::is_running` parses `podman inspect` output**

In `src/podman.rs`:

```rust
use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};

pub struct PodmanClient {
    container: String,
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
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test podman::tests::parse_container_status_running`
Expected: FAIL — `parse_status_output` doesn't exist.

- [ ] **Step 4: Implement `parse_status_output` and `PodmanClient`**

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test podman::tests`
Expected: PASS

- [ ] **Step 6: Write `src/spt/server.rs` — SptClient for talking to running SPT server**

```rust
use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};

pub struct PingResult {
    pub ok: bool,
    pub latency_ms: u64,
}

pub struct SptClient {
    client: reqwest::Client,
    base_url: String,
}

impl SptClient {
    pub fn new(host: &str, port: u16) -> Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .context("failed to build SPT HTTP client")?;

        Ok(Self {
            client,
            base_url: format!("https://{}:{}", host, port),
        })
    }

    pub async fn ping(&self) -> Result<PingResult> {
        let start = Instant::now();
        let resp = self
            .client
            .get(format!("{}/launcher/ping", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => Ok(PingResult {
                ok: true,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
            Ok(_) => Ok(PingResult {
                ok: false,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
            Err(_) => Ok(PingResult {
                ok: false,
                latency_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }

    pub async fn server_version(&self) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/launcher/server/version", self.base_url))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach SPT server for version")?
            .error_for_status()
            .context("SPT server version endpoint returned error")?;

        let body = resp
            .text()
            .await
            .context("failed to read version response")?;
        // Response is a JSON string like "\"4.0.13\"" — strip outer quotes
        let version = body.trim().trim_matches('"').to_string();
        Ok(version)
    }

    pub async fn loaded_server_mods(&self) -> Result<HashMap<String, serde_json::Value>> {
        let resp = self
            .client
            .get(format!(
                "{}/launcher/server/loadedServerMods",
                self.base_url
            ))
            .header("responsecompressed", "0")
            .send()
            .await
            .context("failed to reach SPT server for loaded mods")?
            .error_for_status()
            .context("SPT loaded mods endpoint returned error")?;

        let mods: HashMap<String, serde_json::Value> = resp
            .json()
            .await
            .context("failed to parse loaded mods response")?;
        Ok(mods)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spt_client_constructs_base_url() {
        let client = SptClient::new("192.168.1.10", 6969).unwrap();
        assert_eq!(client.base_url(), "https://192.168.1.10:6969");
    }

    #[test]
    fn spt_client_localhost() {
        let client = SptClient::new("127.0.0.1", 6969).unwrap();
        assert_eq!(client.base_url(), "https://127.0.0.1:6969");
    }

    #[test]
    fn spt_client_custom_port() {
        let client = SptClient::new("10.0.0.1", 7070).unwrap();
        assert_eq!(client.base_url(), "https://10.0.0.1:7070");
    }
}
```

- [ ] **Step 7: Update `src/spt/mod.rs` to include server module**

```rust
pub mod detect;
pub mod mods;
pub mod server;
```

- [ ] **Step 8: Write `src/server_detect.rs` — unified server detection**

```rust
use std::path::Path;

use anyhow::Result;

use crate::config::Config;
use crate::podman::PodmanClient;
use crate::spt::detect::read_http_config;
use crate::spt::server::SptClient;

/// Check whether the SPT server is currently running.
///
/// Priority:
/// 1. If `server_container` is configured, check Podman container state
/// 2. Otherwise, attempt to ping the SPT server
/// 3. If ping fails (connection refused, timeout), assume server is stopped
pub async fn is_server_running(config: &Config, spt_dir: &Path) -> Result<bool> {
    if let Some(ref container) = config.server_container {
        let podman = PodmanClient::new(container);
        return podman.is_running().await;
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
        let configs_dir = spt.join("SPT_Data/Server/configs");
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
        let configs_dir = spt.join("SPT_Data/Server/configs");
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
```

- [ ] **Step 9: Update `src/main.rs` to add new modules**

Add `mod podman;` and `mod server_detect;` after the existing module declarations.

- [ ] **Step 10: Run all tests to verify everything compiles and passes**

Run: `cargo test`
Expected: all tests pass (new tests + existing).

- [ ] **Step 11: Commit**

```bash
git add src/podman.rs src/spt/server.rs src/spt/mod.rs src/server_detect.rs src/main.rs
git commit -m "feat: Podman integration, SPT server client, and unified server detection"
```

---

## Task 14: Change Queue, Apply Command & Install/Update/Remove Retrofit

**Files:**
- Create: `src/queue.rs`
- Create: `src/cli/apply.rs`
- Modify: `src/main.rs` (add `mod queue;`, wire `Command::Apply` dispatch)
- Modify: `src/cli/mod.rs` (add `pub mod apply;`)
- Modify: `src/cli/install.rs` (extract shared install logic, add queue check)
- Modify: `src/cli/update.rs` (add queue drain + queue check)
- Modify: `src/cli/remove.rs` (make async, add queue check)

**Interfaces:**
- Consumes: `is_server_running(config, spt_dir)` from `src/server_detect.rs`
- Consumes: `Database` methods: `insert_pending_op`, `list_pending_ops`, `delete_pending_op` from `src/db/users.rs`
- Consumes: `CliContext` from `src/cli/common.rs`
- Produces:
  - `queue::should_queue(config: &Config, force: bool, spt_dir: &Path) -> Result<bool>` — returns true if the operation should be queued instead of applied
  - `cli::apply::run(force: bool, ctx: &CliContext) -> Result<()>` — interactive apply with confirmation prompt
  - `cli::apply::drain_all(ctx: &CliContext) -> Result<usize>` — apply all pending ops without prompting, returns count applied
  - `cli::install::install_with_deps(ctx: &CliContext, forge_mod_id: i64, version_id: i64) -> Result<()>` — shared install+deps logic used by both `install::run` and `apply::drain_all`

- [ ] **Step 1: Write failing test — `should_queue` returns false when `queue_changes` is false**

In `src/queue.rs`:

```rust
use std::path::Path;

use anyhow::Result;

use crate::config::Config;

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
        let result = rt.block_on(should_queue(&config, false, Path::new("/nonexistent")));
        assert!(!result.unwrap());
    }

    #[test]
    fn should_queue_force_overrides() {
        let config = Config::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(should_queue(&config, true, Path::new("/nonexistent")));
        assert!(!result.unwrap());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test queue::tests::should_queue_disabled_in_config`
Expected: FAIL — `should_queue` doesn't exist.

- [ ] **Step 3: Implement `should_queue`**

```rust
/// Determine whether a mod operation should be queued instead of applied immediately.
///
/// Returns true when: queue_changes is enabled, --force was NOT passed, and the server is running.
pub async fn should_queue(config: &Config, force: bool, spt_dir: &Path) -> Result<bool> {
    if !config.queue_changes || force {
        return Ok(false);
    }

    crate::server_detect::is_server_running(config, spt_dir).await
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test queue::tests`
Expected: PASS

- [ ] **Step 5: Add `mod queue;` to `src/main.rs`**

Add `mod queue;` after the existing module declarations.

- [ ] **Step 6: Extract shared install logic from `src/cli/install.rs`**

Rename the existing `run` to keep the CLI entry point, and extract the "resolve deps + install everything" logic into `install_with_deps`:

```rust
/// Resolve dependencies and install a mod plus all its deps.
/// Used by both the interactive `install` command and `apply::drain_all`.
pub async fn install_with_deps(
    ctx: &CliContext,
    forge_mod_id: i64,
    version_id: i64,
) -> Result<()> {
    let forge_mod = ctx.forge.get_mod(forge_mod_id, false).await?;

    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        println!(
            "  {} already installed (v{}), skipping",
            existing.name, existing.version
        );
        return Ok(());
    }

    let versions = ctx
        .forge
        .get_versions(forge_mod_id, None)
        .await?;

    let selected = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| anyhow::anyhow!(
            "version ID {} not found for {} on Forge",
            version_id, forge_mod.name
        ))?
        .clone();

    let to_install = resolve_deps(ctx, &forge_mod, &selected).await?;
    install_deps(ctx, &to_install).await?;
    let db_id = install_main_mod(ctx, &forge_mod, &selected).await?;
    record_dependency_edges(ctx, db_id, &to_install)?;

    println!(
        "  {} v{} installed successfully.",
        forge_mod.name, selected.version
    );
    Ok(())
}
```

Update `run` to call `install_with_deps` after the confirmation+queue check:

```rust
pub async fn run(mod_ref: &str, force: bool, ctx: &CliContext) -> Result<()> {
    let forge_mod = resolve_mod(&ctx.forge, mod_ref).await?;
    println!("Found: {} (ID: {})", forge_mod.name, forge_mod.id);

    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        bail!(
            "{} is already installed (version {}). Use `quma update` to update it.",
            existing.name,
            existing.version
        );
    }

    let selected_version = pick_version(ctx, &forge_mod).await?;
    check_fika_compat(&forge_mod.name, &selected_version)?;

    let to_install = resolve_deps(ctx, &forge_mod, &selected_version).await?;
    display_install_plan(&forge_mod.name, &selected_version.version, &to_install);

    if !confirm("Proceed with installation?")? {
        println!("Installation cancelled.");
        return Ok(());
    }

    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir).await? {
        ctx.db.insert_pending_op(
            "install",
            forge_mod.id,
            Some(selected_version.id),
            &forge_mod.name,
            None,
            None,
        )?;
        println!(
            "Server is running — operation queued. Run `quma apply` when the server is stopped."
        );
        return Ok(());
    }

    if force {
        let running = crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running {
            println!(
                "Warning: applying changes while the server is running may cause instability."
            );
        }
    }

    install_deps(ctx, &to_install).await?;
    let db_id = install_main_mod(ctx, &forge_mod, &selected_version).await?;
    record_dependency_edges(ctx, db_id, &to_install)?;

    println!(
        "\n{} v{} installed successfully.",
        forge_mod.name, selected_version.version
    );
    Ok(())
}
```

- [ ] **Step 7: Write `src/cli/apply.rs` — the `quma apply` command with `drain_all`**

```rust
use anyhow::{bail, Result};

use super::common::CliContext;

/// Interactive apply — prompts for confirmation, then drains the queue.
pub async fn run(force: bool, ctx: &CliContext) -> Result<()> {
    if !force {
        let running =
            crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running {
            bail!(
                "SPT server is running — stop it first or use --force.\n\
                 Applying changes while the server is running may cause instability."
            );
        }
    }

    let pending = ctx.db.list_pending_ops()?;
    if pending.is_empty() {
        println!("No pending operations to apply.");
        return Ok(());
    }

    println!("Pending operations ({}):", pending.len());
    for op in &pending {
        println!(
            "  {} {} (Forge ID: {}){}",
            op.action,
            op.mod_name,
            op.forge_mod_id,
            op.forge_version_id
                .map(|v| format!(", version ID: {v}"))
                .unwrap_or_default()
        );
    }

    if !super::common::confirm("Apply all pending operations?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let applied = drain_all(ctx).await?;
    println!("\n{applied} operation(s) applied.");
    Ok(())
}

/// Apply all pending operations without prompting for confirmation.
/// Returns the number of operations successfully applied.
/// Used by `quma apply` (after confirmation) and auto-drain (server lifecycle).
pub async fn drain_all(ctx: &CliContext) -> Result<usize> {
    let pending = ctx.db.list_pending_ops()?;
    let mut applied = 0;

    for op in &pending {
        println!("  Applying: {} {}...", op.action, op.mod_name);
        match op.action.as_str() {
            "install" => {
                if let Some(version_id) = op.forge_version_id {
                    crate::cli::install::install_with_deps(ctx, op.forge_mod_id, version_id)
                        .await?;
                } else {
                    println!("    Skipped — no version ID for install operation");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            "remove" => {
                if let Some(installed) = ctx.db.get_mod_by_forge_id(op.forge_mod_id)? {
                    let files = ctx.db.get_files_for_mod(installed.id)?;
                    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
                    crate::spt::mods::delete_mod_files(&ctx.spt_dir, &paths)?;
                    ctx.db.delete_mod(installed.id)?;
                    println!("    Removed {} ({} files)", op.mod_name, paths.len());
                } else {
                    println!("    Skipped — {} not found in database", op.mod_name);
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            "update" => {
                if let (Some(installed), Some(version_id)) = (
                    ctx.db.get_mod_by_forge_id(op.forge_mod_id)?,
                    op.forge_version_id,
                ) {
                    crate::cli::update::apply_update_by_version(
                        ctx, &installed, version_id,
                    )
                    .await?;
                } else {
                    println!("    Skipped — mod not found or no version ID");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            other => {
                println!("    Skipped — unknown action: {other}");
                ctx.db.delete_pending_op(op.id)?;
                continue;
            }
        }

        ctx.db.delete_pending_op(op.id)?;
        applied += 1;
    }

    Ok(applied)
}
```

- [ ] **Step 8: Extract shared update logic from `src/cli/update.rs`**

Extract the download/extract/swap logic from `apply_single_update` into a reusable function `apply_update_by_version` that takes a version ID directly:

```rust
/// Download, extract, and swap files for a specific version.
/// Used by both the interactive `update` command and `apply::drain_all`.
pub async fn apply_update_by_version(
    ctx: &CliContext,
    installed: &InstalledMod,
    target_version_id: i64,
) -> Result<bool> {
    let versions = ctx.forge.get_versions(installed.forge_mod_id, None).await?;
    let version_info = match versions.iter().find(|v| v.id == target_version_id) {
        Some(v) => v,
        None => {
            println!(
                "    Skipping {} — version {} not found",
                installed.name, target_version_id
            );
            return Ok(false);
        }
    };

    let download_url = match &version_info.link {
        Some(url) => url.clone(),
        None => {
            println!(
                "    Skipping {} — no download link for v{}",
                installed.name, version_info.version
            );
            return Ok(false);
        }
    };

    println!(
        "  Updating {} to v{}...",
        installed.name, version_info.version
    );

    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    ctx.forge.download_file(&download_url, &archive_path).await?;

    let staging_dir = tempfile::tempdir()?;
    let new_files = extract_mod(&archive_path, staging_dir.path())?;

    let old_files = ctx.db.get_files_for_mod(installed.id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    delete_mod_files(&ctx.spt_dir, &old_paths)?;
    ctx.db.delete_files_for_mod(installed.id)?;

    for file in &new_files {
        let src = staging_dir.path().join(&file.path);
        let dest = ctx.spt_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dest)
            .or_else(|_| std::fs::copy(&src, &dest).map(|_| ()))?;
    }

    for file in &new_files {
        ctx.db.insert_file(
            installed.id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }

    ctx.db
        .update_mod(installed.id, target_version_id, &version_info.version)?;
    println!(
        "    Updated {} files for {}",
        new_files.len(),
        installed.name
    );
    Ok(true)
}
```

Then update `apply_single_update` to call `apply_update_by_version`:

```rust
async fn apply_single_update(
    update_result: &crate::forge::models::UpdateCheckResult,
    mods: &[InstalledMod],
    ctx: &CliContext,
) -> Result<bool> {
    let installed = mods
        .iter()
        .find(|m| m.forge_mod_id == update_result.mod_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "update result references unknown mod ID {}",
                update_result.mod_id
            )
        })?;

    let latest_version_id = match update_result.latest_version_id {
        Some(id) => id,
        None => {
            println!(
                "  Skipping {} — no version ID in update response",
                installed.name
            );
            return Ok(false);
        }
    };

    apply_update_by_version(ctx, installed, latest_version_id).await
}
```

- [ ] **Step 9: Add queue drain + queue check to `src/cli/update.rs`**

At the top of `update::run`, before checking for updates, add the drain logic:

```rust
pub async fn run(mod_ref: Option<&str>, force: bool, ctx: &CliContext) -> Result<()> {
    // Spec: `quma update` drains pending operations before checking for updates
    let pending = ctx.db.list_pending_ops()?;
    if !pending.is_empty() {
        let running =
            crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running && !force {
            anyhow::bail!(
                "{} pending operation(s) queued — stop the server first or use --force.\n\
                 Run `quma apply` to apply pending operations.",
                pending.len()
            );
        }
        println!(
            "Draining {} pending operation(s) before checking updates...",
            pending.len()
        );
        crate::cli::apply::drain_all(ctx).await?;
    }

    // ... rest of existing run() body ...
```

Before calling `apply_single_update` in the update loop, add the queue check:

```rust
    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir).await? {
        for update_result in &updatable {
            let installed = mods_to_check
                .iter()
                .find(|m| m.forge_mod_id == update_result.mod_id);
            if let Some(m) = installed {
                ctx.db.insert_pending_op(
                    "update",
                    m.forge_mod_id,
                    update_result.latest_version_id,
                    &m.name,
                    None,
                    None,
                )?;
            }
        }
        println!(
            "Server is running — {} update(s) queued. Run `quma apply` when the server is stopped.",
            updatable.len()
        );
        return Ok(());
    }

    if force {
        let running = crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running {
            println!("Warning: applying changes while the server is running may cause instability.");
        }
    }
```

- [ ] **Step 10: Add queue check to `src/cli/remove.rs` and make it async**

Change the function signature from `pub fn run(...)` to `pub async fn run(...)`:

```rust
pub async fn run(mod_ref: &str, force: bool, ctx: &CliContext) -> Result<()> {
    let installed = resolve_installed_mod(mod_ref, ctx)?;

    // Check if we should queue instead of applying
    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir).await? {
        ctx.db.insert_pending_op(
            "remove",
            installed.forge_mod_id,
            None,
            &installed.name,
            None,
            None,
        )?;
        println!(
            "Server is running — removal of {} queued. Run `quma apply` when the server is stopped.",
            installed.name
        );
        return Ok(());
    }

    if force {
        let running = crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running {
            println!("Warning: applying changes while the server is running may cause instability.");
        }
    }

    // ... rest of existing run() body unchanged (reverse dep check, remove_single_mod, etc.) ...
```

Update `src/main.rs` to add `.await` to the `remove` dispatch:

```rust
Command::Remove { mod_ref, force } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::remove::run(mod_ref, *force, &ctx).await
}
```

- [ ] **Step 11: Update `src/cli/mod.rs` to include `apply`**

Add `pub mod apply;` to the module declarations.

- [ ] **Step 12: Wire `Command::Apply` dispatch in `src/main.rs`**

Replace `Command::Apply { .. } => todo!("apply")` with:
```rust
Command::Apply { force } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::apply::run(*force, &ctx).await
}
```

- [ ] **Step 13: Run all tests**

Run: `cargo test`
Expected: all pass. Existing tests don't exercise the queue path (no server running in test environment, so `should_queue` returns false). The `_force` parameter is now `force` (no underscore) in install/update/remove.

Run: `cargo clippy -- -D warnings`
Expected: no warnings.

- [ ] **Step 14: Commit**

```bash
git add src/queue.rs src/cli/apply.rs src/cli/mod.rs src/main.rs src/cli/install.rs src/cli/update.rs src/cli/remove.rs
git commit -m "feat: change queue system with apply command, queue integration in install/update/remove"
```

---

## Task 16: Health Checks (Status Command)

**Note:** This task is implemented BEFORE Task 15 (server lifecycle) because Task 15's `ServerAction::Status` arm calls `cli::status::run`.

**Files:**
- Create: `src/health.rs`
- Create: `src/cli/status.rs`
- Modify: `src/main.rs` (add `mod health;`, wire `Command::Status` dispatch)
- Modify: `src/cli/mod.rs` (add `pub mod status;`)

**Interfaces:**
- Consumes: `SptClient` from `src/spt/server.rs` (`ping`, `server_version`)
- Consumes: `ForgeClient::check_updates` from `src/forge/client.rs`
- Consumes: `Database` methods: `list_mods`, `get_all_tracked_files`
- Consumes: `spt::mods::compute_file_hash`, `spt::mods::scan_mod_directories` from `src/spt/mods.rs`
- Consumes: `resolve_server_addr` from `src/server_detect.rs`
- Consumes: `SptInfo` from `src/spt/detect.rs`
- Produces:
  - `health::HealthReport` — struct with all check results
  - `health::run_checks(ctx: &CliContext) -> Result<HealthReport>`
  - `cli::status::run(json: bool, ctx: &CliContext) -> Result<()>` — prints report, sets exit code

**Note on loaded-mod verification:** The `/launcher/server/loadedServerMods` endpoint only returns server-side mods. Comparing against all installed mods would false-positive on BepInEx client plugins. This check is deferred to a future phase when we track mod type (server vs client) in the DB. For now, the health check reports installed count and checks for updates/compatibility but does NOT compare loaded vs installed.

- [ ] **Step 1: Write `src/health.rs` — health check data types and exit code logic**

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub server: ServerHealth,
    pub mods: ModsHealth,
    pub integrity: IntegrityHealth,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerHealth {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub version: Option<String>,
    pub version_matches: Option<bool>,
    pub address: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModsHealth {
    pub installed_count: usize,
    pub updates_available: usize,
    pub incompatible_mods: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrityHealth {
    pub tracked_files: usize,
    pub missing_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub untracked_dirs: Vec<UntrackedDir>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UntrackedDir {
    pub path: String,
    pub file_count: usize,
}

impl HealthReport {
    /// Exit code per spec:
    /// - 0: all checks pass
    /// - 1: server is down or unreachable
    /// - 2: mod issues (incompatible mods, missing files, modified files)
    pub fn exit_code(&self) -> i32 {
        if !self.server.reachable {
            return 1;
        }
        if !self.mods.incompatible_mods.is_empty()
            || !self.integrity.missing_files.is_empty()
            || !self.integrity.modified_files.is_empty()
        {
            return 2;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_server() -> ServerHealth {
        ServerHealth {
            reachable: true,
            latency_ms: Some(12),
            version: Some("4.0.13".to_string()),
            version_matches: Some(true),
            address: "https://127.0.0.1:6969".to_string(),
            error: None,
        }
    }

    fn good_mods() -> ModsHealth {
        ModsHealth {
            installed_count: 5,
            updates_available: 0,
            incompatible_mods: vec![],
        }
    }

    fn good_integrity() -> IntegrityHealth {
        IntegrityHealth {
            tracked_files: 100,
            missing_files: vec![],
            modified_files: vec![],
            untracked_dirs: vec![],
        }
    }

    #[test]
    fn exit_code_all_good() {
        let report = HealthReport {
            server: good_server(),
            mods: good_mods(),
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn exit_code_server_down() {
        let report = HealthReport {
            server: ServerHealth {
                reachable: false,
                latency_ms: None,
                version: None,
                version_matches: None,
                address: "https://127.0.0.1:6969".to_string(),
                error: Some("connection refused".to_string()),
            },
            mods: good_mods(),
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn exit_code_incompatible_mods() {
        let report = HealthReport {
            server: good_server(),
            mods: ModsHealth {
                installed_count: 5,
                updates_available: 0,
                incompatible_mods: vec!["OldMod".to_string()],
            },
            integrity: good_integrity(),
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_missing_files() {
        let report = HealthReport {
            server: good_server(),
            mods: good_mods(),
            integrity: IntegrityHealth {
                tracked_files: 50,
                missing_files: vec!["user/mods/Gone/file.dll".to_string()],
                modified_files: vec![],
                untracked_dirs: vec![],
            },
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_modified_files() {
        let report = HealthReport {
            server: good_server(),
            mods: good_mods(),
            integrity: IntegrityHealth {
                tracked_files: 50,
                missing_files: vec![],
                modified_files: vec!["user/mods/X/x.dll".to_string()],
                untracked_dirs: vec![],
            },
        };
        assert_eq!(report.exit_code(), 2);
    }

    #[test]
    fn exit_code_server_down_trumps_mod_issues() {
        let report = HealthReport {
            server: ServerHealth {
                reachable: false,
                latency_ms: None,
                version: None,
                version_matches: None,
                address: "https://127.0.0.1:6969".to_string(),
                error: Some("timeout".to_string()),
            },
            mods: ModsHealth {
                installed_count: 5,
                updates_available: 0,
                incompatible_mods: vec!["X".to_string()],
            },
            integrity: IntegrityHealth {
                tracked_files: 50,
                missing_files: vec!["a.dll".to_string()],
                modified_files: vec![],
                untracked_dirs: vec![],
            },
        };
        assert_eq!(report.exit_code(), 1, "server down (1) should take precedence over mod issues (2)");
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test health::tests`
Expected: PASS

- [ ] **Step 3: Implement `run_checks` in `src/health.rs`**

```rust
use crate::cli::common::CliContext;
use crate::server_detect::resolve_server_addr;
use crate::spt::mods::{compute_file_hash, scan_mod_directories};
use crate::spt::server::SptClient;

use anyhow::Result;

pub async fn run_checks(ctx: &CliContext) -> Result<HealthReport> {
    let (host, port) = resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    let server = check_server(&spt_client, &ctx.spt_info.spt_version, &address).await;

    let installed_mods = ctx.db.list_mods()?;
    let mods = check_mods_compat(&installed_mods, ctx).await;

    let integrity = check_integrity(ctx)?;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}

async fn check_server(
    spt_client: &SptClient,
    expected_version: &str,
    address: &str,
) -> ServerHealth {
    let ping = spt_client.ping().await;

    let (reachable, latency_ms, error) = match &ping {
        Ok(p) if p.ok => (true, Some(p.latency_ms), None),
        Ok(_) => (false, None, Some("server returned error".to_string())),
        Err(e) => (false, None, Some(format!("{e:#}"))),
    };

    if !reachable {
        return ServerHealth {
            reachable,
            latency_ms,
            version: None,
            version_matches: None,
            address: address.to_string(),
            error,
        };
    }

    let version = spt_client.server_version().await.ok();
    let version_matches = version.as_deref().map(|v| v == expected_version);

    ServerHealth {
        reachable,
        latency_ms,
        version,
        version_matches,
        address: address.to_string(),
        error: None,
    }
}

async fn check_mods_compat(
    installed_mods: &[crate::db::mods::InstalledMod],
    ctx: &CliContext,
) -> ModsHealth {
    let mut updates_available = 0;
    let mut incompatible_mods = Vec::new();

    if !installed_mods.is_empty() {
        let check_list: Vec<(i64, String)> = installed_mods
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();

        if let Ok(results) = ctx
            .forge
            .check_updates(&check_list, &ctx.spt_info.spt_version)
            .await
        {
            updates_available = results.iter().filter(|r| r.status == "updated").count();

            for r in &results {
                if r.status == "incompatible" {
                    let name = installed_mods
                        .iter()
                        .find(|m| m.forge_mod_id == r.mod_id)
                        .map(|m| m.name.as_str())
                        .unwrap_or("unknown");
                    incompatible_mods.push(name.to_string());
                }
            }
        }
    }

    ModsHealth {
        installed_count: installed_mods.len(),
        updates_available,
        incompatible_mods,
    }
}

fn check_integrity(ctx: &CliContext) -> Result<IntegrityHealth> {
    let tracked_files = ctx.db.get_all_tracked_files()?;
    let mut missing_files = Vec::new();
    let mut modified_files = Vec::new();

    for file in &tracked_files {
        let full_path = ctx.spt_dir.join(&file.file_path);
        if !full_path.exists() {
            missing_files.push(file.file_path.clone());
            continue;
        }

        if let Some(ref expected_hash) = file.file_hash {
            if let Ok(actual_hash) = compute_file_hash(&full_path) {
                if actual_hash != *expected_hash {
                    modified_files.push(file.file_path.clone());
                }
            }
        }
    }

    let all_disk_files = scan_mod_directories(&ctx.spt_dir)?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let untracked: Vec<&str> = all_disk_files
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let mut dir_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for path in &untracked {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 3 {
            let dir = format!("{}/{}/{}", parts[0], parts[1], parts[2]);
            *dir_counts.entry(dir).or_default() += 1;
        }
    }

    let untracked_dirs: Vec<UntrackedDir> = dir_counts
        .into_iter()
        .map(|(path, file_count)| UntrackedDir { path, file_count })
        .collect();

    Ok(IntegrityHealth {
        tracked_files: tracked_files.len(),
        missing_files,
        modified_files,
        untracked_dirs,
    })
}
```

- [ ] **Step 4: Write tests for `check_integrity`**

Add to `src/health.rs` tests:

```rust
    #[test]
    fn check_integrity_detects_missing_file() {
        use crate::cli::common::CliContext;
        use crate::config::Config;
        use crate::db::Database;
        use crate::forge::client::ForgeClient;
        use crate::spt::detect::SptInfo;

        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();
        db.insert_file(mod_id, "user/mods/TestMod/test.dll", Some("abc123"), Some(100))
            .unwrap();

        let ctx = CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            config_path: tmp.path().join("quartermaster.toml"),
            db,
            forge: ForgeClient::new(None).unwrap(),
        };

        let result = check_integrity(&ctx).unwrap();
        assert_eq!(result.tracked_files, 1);
        assert_eq!(result.missing_files, vec!["user/mods/TestMod/test.dll"]);
        assert!(result.modified_files.is_empty());
    }

    #[test]
    fn check_integrity_detects_modified_file() {
        use crate::cli::common::CliContext;
        use crate::config::Config;
        use crate::db::Database;
        use crate::forge::client::ForgeClient;
        use crate::spt::detect::SptInfo;
        use crate::spt::mods::compute_file_hash;

        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("user/mods/TestMod")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let file_path = spt_dir.join("user/mods/TestMod/test.dll");
        std::fs::write(&file_path, b"original content").unwrap();
        let original_hash = compute_file_hash(&file_path).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "user/mods/TestMod/test.dll",
            Some(&original_hash),
            Some(16),
        )
        .unwrap();

        // Tamper with the file after recording
        std::fs::write(&file_path, b"tampered content").unwrap();

        let ctx = CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            config_path: tmp.path().join("quartermaster.toml"),
            db,
            forge: ForgeClient::new(None).unwrap(),
        };

        let result = check_integrity(&ctx).unwrap();
        assert!(result.missing_files.is_empty());
        assert_eq!(result.modified_files, vec!["user/mods/TestMod/test.dll"]);
    }

    #[test]
    fn check_integrity_detects_untracked_files() {
        use crate::cli::common::CliContext;
        use crate::config::Config;
        use crate::db::Database;
        use crate::forge::client::ForgeClient;
        use crate::spt::detect::SptInfo;

        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("user/mods/UnknownMod")).unwrap();
        std::fs::write(spt_dir.join("user/mods/UnknownMod/mod.dll"), b"x").unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();

        let ctx = CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            config_path: tmp.path().join("quartermaster.toml"),
            db,
            forge: ForgeClient::new(None).unwrap(),
        };

        let result = check_integrity(&ctx).unwrap();
        assert_eq!(result.tracked_files, 0);
        assert_eq!(result.untracked_dirs.len(), 1);
        assert_eq!(result.untracked_dirs[0].path, "user/mods/UnknownMod");
        assert_eq!(result.untracked_dirs[0].file_count, 1);
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test health::tests`
Expected: all PASS

- [ ] **Step 6: Write `src/cli/status.rs` — CLI status command**

```rust
use anyhow::Result;

use crate::health;

use super::common::CliContext;

pub async fn run(json: bool, ctx: &CliContext) -> Result<()> {
    let report = health::run_checks(ctx).await?;
    let exit_code = report.exit_code();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        print_report(&report, &ctx.spt_info);
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

fn print_report(report: &health::HealthReport, spt_info: &crate::spt::detect::SptInfo) {
    println!("SPT Server");
    if report.server.reachable {
        println!(
            "  Status:     running (responded in {}ms)",
            report.server.latency_ms.unwrap_or(0)
        );
        if let Some(ref ver) = report.server.version {
            let match_status = match report.server.version_matches {
                Some(true) => " (matches core.json)",
                Some(false) => " (MISMATCH with core.json!)",
                None => "",
            };
            println!("  Version:    {}{}", ver, match_status);
        }
        println!("  EFT Build:  {}", spt_info.tarkov_version);
        println!("  Address:    {}", report.server.address);
    } else {
        let reason = report.server.error.as_deref().unwrap_or("unreachable");
        println!("  Status:     DOWN ({})", reason);
        println!("  Address:    {}", report.server.address);
    }

    println!();
    println!("Mods ({} installed)", report.mods.installed_count);

    if !report.mods.incompatible_mods.is_empty() {
        for name in &report.mods.incompatible_mods {
            println!(
                "  WARNING: {} is incompatible with SPT {}",
                name, spt_info.spt_version
            );
        }
    }

    if report.mods.updates_available > 0 {
        println!(
            "  {} update(s) available (run `quma check` for details)",
            report.mods.updates_available
        );
    }

    if report.mods.incompatible_mods.is_empty() && report.mods.updates_available == 0 {
        println!("  All mods compatible and up to date.");
    }

    println!();
    println!("Integrity ({} tracked files)", report.integrity.tracked_files);

    if report.integrity.missing_files.is_empty()
        && report.integrity.modified_files.is_empty()
        && report.integrity.untracked_dirs.is_empty()
    {
        println!("  All mod files present on disk, hashes match.");
    } else {
        if !report.integrity.missing_files.is_empty() {
            println!(
                "  {} file(s) MISSING from disk:",
                report.integrity.missing_files.len()
            );
            for f in &report.integrity.missing_files {
                println!("    - {f}");
            }
        }
        if !report.integrity.modified_files.is_empty() {
            println!(
                "  {} file(s) MODIFIED (hash mismatch):",
                report.integrity.modified_files.len()
            );
            for f in &report.integrity.modified_files {
                println!("    - {f}");
            }
        }
        for dir in &report.integrity.untracked_dirs {
            println!("  {} untracked file(s) in {}", dir.file_count, dir.path);
        }
    }
}
```

- [ ] **Step 7: Update `src/cli/mod.rs` to include `status`**

Add `pub mod status;` to the module declarations.

- [ ] **Step 8: Update `src/main.rs` — add `mod health;` and wire `Command::Status`**

Add `mod health;` after the existing module declarations.

Replace `Command::Status { .. } => todo!("status")` with:
```rust
Command::Status { json } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::status::run(*json, &ctx).await
}
```

- [ ] **Step 9: Run all tests and verify compilation**

Run: `cargo test`
Expected: all pass

Run: `cargo clippy -- -D warnings`
Expected: no warnings

- [ ] **Step 10: Commit**

```bash
git add src/health.rs src/cli/status.rs src/cli/mod.rs src/main.rs
git commit -m "feat: health checks and status command with integrity verification and SPT compat check"
```

---

## Task 15: Server Lifecycle Commands

**Files:**
- Create: `src/cli/server.rs`
- Modify: `src/main.rs` (wire `Command::Server` dispatch)
- Modify: `src/cli/mod.rs` (add `pub mod server;`)

**Interfaces:**
- Consumes: `PodmanClient` from `src/podman.rs`
- Consumes: `SptClient` from `src/spt/server.rs`
- Consumes: `is_server_running`, `resolve_server_addr` from `src/server_detect.rs`
- Consumes: `CliContext` from `src/cli/common.rs`
- Consumes: `cli::apply::drain_all` for queue draining (no confirmation prompt)
- Consumes: `cli::status::run` for `server status` alias
- Produces: `cli::server::run(action: &ServerAction, ctx: &CliContext) -> Result<()>`

- [ ] **Step 1: Write `src/cli/server.rs`**

```rust
use anyhow::{bail, Result};

use crate::podman::PodmanClient;
use crate::spt::server::SptClient;

use super::common::CliContext;
use super::ServerAction;

pub async fn run(action: &ServerAction, ctx: &CliContext) -> Result<()> {
    match action {
        ServerAction::Start { timeout } => start(ctx, *timeout).await,
        ServerAction::Stop => stop(ctx).await,
        ServerAction::Restart { drain, skip_queue } => {
            restart(ctx, *drain, *skip_queue).await
        }
        ServerAction::Logs { follow } => logs(ctx, *follow).await,
        ServerAction::Status { json } => crate::cli::status::run(*json, ctx).await,
    }
}

async fn start(ctx: &CliContext, timeout_secs: u64) -> Result<()> {
    let podman = require_container(ctx)?;

    if ctx.config.auto_drain_on_lifecycle {
        drain_if_pending(ctx).await?;
    }

    println!("Starting SPT server container...");
    podman.start().await?;

    wait_for_ping(ctx, timeout_secs).await
}

async fn stop(ctx: &CliContext) -> Result<()> {
    let podman = require_container(ctx)?;

    if ctx.config.auto_drain_on_lifecycle {
        drain_if_pending(ctx).await?;
    }

    println!("Stopping SPT server container...");
    podman.stop().await?;
    println!("Server stopped.");
    Ok(())
}

async fn restart(ctx: &CliContext, force_drain: bool, skip_queue: bool) -> Result<()> {
    let podman = require_container(ctx)?;

    println!("Stopping SPT server container...");
    podman.stop().await?;
    println!("Server stopped.");

    let should_drain = if skip_queue {
        false
    } else if force_drain {
        true
    } else {
        ctx.config.auto_drain_on_lifecycle
    };

    if should_drain {
        drain_if_pending(ctx).await?;
    }

    println!("Starting SPT server container...");
    podman.start().await?;

    wait_for_ping(ctx, 60).await
}

async fn logs(ctx: &CliContext, follow: bool) -> Result<()> {
    let podman = require_container(ctx)?;
    podman.logs(follow, 100).await
}

async fn wait_for_ping(ctx: &CliContext, timeout_secs: u64) -> Result<()> {
    let (host, port) = crate::server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&host, port)?;

    println!("Waiting for server to respond (timeout: {timeout_secs}s)...");
    let start_time = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        if start_time.elapsed() > timeout {
            bail!(
                "Server did not respond within {timeout_secs}s — check `quma server logs` for errors"
            );
        }

        let ping = spt_client.ping().await?;
        if ping.ok {
            println!("Server is ready (responded in {}ms).", ping.latency_ms);
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

fn require_container(ctx: &CliContext) -> Result<PodmanClient> {
    match &ctx.config.server_container {
        Some(name) => Ok(PodmanClient::new(name)),
        None => bail!(
            "no server_container configured.\n\
             Set it with: quma config set server_container <name>\n\
             Or run `quma setup` to auto-detect."
        ),
    }
}

async fn drain_if_pending(ctx: &CliContext) -> Result<()> {
    let pending = ctx.db.list_pending_ops()?;
    if pending.is_empty() {
        return Ok(());
    }
    println!("\nDraining {} pending operation(s)...", pending.len());
    crate::cli::apply::drain_all(ctx).await?;
    Ok(())
}
```

- [ ] **Step 2: Update `src/cli/mod.rs` to include `server`**

Add `pub mod server;` to the module declarations.

- [ ] **Step 3: Wire `Command::Server` dispatch in `src/main.rs`**

Replace `Command::Server { .. } => todo!("server")` with:
```rust
Command::Server { action } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::server::run(action, &ctx).await
}
```

- [ ] **Step 4: Remove stale `#[allow(dead_code)]` annotations**

- Remove `#[allow(dead_code)]` from `config` and `config_path` fields in `src/cli/common.rs` `CliContext` struct — both are now used by queue and server commands.
- Remove `#![allow(dead_code)]` from `src/error.rs` if `ServerRunning` is now used — check if any code path uses it; if not, keep the allow but remove the `ServerRunning` variant (it's currently unused; the plan uses `bail!()` with inline messages instead). Actually: audit whether `ServerRunning` is used. The plan's queue check uses `bail!()` with custom messages rather than `QumaError::ServerRunning`. Decision: keep the variant for potential future use but leave the `#![allow(dead_code)]` on `error.rs` since some variants are still unused until Phase 4+.
- Keep `#![allow(dead_code)]` on `src/db/mod.rs` and `src/config.rs` — user CRUD and invite methods are not used until Phase 4.

- [ ] **Step 5: Run all tests and clippy**

Run: `cargo test`
Expected: all pass

Run: `cargo clippy -- -D warnings`
Expected: no warnings (fix any findings)

- [ ] **Step 6: Commit**

```bash
git add src/cli/server.rs src/cli/mod.rs src/main.rs src/cli/common.rs src/error.rs
git commit -m "feat: server lifecycle commands — start, stop, restart, logs with queue drain"
```
