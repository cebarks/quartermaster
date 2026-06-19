# Container Creation Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow quartermaster to create properly configured SPT server containers via `quma setup` and `quma server create`, eliminating error-prone manual `podman create` invocations.

**Architecture:** Add `pull_image()` and `create_spt_container()` associated functions to `PodmanClient`, wire them into the existing `configure_container()` setup flow and a new `ServerAction::Create` CLI subcommand.

**Tech Stack:** Rust, tokio::process::Command (podman CLI), clap (CLI args)

## Global Constraints

- Image: `ghcr.io/zhliau/fika-spt-server-docker:latest`
- Always pull before create (no local cache check)
- No Fika-specific env vars (`FIKA_MODE` etc.) — managed through web UI
- No web UI changes (deferred to setup wizard)
- Container env vars: `TAKE_OWNERSHIP=true`, `CHANGE_PERMISSIONS=true`, `LISTEN_ALL_NETWORKS=true`
- Container flags: `--user root`

---

### Task 1: Add `pull_image()` and `create_spt_container()` to PodmanClient

**Files:**
- Modify: `src/podman.rs`

**Interfaces:**
- Consumes: nothing new
- Produces:
  - `PodmanClient::pull_image(image: &str) -> Result<()>` (associated fn)
  - `PodmanClient::create_spt_container(name: &str, spt_dir: &Path, port: u16) -> Result<()>` (associated fn)
  - `pub const SPT_SERVER_IMAGE: &str`
  - `pub const DEFAULT_CONTAINER_NAME: &str`
  - `pub const DEFAULT_SPT_PORT: u16`

- [ ] **Step 1: Write failing tests for `pull_image` argument construction**

Add to the `#[cfg(test)] mod tests` block in `src/podman.rs`:

```rust
#[test]
fn pull_image_constructs_correct_command() {
    // Verify the constant is correct
    assert_eq!(SPT_SERVER_IMAGE, "ghcr.io/zhliau/fika-spt-server-docker:latest");
}
```

- [ ] **Step 2: Write failing tests for `create_spt_container` argument construction**

Add to the same test block:

```rust
#[test]
fn default_container_constants() {
    assert_eq!(DEFAULT_CONTAINER_NAME, "spt-server");
    assert_eq!(DEFAULT_SPT_PORT, 6969);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p quartermaster podman::tests -- --nocapture`
Expected: compilation failure — constants and functions don't exist yet.

- [ ] **Step 4: Add constants and implement `pull_image`**

Add the following near the top of `src/podman.rs`, after the existing imports:

```rust
pub const SPT_SERVER_IMAGE: &str = "ghcr.io/zhliau/fika-spt-server-docker:latest";
pub const DEFAULT_CONTAINER_NAME: &str = "spt-server";
pub const DEFAULT_SPT_PORT: u16 = 6969;
```

Add this associated function inside the `impl PodmanClient` block (before the `pub fn new` method):

```rust
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
```

- [ ] **Step 5: Implement `create_spt_container`**

Add this associated function inside the `impl PodmanClient` block, after `pull_image`:

```rust
pub async fn create_spt_container(name: &str, spt_dir: &Path, port: u16) -> Result<()> {
    let mount = format!("{}:/opt/server", spt_dir.display());
    let port_map = format!("{port}:6969");

    tracing::info!(name, spt_dir = %spt_dir.display(), port, "creating SPT server container");
    let output = tokio::process::Command::new("podman")
        .args([
            "create",
            "--name", name,
            "-p", &port_map,
            "-v", &mount,
            "--user", "root",
            "-e", "TAKE_OWNERSHIP=true",
            "-e", "CHANGE_PERMISSIONS=true",
            "-e", "LISTEN_ALL_NETWORKS=true",
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
```

Add `use std::path::Path;` to the existing imports at the top of the file (it's already imported, verify).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p quartermaster podman::tests -- --nocapture`
Expected: all tests pass, including existing ones.

- [ ] **Step 7: Commit**

```bash
git add src/podman.rs
git commit -m "feat: add pull_image and create_spt_container to PodmanClient"
```

---

### Task 2: Add container creation to `quma setup` flow

**Files:**
- Modify: `src/cli/setup.rs`

**Interfaces:**
- Consumes:
  - `PodmanClient::pull_image(image: &str) -> Result<()>`
  - `PodmanClient::create_spt_container(name: &str, spt_dir: &Path, port: u16) -> Result<()>`
  - `podman::SPT_SERVER_IMAGE`, `podman::DEFAULT_CONTAINER_NAME`, `podman::DEFAULT_SPT_PORT`
- Produces: modified `configure_container()` with container creation branch

- [ ] **Step 1: Modify `configure_container` to add creation branch**

In `src/cli/setup.rs`, replace the final `if !non_interactive { ... }` block in `configure_container()` (lines 167-180) with a creation flow. The new end of the function (replacing from line 163 `} else {` through line 182 closing `}`) becomes:

```rust
    } else {
        println!("No Podman containers detected.");
    }

    // Offer to create a container
    if non_interactive || confirm("Create an SPT server container?")? {
        let name = if non_interactive {
            crate::podman::DEFAULT_CONTAINER_NAME.to_string()
        } else {
            print!(
                "Container name [{}]: ",
                crate::podman::DEFAULT_CONTAINER_NAME
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if input.is_empty() {
                crate::podman::DEFAULT_CONTAINER_NAME.to_string()
            } else {
                input.to_string()
            }
        };

        let port = if non_interactive {
            crate::podman::DEFAULT_SPT_PORT
        } else {
            print!("Host port [{}]: ", crate::podman::DEFAULT_SPT_PORT);
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if input.is_empty() {
                crate::podman::DEFAULT_SPT_PORT
            } else {
                input
                    .parse()
                    .context("invalid port number")?
            }
        };

        println!("Pulling {}...", crate::podman::SPT_SERVER_IMAGE);
        PodmanClient::pull_image(crate::podman::SPT_SERVER_IMAGE).await?;

        println!("Creating container '{name}'...");
        PodmanClient::create_spt_container(&name, spt_dir, port).await?;

        println!("Container '{name}' created successfully.");
        config.server_container = Some(name);
        return Ok(());
    }

    // Manual entry fallback
    if !non_interactive {
        print!("Enter container name (or press Enter to skip): ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let name = input.trim();
        if !name.is_empty() {
            config.server_container = Some(name.to_string());
        } else {
            println!(
                "Skipping container setup. Set it later with: quma config set server_container <name>"
            );
        }
    }

    Ok(())
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build 2>&1 | tail -5`
Expected: compiles with no errors.

- [ ] **Step 3: Run existing setup tests to verify no regressions**

Run: `cargo test -p quartermaster setup::tests -- --nocapture`
Expected: all existing tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/cli/setup.rs
git commit -m "feat: add container creation to quma setup flow"
```

---

### Task 3: Add `quma server create` subcommand

**Files:**
- Modify: `src/cli/mod.rs` (add `Create` variant to `ServerAction`)
- Modify: `src/cli/server.rs` (add `create` handler)

**Interfaces:**
- Consumes:
  - `PodmanClient::pull_image(image: &str) -> Result<()>`
  - `PodmanClient::create_spt_container(name: &str, spt_dir: &Path, port: u16) -> Result<()>`
  - `podman::SPT_SERVER_IMAGE`, `podman::DEFAULT_CONTAINER_NAME`, `podman::DEFAULT_SPT_PORT`
  - `Config::save(&self, path: &Path) -> Result<()>`
  - `Config::resolve_path(cli_config: Option<&Path>, spt_dir: Option<&Path>) -> PathBuf`
  - `CliContext` (from `cli::common`)
- Produces: `quma server create [--name <name>] [--port <port>]` subcommand

- [ ] **Step 1: Add `Create` variant to `ServerAction` enum**

In `src/cli/mod.rs`, add the `Create` variant to the `ServerAction` enum (after the existing `Status` variant):

```rust
/// Create a new SPT server container
Create {
    /// Container name
    #[arg(long, default_value = "spt-server")]
    name: String,
    /// Host port to map to container port 6969
    #[arg(long, default_value = "6969")]
    port: u16,
},
```

- [ ] **Step 2: Add match arm in `server::run`**

In `src/cli/server.rs`, add the import for `Cli` at the top and add the match arm in the `run` function:

Add to imports:
```rust
use super::Cli;
```

Add match arm in `run()`:
```rust
ServerAction::Create { name, port } => create(ctx, name, *port).await,
```

- [ ] **Step 3: Implement the `create` function**

Add this function to `src/cli/server.rs` (after the existing `logs` function):

```rust
async fn create(ctx: &CliContext, name: &str, port: u16) -> Result<()> {
    println!("Pulling {}...", crate::podman::SPT_SERVER_IMAGE);
    PodmanClient::pull_image(crate::podman::SPT_SERVER_IMAGE).await?;

    println!("Creating container '{name}'...");
    PodmanClient::create_spt_container(name, &ctx.spt_dir, port).await?;
    println!("Container '{name}' created successfully.");

    if ctx.config.server_container.is_none() {
        let mut config = ctx.config.clone();
        config.server_container = Some(name.to_string());
        let config_path = Config::resolve_path(None, Some(&ctx.spt_dir));
        config.save(&config_path)?;
        println!("Updated config: server_container = {name}");
    }

    Ok(())
}
```

Add to the imports at the top of `src/cli/server.rs`:
```rust
use crate::config::Config;
```

- [ ] **Step 4: Update the `require_container` error message**

Update the error message in the existing `require_container` function to also mention `quma server create`:

```rust
fn require_container(ctx: &CliContext) -> Result<PodmanClient> {
    match &ctx.config.server_container {
        Some(name) => Ok(PodmanClient::new(name)),
        None => bail!(
            "no server_container configured.\n\
             Run `quma server create` to create one, or\n\
             set it with: quma config set server_container <name>"
        ),
    }
}
```

- [ ] **Step 5: Verify compilation and test**

Run: `cargo build 2>&1 | tail -5`
Expected: compiles with no errors.

Run: `cargo test -p quartermaster -- --nocapture 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 6: Verify CLI help output**

Run: `cargo run -- server --help`
Expected: shows `create` as a subcommand alongside start/stop/restart/logs/status.

Run: `cargo run -- server create --help`
Expected: shows `--name` (default "spt-server") and `--port` (default 6969) options.

- [ ] **Step 7: Commit**

```bash
git add src/cli/mod.rs src/cli/server.rs
git commit -m "feat: add quma server create subcommand"
```
