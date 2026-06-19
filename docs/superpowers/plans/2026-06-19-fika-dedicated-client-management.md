# Fika Dedicated Client Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add declarative management of Fika headless dedicated clients to Quartermaster, including a bollard migration to replace CLI-based Podman interaction.

**Architecture:** Replace `PodmanClient` (CLI shelling to `podman`) with `ContainerManager` (bollard, native Podman API via Docker-compat socket). Add a `[clients]` config section for declarative client count. A convergence engine reconciles desired vs actual state. A supervisor background task monitors health and auto-restarts. New `quma client` CLI commands and `/clients` web UI pages expose the feature.

**Tech Stack:** Rust, bollard (Podman/Docker API), tokio (async runtime), actix-web (web server), Askama (templates), HTMX (frontend interactivity), SSE (real-time updates)

## Global Constraints

- Linux only (v1)
- SELinux volume labels required: `:z` (shared) for read-only base mounts, `:Z` (private) for per-client overlay mounts
- `podman.socket` must be enabled for bollard to connect
- All quma-created containers must have label `managed-by=quma`
- Fika detection gate: entire feature disabled if `<spt_dir>/SPT/user/mods/fika-server/` does not exist
- `json_comments` (already a dep) for reading JSONC; text-level string replacement for writing (preserve user comments)
- Container naming: `fika-headless-1`, `fika-headless-2`, etc. (deterministic, sequential)
- UDP ports: `base_udp_port + n - 1` (validated: must not exceed 65535)
- Headless client image: `ghcr.io/zhliau/fika-headless-docker:latest` (configurable)

**Spec:** `docs/superpowers/specs/2026-06-19-fika-dedicated-client-management-design.md`

---

### Task 1: Add bollard dependency and implement ContainerManager

**Files:**
- Modify: `Cargo.toml` — add `bollard` and `tokio-util` dependencies
- Create: `src/container.rs` — `ContainerManager` struct wrapping bollard
- Modify: `src/main.rs:9` — add `mod container;`
- Test: `src/container.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing (foundation task)
- Produces:
  - `ContainerManager::new() -> Result<Self>`
  - `ContainerManager::start(&self, container: &str) -> Result<()>`
  - `ContainerManager::stop(&self, container: &str) -> Result<()>`
  - `ContainerManager::restart(&self, container: &str) -> Result<()>`
  - `ContainerManager::is_running(&self, container: &str) -> Result<bool>`
  - `ContainerManager::inspect(&self, container: &str) -> Result<ContainerInspectResponse>`
  - `ContainerManager::stats_stream(&self, container: &str) -> impl Stream`
  - `ContainerManager::log_stream(&self, container: &str, tail: usize, follow: bool) -> impl Stream`
  - `ContainerManager::container_events(&self) -> impl Stream`
  - `ContainerManager::pull_image(&self, image: &str) -> Result<()>`
  - `ContainerManager::create_container(&self, opts: CreateContainerOpts) -> Result<String>`
  - `ContainerManager::remove_container(&self, container: &str) -> Result<()>`
  - `ContainerManager::detect_containers_by_label(&self, key: &str, value: &str) -> Result<Vec<String>>`
  - `CreateContainerOpts`, `VolumeMount`, `SelinuxLabel`, `PortMapping`, `Protocol` types

- [ ] **Step 1: Add dependencies to Cargo.toml**

Add `bollard` and `tokio-util` to `[dependencies]` in `Cargo.toml`:

```toml
# Container management (Podman/Docker API)
bollard = "0.20"
tokio-util = "0.7"
```

- [ ] **Step 2: Write unit tests for ContainerManager types and construction**

Create `src/container.rs` with tests first. Since bollard needs a live Podman socket, unit tests focus on type construction and SELinux label formatting. Integration tests for actual container ops are gated behind an env var.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn selinux_label_display() {
        assert_eq!(SelinuxLabel::Private.as_suffix(), ":Z");
        assert_eq!(SelinuxLabel::Shared.as_suffix(), ":z");
        assert_eq!(SelinuxLabel::None.as_suffix(), "");
    }

    #[test]
    fn volume_mount_to_bind_string() {
        let mount = VolumeMount {
            host_path: PathBuf::from("/opt/fika-client"),
            container_path: "/opt/tarkov".to_string(),
            read_only: true,
            selinux: SelinuxLabel::Shared,
        };
        assert_eq!(
            mount.to_bind_string(),
            "/opt/fika-client:/opt/tarkov:ro,z"
        );
    }

    #[test]
    fn volume_mount_rw_private() {
        let mount = VolumeMount {
            host_path: PathBuf::from("/data/clients/1/BepInEx/config"),
            container_path: "/opt/tarkov/BepInEx/config".to_string(),
            read_only: false,
            selinux: SelinuxLabel::Private,
        };
        assert_eq!(
            mount.to_bind_string(),
            "/data/clients/1/BepInEx/config:/opt/tarkov/BepInEx/config:rw,Z"
        );
    }

    #[test]
    fn create_container_opts_always_includes_managed_label() {
        let opts = CreateContainerOpts {
            name: "test".to_string(),
            image: "test:latest".to_string(),
            env: vec![],
            volumes: vec![],
            ports: vec![],
            labels: vec![("custom".to_string(), "value".to_string())],
        };
        let labels = opts.all_labels();
        assert!(labels.iter().any(|(k, v)| k == "managed-by" && v == "quma"));
    }

    #[test]
    fn port_mapping_to_key() {
        let pm = PortMapping {
            host_port: 25565,
            container_port: 25565,
            protocol: Protocol::Udp,
        };
        assert_eq!(pm.container_key(), "25565/udp");

        let pm_tcp = PortMapping {
            host_port: 8080,
            container_port: 80,
            protocol: Protocol::Tcp,
        };
        assert_eq!(pm_tcp.container_key(), "80/tcp");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p quartermaster container::tests -- --nocapture`
Expected: FAIL — module and types don't exist yet

- [ ] **Step 4: Implement ContainerManager struct, types, and all methods**

Implement the full `src/container.rs`. Key implementation details:

- `new()` calls `Docker::connect_with_podman_defaults()` (bollard auto-discovers rootless/system Podman socket)
- All bollard option types use the builder pattern: `StopContainerOptionsBuilder::default().t(10).build()`
- Model types (`ContainerCreateBody`, `ContainerInspectResponse`, `PortBinding`, `HostConfig`, `EventMessage`) are in `bollard::models`
- Query parameter types (`StartContainerOptions`, builders) are in `bollard::query_parameters`
- `VolumeMount::to_bind_string()` produces Docker bind mount syntax with SELinux suffix
- `PortMapping::container_key()` produces `"port/proto"` format for bollard's port maps
- Constants `SPT_SERVER_IMAGE`, `DEFAULT_CONTAINER_NAME`, `DEFAULT_SPT_PORT` moved here from `podman.rs`

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, EventMessage, HostConfig, PortBinding,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
    EventsOptionsBuilder, InspectContainerOptionsBuilder,
    ListContainersOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, StartContainerOptions,
    StatsOptionsBuilder, StopContainerOptionsBuilder,
};
use futures_util::Stream;

pub const SPT_SERVER_IMAGE: &str = "ghcr.io/zhliau/fika-spt-server-docker:latest";
pub const DEFAULT_CONTAINER_NAME: &str = "spt-server";
pub const DEFAULT_SPT_PORT: u16 = 6969;

pub struct ContainerManager {
    docker: Arc<Docker>,
}

#[derive(Debug, Clone)]
pub enum SelinuxLabel {
    Private,
    Shared,
    None,
}

impl SelinuxLabel {
    pub fn as_suffix(&self) -> &str {
        match self {
            SelinuxLabel::Private => ":Z",
            SelinuxLabel::Shared => ":z",
            SelinuxLabel::None => "",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone)]
pub struct VolumeMount {
    pub host_path: PathBuf,
    pub container_path: String,
    pub read_only: bool,
    pub selinux: SelinuxLabel,
}

impl VolumeMount {
    pub fn to_bind_string(&self) -> String {
        let rw = if self.read_only { "ro" } else { "rw" };
        let sel = self.selinux.as_suffix();
        if sel.is_empty() {
            format!("{}:{}:{}", self.host_path.display(), self.container_path, rw)
        } else {
            format!("{}:{}:{},{}", self.host_path.display(), self.container_path, rw, &sel[1..])
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: Protocol,
}

impl PortMapping {
    pub fn container_key(&self) -> String {
        let proto = match self.protocol { Protocol::Tcp => "tcp", Protocol::Udp => "udp" };
        format!("{}/{proto}", self.container_port)
    }
}

#[derive(Debug, Clone)]
pub struct CreateContainerOpts {
    pub name: String,
    pub image: String,
    pub env: Vec<(String, String)>,
    pub volumes: Vec<VolumeMount>,
    pub ports: Vec<PortMapping>,
    pub labels: Vec<(String, String)>,
    pub user: Option<String>,
}

impl CreateContainerOpts {
    pub fn all_labels(&self) -> Vec<(String, String)> {
        let mut labels = self.labels.clone();
        if !labels.iter().any(|(k, _)| k == "managed-by") {
            labels.push(("managed-by".to_string(), "quma".to_string()));
        }
        labels
    }
}

impl ContainerManager {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_podman_defaults()
            .context(
                "failed to connect to Podman socket. Ensure podman.socket is enabled:\n  \
                 systemctl --user enable --now podman.socket"
            )?;
        Ok(Self { docker: Arc::new(docker) })
    }

    pub fn docker(&self) -> &Arc<Docker> {
        &self.docker
    }

    pub async fn start(&self, container: &str) -> Result<()> {
        tracing::debug!(container, "starting container");
        self.docker
            .start_container(container, None::<StartContainerOptions>)
            .await
            .with_context(|| format!("failed to start container '{container}'"))
    }

    pub async fn stop(&self, container: &str) -> Result<()> {
        tracing::debug!(container, "stopping container");
        self.docker
            .stop_container(
                container,
                Some(StopContainerOptionsBuilder::default().t(10).build()),
            )
            .await
            .with_context(|| format!("failed to stop container '{container}'"))
    }

    pub async fn restart(&self, container: &str) -> Result<()> {
        self.stop(container).await?;
        self.start(container).await
    }

    pub async fn is_running(&self, container: &str) -> Result<bool> {
        let info = self.docker
            .inspect_container(
                container,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .with_context(|| format!("failed to inspect container '{container}'"))?;
        Ok(info.state
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .is_some_and(|s| s.as_ref() == "running"))
    }

    pub async fn inspect(&self, container: &str) -> Result<ContainerInspectResponse> {
        self.docker
            .inspect_container(
                container,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .with_context(|| format!("failed to inspect container '{container}'"))
    }

    pub fn stats_stream(
        &self,
        container: &str,
    ) -> impl Stream<Item = Result<bollard::models::Stats, bollard::errors::Error>> {
        self.docker.stats(
            container,
            Some(StatsOptionsBuilder::default().stream(true).build()),
        )
    }

    pub fn log_stream(
        &self,
        container: &str,
        tail: usize,
        follow: bool,
    ) -> impl Stream<Item = Result<bollard::container::LogOutput, bollard::errors::Error>> {
        self.docker.logs(
            container,
            Some(
                LogsOptionsBuilder::default()
                    .stdout(true)
                    .stderr(true)
                    .follow(follow)
                    .tail(&tail.to_string())
                    .timestamps(true)
                    .build(),
            ),
        )
    }

    pub fn container_events(
        &self,
    ) -> impl Stream<Item = Result<EventMessage, bollard::errors::Error>> {
        let mut filters = HashMap::new();
        filters.insert("type", vec!["container"]);
        self.docker.events(Some(
            EventsOptionsBuilder::default().filters(&filters).build(),
        ))
    }

    pub async fn pull_image(&self, image: &str) -> Result<()> {
        tracing::info!(image, "pulling container image");
        use futures_util::TryStreamExt;
        self.docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(image)
                        .build(),
                ),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
            .with_context(|| format!("failed to pull image '{image}'"))?;
        Ok(())
    }

    pub async fn create_container(&self, opts: CreateContainerOpts) -> Result<String> {
        let env: Vec<String> = opts.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let binds: Vec<String> = opts.volumes.iter().map(|v| v.to_bind_string()).collect();
        let labels: HashMap<String, String> = opts.all_labels().into_iter().collect();

        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for pm in &opts.ports {
            port_bindings.insert(
                pm.container_key(),
                Some(vec![PortBinding {
                    host_port: Some(pm.host_port.to_string()),
                    ..Default::default()
                }]),
            );
        }

        let body = ContainerCreateBody {
            image: Some(opts.image.clone()),
            env: Some(env),
            labels: Some(labels),
            user: opts.user.clone(),
            host_config: Some(HostConfig {
                binds: Some(binds),
                port_bindings: Some(port_bindings),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_opts = CreateContainerOptionsBuilder::default()
            .name(&opts.name)
            .build();
        let response = self.docker
            .create_container(Some(create_opts), body)
            .await
            .with_context(|| format!("failed to create container '{}'", opts.name))?;
        tracing::info!(container = %opts.name, id = %response.id, "container created");
        Ok(response.id)
    }

    pub async fn remove_container(&self, container: &str) -> Result<()> {
        tracing::debug!(container, "removing container");
        self.docker
            .remove_container(
                container,
                Some(
                    RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .build(),
                ),
            )
            .await
            .with_context(|| format!("failed to remove container '{container}'"))
    }

    pub async fn detect_containers_by_label(&self, key: &str, value: &str) -> Result<Vec<String>> {
        let mut filters = HashMap::new();
        filters.insert("label", vec![&format!("{key}={value}") as &str]);
        let containers = self.docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .context("failed to list containers")?;
        Ok(containers
            .into_iter()
            .filter_map(|c| {
                c.names?
                    .into_iter()
                    .next()
                    .map(|n| n.trim_start_matches('/').to_string())
            })
            .collect())
    }

    /// Detect SPT containers by checking volume mounts (for setup wizard backward compat)
    pub async fn detect_spt_containers(&self, spt_dir: &std::path::Path) -> Result<Vec<String>> {
        let containers = self.docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await
            .context("failed to list containers")?;

        let spt_dir_str = spt_dir.to_string_lossy();
        Ok(containers
            .into_iter()
            .filter_map(|c| {
                let mounts = c.mounts.as_ref()?;
                let has_spt_mount = mounts.iter().any(|m| {
                    m.source.as_deref().is_some_and(|s| s.contains(spt_dir_str.as_ref()))
                });
                if has_spt_mount {
                    c.names?.into_iter().next().map(|n| n.trim_start_matches('/').to_string())
                } else {
                    None
                }
            })
            .collect())
    }
}
```

- [ ] **Step 5: Register module in main.rs**

Add `mod container;` to `src/main.rs` after `mod config;` (line 2).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p quartermaster container::tests -- --nocapture`
Expected: PASS — all type construction and formatting tests pass

- [ ] **Step 7: Run `cargo check` to verify compilation**

Run: `cargo check`
Expected: compiles with no errors (dead_code warnings are fine — callsites come in Task 2)

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/container.rs src/main.rs
git commit -m "feat: add ContainerManager backed by bollard for native Podman API"
```

---

### Task 2: Migrate existing PodmanClient callsites to ContainerManager

**Files:**
- Modify: `src/cli/server.rs` — replace `PodmanClient` with `ContainerManager`
- Modify: `src/cli/serve.rs` — replace `PodmanClient` with `ContainerManager`
- Modify: `src/cli/setup.rs:10` — remove `use crate::podman::PodmanClient`
- Modify: `src/server_detect.rs` — replace `PodmanClient` with `ContainerManager`
- Modify: `src/cli/common.rs` — add `ContainerManager` to `CliContext`
- Modify: `src/web/state.rs` — add `ContainerManager` to `AppState`
- Modify: `src/web/mod.rs` — create `ContainerManager` and pass to `AppState`
- Modify: `src/web/handlers/server.rs` — replace `PodmanClient` with `ContainerManager`
- Modify: `src/main.rs` — remove `mod podman;`, add `Client` command handling
- Delete: `src/podman.rs`
- Test: existing tests must still pass after migration

**Interfaces:**
- Consumes: `ContainerManager` from Task 1
- Produces: All existing container operations use `ContainerManager` instead of `PodmanClient`. `CliContext` gains an `Option<ContainerManager>` field. `AppState` gains a `ContainerManager` field.

- [ ] **Step 1: Update `CliContext` to optionally hold a `ContainerManager`**

In `src/cli/common.rs`, add an optional `ContainerManager` field. It's optional because `ContainerManager::new()` can fail (no socket), and many CLI commands don't need container ops.

Add the field to `CliContext`:

```rust
pub struct CliContext {
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub config: Config,
    pub db: Database,
    pub forge: ForgeClient,
    pub container_mgr: Option<crate::container::ContainerManager>,
}
```

Add `use crate::container::ContainerManager;` at the top. In `resolve_context()`, attempt to create a `ContainerManager` but don't fail if the socket isn't available:

```rust
let container_mgr = ContainerManager::new().ok();
```

Add it to the `CliContext` construction.

- [ ] **Step 2: Migrate `src/cli/server.rs` from `PodmanClient` to `ContainerManager`**

Replace `require_container()` to return `&ContainerManager` from the context:

```rust
fn require_container(ctx: &CliContext) -> Result<&ContainerManager> {
    match (&ctx.config.server_container, &ctx.container_mgr) {
        (None, _) => bail!(
            "no server_container configured.\n\
             Run `quma server create` to create one, or\n\
             set it with: quma config set server_container <name>"
        ),
        (Some(_), None) => bail!(
            "failed to connect to Podman socket.\n\
             Ensure podman.socket is enabled:\n  \
             systemctl --user enable --now podman.socket"
        ),
        (Some(_), Some(mgr)) => Ok(mgr),
    }
}
```

Update `start`, `stop`, `restart`, `logs` to use the container name from config with `ContainerManager` methods. The `create_container` function uses `ContainerManager::pull_image()` and `ContainerManager::create_container()` — note that the existing `create_spt_container` sets `--user root` and env vars `TAKE_OWNERSHIP=true`, `CHANGE_PERMISSIONS=true`, `LISTEN_ALL_NETWORKS=true`; replicate these via `CreateContainerOpts { user: Some("root".to_string()), env: vec![...], ... }`.

The `logs` function changes from `podman.logs(follow, 100)` (which used `Stdio::inherit()` for direct terminal passthrough) to `ContainerManager::log_stream()` which returns `Stream<Item = Result<LogOutput>>`. The `LogOutput` enum has `StdOut(Bytes)`, `StdErr(Bytes)`, `StdIn(Bytes)`, `Console(Bytes)` variants. Print each variant's bytes as lossy UTF-8 to stdout/stderr respectively:

```rust
async fn logs(ctx: &CliContext, follow: bool) -> Result<()> {
    let mgr = require_container(ctx)?;
    let container = ctx.config.server_container.as_ref().unwrap();
    use futures_util::StreamExt;
    let mut stream = mgr.log_stream(container, 100, follow);
    while let Some(log) = stream.next().await {
        match log? {
            bollard::container::LogOutput::StdOut { message } => {
                print!("{}", String::from_utf8_lossy(&message));
            }
            bollard::container::LogOutput::StdErr { message } => {
                eprint!("{}", String::from_utf8_lossy(&message));
            }
            _ => {}
        }
    }
    Ok(())
}
```

Constants `SPT_SERVER_IMAGE`, `DEFAULT_CONTAINER_NAME`, `DEFAULT_SPT_PORT` are now in `src/container.rs` (moved from `podman.rs`). Update all references: `crate::podman::SPT_SERVER_IMAGE` → `crate::container::SPT_SERVER_IMAGE`, etc.

Remove `use crate::podman::PodmanClient;`.

- [ ] **Step 3: Migrate `src/cli/serve.rs` from `PodmanClient` to `ContainerManager`**

Replace the auto-start block (lines 50-68) to use `ContainerManager`:

```rust
if config.auto_start_server {
    if let Some(ref container) = config.server_container {
        match crate::container::ContainerManager::new() {
            Ok(mgr) => match mgr.is_running(container).await {
                Ok(true) => {
                    tracing::info!(container, "server container already running");
                }
                Ok(false) => {
                    tracing::info!(container, "auto-starting server container");
                    if let Err(e) = mgr.start(container).await {
                        tracing::warn!(container, error = %e, "failed to auto-start server container — web UI will start anyway");
                    }
                }
                Err(e) => {
                    tracing::warn!(container, error = %e, "failed to check container status — skipping auto-start");
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "failed to connect to Podman — skipping auto-start");
            }
        }
    }
}
```

Remove `use crate::podman::PodmanClient;`.

- [ ] **Step 4: Migrate `src/server_detect.rs` to use `ContainerManager`**

Change `is_server_running` to accept an `Option<&ContainerManager>` instead of creating `PodmanClient` inline:

```rust
pub async fn is_server_running(config: &Config, spt_dir: &Path, container_mgr: Option<&ContainerManager>) -> Result<bool> {
    if let Some(ref container) = config.server_container {
        if let Some(mgr) = container_mgr {
            return mgr.is_running(container).await;
        }
        bail!("Podman socket not available — cannot check container status");
    }
    // fallback to HTTP ping...
}
```

Update all callers of `is_server_running` to pass the new parameter:
- `src/queue.rs` — `should_queue()` calls `is_server_running`
- `src/cli/install.rs`, `src/cli/update.rs`, `src/cli/remove.rs` — queue decision calls
- `src/web/handlers/status.rs` — status page checks
- Any other callers found via `rg 'is_server_running' src/`

Remove `use crate::podman::PodmanClient;`.

- [ ] **Step 5: Migrate `src/web/state.rs` and `src/web/mod.rs`**

Add `ContainerManager` to `AppState`:

```rust
pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
    pub update_cache: UpdateCache,
    pub events: broadcast::Sender<ServerEvent>,
    pub log_broadcast: Arc<LogBroadcast>,
    pub container_mgr: Option<Arc<crate::container::ContainerManager>>,
}
```

In `src/web/mod.rs` `start_server()`, create the `ContainerManager` and add to `AppState`:

```rust
let container_mgr = crate::container::ContainerManager::new()
    .map(Arc::new)
    .ok();
```

- [ ] **Step 6: Migrate `src/web/handlers/server.rs` to use `ContainerManager`**

Replace `PodmanClient::new(container)` with `state.container_mgr.as_ref()`. Each handler checks for both `server_container` config and `container_mgr` availability. Remove `use crate::podman::PodmanClient;`.

- [ ] **Step 7: Update `src/cli/setup.rs` to remove `PodmanClient` import**

Remove `use crate::podman::PodmanClient;` (line 10). The setup module uses:
- `PodmanClient::pull_image()` → `ContainerManager::pull_image()`
- `PodmanClient::create_spt_container()` → `ContainerManager::create_container()` with `user: Some("root".to_string())` and SPT-specific env vars
- `PodmanClient::detect_spt_containers()` → `ContainerManager::detect_spt_containers()` (mount-path-based detection preserved for backward compat with existing un-labeled containers)

- [ ] **Step 8: Delete `src/podman.rs` and remove `mod podman;` from `src/main.rs`**

Delete the file. Remove `mod podman;` from `src/main.rs` (line 9).

- [ ] **Step 9: Run all tests and clippy**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: all 175 existing tests pass, no clippy warnings. Some `podman` tests are deleted with the file — that's expected.

- [ ] **Step 10: Commit**

Stage the specific changed files (verify with `git diff --name-only` and `git status` first):

```bash
git add src/cli/server.rs src/cli/serve.rs src/cli/setup.rs src/cli/common.rs src/server_detect.rs src/web/state.rs src/web/mod.rs src/web/handlers/server.rs src/main.rs src/queue.rs
git rm src/podman.rs
git commit -m "refactor: migrate all container ops from PodmanClient to bollard-backed ContainerManager"
```

---

### Task 3: Add ClientsConfig to config system

**Files:**
- Modify: `src/config.rs` — add `ClientsConfig`, `RestartPolicy`, validation, env var overrides, serde defaults
- Test: `src/config.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `ClientsConfig { count, install_dir, restart_policy, max_restart_attempts, restart_backoff_cap, base_udp_port, image, isolated_paths }`
  - `RestartPolicy` enum (`Auto` | `Manual`)
  - `Config.clients: Option<ClientsConfig>`
  - `ClientsConfig::validate(&self, config: &Config, spt_dir: &Path) -> Result<()>`
  - `is_fika_installed(spt_dir: &Path) -> bool`

- [ ] **Step 1: Write tests for ClientsConfig**

Add tests to the existing `#[cfg(test)] mod tests` in `src/config.rs`:

```rust
#[test]
fn clients_config_defaults() {
    let config: Config = toml::from_str("").expect("empty config");
    assert!(config.clients.is_none());
}

#[test]
fn clients_config_full_deserialization() {
    let toml_str = r#"
[clients]
count = 3
install_dir = "/opt/fika-client"
restart_policy = "auto"
max_restart_attempts = 10
restart_backoff_cap = 600
base_udp_port = 25565
image = "ghcr.io/zhliau/fika-headless-docker:v2.1.0"
isolated_paths = ["BepInEx/config", "BepInEx/cache"]
"#;
    let config: Config = toml::from_str(toml_str).expect("should parse");
    let clients = config.clients.unwrap();
    assert_eq!(clients.count, 3);
    assert_eq!(clients.install_dir, PathBuf::from("/opt/fika-client"));
    assert_eq!(clients.restart_policy, RestartPolicy::Auto);
    assert_eq!(clients.max_restart_attempts, 10);
    assert_eq!(clients.restart_backoff_cap, 600);
    assert_eq!(clients.base_udp_port, 25565);
    assert_eq!(clients.image, "ghcr.io/zhliau/fika-headless-docker:v2.1.0");
    assert_eq!(clients.isolated_paths, vec!["BepInEx/config", "BepInEx/cache"]);
}

#[test]
fn clients_config_minimal_with_defaults() {
    let toml_str = r#"
[clients]
count = 2
install_dir = "/opt/fika"
"#;
    let config: Config = toml::from_str(toml_str).expect("should parse");
    let clients = config.clients.unwrap();
    assert_eq!(clients.count, 2);
    assert_eq!(clients.restart_policy, RestartPolicy::Auto);
    assert_eq!(clients.max_restart_attempts, 5);
    assert_eq!(clients.restart_backoff_cap, 300);
    assert_eq!(clients.base_udp_port, 25565);
    assert_eq!(clients.image, "ghcr.io/zhliau/fika-headless-docker:latest");
    assert_eq!(clients.isolated_paths, vec!["BepInEx/config".to_string()]);
}

#[test]
fn clients_config_validation_port_overflow() {
    let clients = ClientsConfig {
        count: 3,
        install_dir: PathBuf::from("/tmp/fake"),
        restart_policy: RestartPolicy::Auto,
        max_restart_attempts: 5,
        restart_backoff_cap: 300,
        base_udp_port: 65534,
        image: "test".to_string(),
        isolated_paths: vec![],
    };
    let config = Config { server_container: Some("spt".to_string()), ..Config::default() };
    let tmp = tempfile::tempdir().unwrap();
    let spt_dir = tmp.path();
    // Create fika-server dir so fika detection passes
    std::fs::create_dir_all(spt_dir.join("SPT/user/mods/fika-server")).unwrap();
    // Create install_dir
    std::fs::create_dir_all(&clients.install_dir).ok();

    let result = clients.validate(&config, spt_dir);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("port"));
}

#[test]
fn clients_config_validation_no_fika() {
    let clients = ClientsConfig {
        count: 1,
        install_dir: PathBuf::from("/tmp"),
        restart_policy: RestartPolicy::Auto,
        max_restart_attempts: 5,
        restart_backoff_cap: 300,
        base_udp_port: 25565,
        image: "test".to_string(),
        isolated_paths: vec![],
    };
    let config = Config { server_container: Some("spt".to_string()), ..Config::default() };
    let tmp = tempfile::tempdir().unwrap();
    // No fika-server dir
    let result = clients.validate(&config, tmp.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Fika"));
}

#[test]
fn clients_config_env_override_count() {
    temp_env::with_vars([("QUMA_CLIENTS_COUNT", Some("5"))], || {
        let mut config = Config::default();
        config.clients = Some(ClientsConfig::default());
        config.apply_env_overrides();
        assert_eq!(config.clients.unwrap().count, 5);
    });
}

#[test]
fn fika_detection() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(!is_fika_installed(tmp.path()));
    std::fs::create_dir_all(tmp.path().join("SPT/user/mods/fika-server")).unwrap();
    assert!(is_fika_installed(tmp.path()));
}

#[test]
fn restart_policy_serde() {
    assert_eq!(
        serde_json::from_str::<RestartPolicy>(r#""auto""#).unwrap(),
        RestartPolicy::Auto
    );
    assert_eq!(
        serde_json::from_str::<RestartPolicy>(r#""manual""#).unwrap(),
        RestartPolicy::Manual
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p quartermaster config::tests -- --nocapture`
Expected: FAIL — `ClientsConfig`, `RestartPolicy`, `is_fika_installed`, `validate` don't exist yet

- [ ] **Step 3: Implement ClientsConfig, RestartPolicy, validation, env overrides**

Add to `src/config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    Auto,
    Manual,
}

fn default_restart_policy() -> RestartPolicy { RestartPolicy::Auto }
fn default_max_restart_attempts() -> u32 { 5 }
fn default_restart_backoff_cap() -> u64 { 300 }
fn default_base_udp_port() -> u16 { 25565 }
fn default_headless_image() -> String { "ghcr.io/zhliau/fika-headless-docker:latest".to_string() }
fn default_isolated_paths() -> Vec<String> { vec!["BepInEx/config".to_string()] }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClientsConfig {
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub install_dir: PathBuf,
    #[serde(default = "default_restart_policy")]
    pub restart_policy: RestartPolicy,
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,
    #[serde(default = "default_restart_backoff_cap")]
    pub restart_backoff_cap: u64,
    #[serde(default = "default_base_udp_port")]
    pub base_udp_port: u16,
    #[serde(default = "default_headless_image")]
    pub image: String,
    #[serde(default = "default_isolated_paths")]
    pub isolated_paths: Vec<String>,
}

impl Default for ClientsConfig {
    fn default() -> Self {
        Self {
            count: 0,
            install_dir: PathBuf::new(),
            restart_policy: RestartPolicy::Auto,
            max_restart_attempts: 5,
            restart_backoff_cap: 300,
            base_udp_port: 25565,
            image: default_headless_image(),
            isolated_paths: default_isolated_paths(),
        }
    }
}

impl ClientsConfig {
    pub fn validate(&self, config: &Config, spt_dir: &Path) -> Result<()> {
        if self.count == 0 { return Ok(()); }
        if !is_fika_installed(spt_dir) {
            bail!("Fika server mod not found at {}. Dedicated client management requires Fika.",
                spt_dir.join("SPT/user/mods/fika-server").display());
        }
        if self.install_dir.as_os_str().is_empty() || !self.install_dir.exists() {
            bail!("clients.install_dir '{}' does not exist", self.install_dir.display());
        }
        let max_port = self.base_udp_port as u32 + self.count - 1;
        if max_port > 65535 {
            bail!("clients.base_udp_port ({}) + count ({}) exceeds port range (max port would be {})",
                self.base_udp_port, self.count, max_port);
        }
        if config.server_container.is_none() {
            bail!("server_container must be configured for dedicated client management — convergence needs to restart the server");
        }
        Ok(())
    }
}

pub fn is_fika_installed(spt_dir: &Path) -> bool {
    spt_dir.join("SPT/user/mods/fika-server").is_dir()
}
```

Add `clients` field to `Config` with `skip_serializing_if` to avoid polluting existing config files with empty `[clients]` sections on save:

```rust
#[serde(default)]
#[serde(skip_serializing_if = "Option::is_none")]
pub clients: Option<ClientsConfig>,
```

Add env var overrides in `apply_env_overrides()`:

```rust
if let Ok(val) = std::env::var("QUMA_CLIENTS_COUNT") {
    if let Ok(count) = val.parse::<u32>() {
        self.clients.get_or_insert_with(ClientsConfig::default).count = count;
    }
}
if let Ok(val) = std::env::var("QUMA_CLIENTS_INSTALL_DIR") {
    self.clients.get_or_insert_with(ClientsConfig::default).install_dir = PathBuf::from(val);
}
if let Ok(val) = std::env::var("QUMA_CLIENTS_RESTART_POLICY") {
    if let Ok(policy) = serde_json::from_str::<RestartPolicy>(&format!("\"{val}\"")) {
        self.clients.get_or_insert_with(ClientsConfig::default).restart_policy = policy;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p quartermaster config::tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add ClientsConfig for declarative dedicated client management"
```

---

### Task 4: Fika headless API client

**Files:**
- Create: `src/spt/headless.rs` — types for Fika headless API responses
- Modify: `src/spt/mod.rs` — add `pub mod headless;`
- Modify: `src/spt/server.rs` — add `headless_clients()`, `available_headless_clients()`, `headless_restart_config()` methods to `SptClient`
- Test: `src/spt/headless.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `SptClient` from `src/spt/server.rs`
- Produces:
  - `GetHeadlessesResponse { headlesses: HashMap<String, HeadlessClientInfo> }` — `/fika/headless/get`
  - `HeadlessClientInfo { state, players, requester_session_id, level }` — per-client status from the `headlesses` dict
  - `EHeadlessStatus` enum (numeric: `Ready = 1`, `InRaid = 2`)
  - `HeadlessAvailableClient { headless_session_id, alias }` — `/fika/headless/available`
  - `HeadlessRestartConfig { amount }` — `/fika/headless/restartafterraidamount`
  - `SptClient::headless_clients(&self) -> Result<GetHeadlessesResponse>`
  - `SptClient::available_headless_clients(&self) -> Result<Vec<HeadlessAvailableClient>>`
  - `SptClient::headless_restart_config(&self) -> Result<HeadlessRestartConfig>`

**Fika API verified against source** ([Fika-Server-CSharp](https://github.com/project-fika/Fika-Server-CSharp)):
- `GET /fika/headless/get` → `GetHeadlessesResponse` with `[JsonPropertyName("headlesses")]` containing `Dictionary<MongoId, HeadlessClientInfo>`
- `GET /fika/headless/available` → `HeadlessAvailableClients[]` with `[JsonPropertyName("headlessSessionID")]` and `[JsonPropertyName("alias")]`
- `GET /fika/headless/restartafterraidamount` → `GetHeadlessRestartAfterAmountOfRaids` with `[JsonPropertyName("amount")]`
- `EHeadlessStatus` enum: `READY = 1`, `IN_RAID = 2` (C# numeric enum — JSON serialization depends on server config; support both numeric and string via custom deserializer)
- `HeadlessClientInfo` has `State`, `Players` (List<MongoId>), `RequesterSessionID`, `HasNotifiedRequester`, `Level`, `WebSocket` (not serialized) — PascalCase in C#, casing in JSON depends on SPTarkov's serializer config

- [ ] **Step 1: Write tests for headless types deserialization**

Create `src/spt/headless.rs`. Types match the verified Fika-Server-CSharp source. Use `serde(untagged)` on the status enum to handle both numeric and string representations. Use `#[serde(alias)]` for PascalCase/camelCase field name flexibility since the exact JSON casing depends on the SPTarkov serializer config:

```rust
use std::collections::HashMap;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct GetHeadlessesResponse {
    #[serde(default)]
    pub headlesses: HashMap<String, HeadlessClientInfo>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HeadlessClientInfo {
    #[serde(alias = "State")]
    pub state: EHeadlessStatus,
    #[serde(alias = "Players", default)]
    pub players: Vec<String>,
    #[serde(alias = "RequesterSessionID", default)]
    pub requester_session_id: Option<String>,
    #[serde(alias = "HasNotifiedRequester", default)]
    pub has_notified_requester: Option<bool>,
    #[serde(alias = "Level", default)]
    pub level: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EHeadlessStatus {
    Ready,
    InRaid,
    Unknown(Value),
}

impl<'de> Deserialize<'de> for EHeadlessStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let v = Value::deserialize(deserializer)?;
        match &v {
            Value::Number(n) => match n.as_u64() {
                Some(1) => Ok(EHeadlessStatus::Ready),
                Some(2) => Ok(EHeadlessStatus::InRaid),
                _ => Ok(EHeadlessStatus::Unknown(v)),
            },
            Value::String(s) => match s.as_str() {
                "READY" | "Ready" => Ok(EHeadlessStatus::Ready),
                "IN_RAID" | "InRaid" => Ok(EHeadlessStatus::InRaid),
                _ => Ok(EHeadlessStatus::Unknown(v)),
            },
            _ => Ok(EHeadlessStatus::Unknown(v)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeadlessAvailableClient {
    #[serde(alias = "headlessSessionID")]
    pub headless_session_id: String,
    #[serde(default)]
    pub alias: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeadlessRestartConfig {
    pub amount: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_headless_status_numeric() {
        let v: EHeadlessStatus = serde_json::from_str("1").unwrap();
        assert_eq!(v, EHeadlessStatus::Ready);
        let v: EHeadlessStatus = serde_json::from_str("2").unwrap();
        assert_eq!(v, EHeadlessStatus::InRaid);
    }

    #[test]
    fn deserialize_headless_status_string() {
        let v: EHeadlessStatus = serde_json::from_str(r#""READY""#).unwrap();
        assert_eq!(v, EHeadlessStatus::Ready);
        let v: EHeadlessStatus = serde_json::from_str(r#""IN_RAID""#).unwrap();
        assert_eq!(v, EHeadlessStatus::InRaid);
    }

    #[test]
    fn deserialize_headlesses_response() {
        let json = r#"{
            "headlesses": {
                "abc123": {
                    "State": 1,
                    "Players": [],
                    "RequesterSessionID": null,
                    "HasNotifiedRequester": null,
                    "Level": 0
                },
                "def456": {
                    "State": 2,
                    "Players": ["player1", "player2"],
                    "RequesterSessionID": "req789",
                    "HasNotifiedRequester": true,
                    "Level": 15
                }
            }
        }"#;
        let resp: GetHeadlessesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.headlesses.len(), 2);
        let client = &resp.headlesses["abc123"];
        assert_eq!(client.state, EHeadlessStatus::Ready);
        assert!(client.players.is_empty());
        let raiding = &resp.headlesses["def456"];
        assert_eq!(raiding.state, EHeadlessStatus::InRaid);
        assert_eq!(raiding.players.len(), 2);
        assert_eq!(raiding.requester_session_id.as_deref(), Some("req789"));
    }

    #[test]
    fn deserialize_available_clients() {
        let json = r#"[
            {"headlessSessionID": "abc123", "alias": "Headless 1"},
            {"headlessSessionID": "def456", "alias": "Headless 2"}
        ]"#;
        let clients: Vec<HeadlessAvailableClient> = serde_json::from_str(json).unwrap();
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].headless_session_id, "abc123");
        assert_eq!(clients[0].alias, "Headless 1");
    }

    #[test]
    fn deserialize_restart_config() {
        let json = r#"{"amount": 10}"#;
        let config: HeadlessRestartConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.amount, 10);
    }

    #[test]
    fn deserialize_empty_headlesses() {
        let json = r#"{"headlesses": {}}"#;
        let resp: GetHeadlessesResponse = serde_json::from_str(json).unwrap();
        assert!(resp.headlesses.is_empty());
    }
}
```

- [ ] **Step 2: Register module and run tests**

Add `pub mod headless;` to `src/spt/mod.rs`. Run: `cargo test -p quartermaster spt::headless::tests`
Expected: PASS

- [ ] **Step 3: Add HTTP methods to SptClient**

In `src/spt/server.rs`, add methods after `loaded_server_mods()`:

```rust
pub async fn headless_clients(&self) -> Result<crate::spt::headless::GetHeadlessesResponse> {
    let resp = self.client
        .get(format!("{}/fika/headless/get", self.base_url))
        .header("responsecompressed", "0")
        .send()
        .await
        .context("failed to reach Fika headless endpoint")?
        .error_for_status()
        .context("Fika headless endpoint returned error")?;
    resp.json().await.context("failed to parse headless clients response")
}

pub async fn available_headless_clients(&self) -> Result<Vec<crate::spt::headless::HeadlessAvailableClient>> {
    let resp = self.client
        .get(format!("{}/fika/headless/available", self.base_url))
        .header("responsecompressed", "0")
        .send()
        .await
        .context("failed to reach Fika headless available endpoint")?
        .error_for_status()
        .context("Fika headless available endpoint returned error")?;
    resp.json().await.context("failed to parse available headless clients response")
}

pub async fn headless_restart_config(&self) -> Result<crate::spt::headless::HeadlessRestartConfig> {
    let resp = self.client
        .get(format!("{}/fika/headless/restartafterraidamount", self.base_url))
        .header("responsecompressed", "0")
        .send()
        .await
        .context("failed to reach Fika headless restart config endpoint")?
        .error_for_status()
        .context("Fika headless restart config endpoint returned error")?;
    resp.json().await.context("failed to parse headless restart config response")
}
```

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/spt/headless.rs src/spt/mod.rs src/spt/server.rs
git commit -m "feat: add Fika headless API client for dedicated client status"
```

---

### Task 5: Client shared types and supervisor

**Files:**
- Create: `src/client/mod.rs` — shared types (`ClientState`, `ContainerStatus`, `ClientHealth`), Fika detection re-export
- Create: `src/client/supervisor.rs` — `ClientSupervisor` background task
- Modify: `src/main.rs` — add `mod client;`
- Test: `src/client/supervisor.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `ContainerManager` (Task 1), `SptClient` (Task 4), `ClientsConfig` (Task 3), `HeadlessClientStatus` (Task 4)
- Produces:
  - `ClientState`, `ContainerStatus`, `ClientHealth` types
  - `ClientSupervisor::new(container_mgr, spt_client, clients_config, spt_dir, converging, cancel_token) -> Self`
  - `ClientSupervisor::run(self) -> JoinHandle<()>` — spawns the monitoring tokio task
  - `ClientSupervisor::state() -> Arc<RwLock<Vec<ClientState>>>` — shared state readable by web handlers
  - `compute_health(container_running, fika_status, server_up) -> ClientHealth`
  - `backoff_duration(failures, cap) -> Duration`

- [ ] **Step 1: Write tests for health computation and backoff**

Create `src/client/mod.rs` and `src/client/supervisor.rs`. Write tests in `supervisor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_healthy_when_running_and_ready() {
        assert_eq!(
            compute_health(true, Some(EHeadlessStatus::Ready), true),
            ClientHealth::Healthy
        );
    }

    #[test]
    fn health_healthy_when_running_and_in_raid() {
        assert_eq!(
            compute_health(true, Some(EHeadlessStatus::InRaid), true),
            ClientHealth::Healthy
        );
    }

    #[test]
    fn health_degraded_when_running_but_not_connected() {
        assert_eq!(
            compute_health(true, None, true),
            ClientHealth::Degraded
        );
    }

    #[test]
    fn health_degraded_when_server_down() {
        assert_eq!(
            compute_health(true, Some(EHeadlessStatus::Ready), false),
            ClientHealth::Degraded
        );
    }

    #[test]
    fn health_down_when_container_stopped() {
        assert_eq!(
            compute_health(false, None, true),
            ClientHealth::Down
        );
    }

    #[test]
    fn backoff_exponential() {
        assert_eq!(backoff_duration(0, 300), Duration::from_secs(5));
        assert_eq!(backoff_duration(1, 300), Duration::from_secs(10));
        assert_eq!(backoff_duration(2, 300), Duration::from_secs(20));
        assert_eq!(backoff_duration(3, 300), Duration::from_secs(40));
    }

    #[test]
    fn backoff_capped() {
        assert_eq!(backoff_duration(10, 300), Duration::from_secs(300));
        assert_eq!(backoff_duration(100, 300), Duration::from_secs(300));
    }

    #[test]
    fn backoff_custom_cap() {
        assert_eq!(backoff_duration(10, 60), Duration::from_secs(60));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p quartermaster client::supervisor::tests`
Expected: FAIL

- [ ] **Step 3: Implement shared types in `src/client/mod.rs`**

```rust
pub mod supervisor;

use chrono::{DateTime, Utc};
use crate::spt::headless::EHeadlessStatus;

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerStatus {
    Running,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientHealth {
    Healthy,
    Degraded,
    Down,
    GivenUp,
}

#[derive(Debug, Clone)]
pub struct ClientState {
    pub index: u32,
    pub container_name: String,
    pub container_status: ContainerStatus,
    pub fika_status: Option<EHeadlessStatus>,
    pub players: Vec<String>,
    pub cpu_percent: Option<f64>,
    pub memory_mb: Option<f64>,
    pub restart_count: u32,
    pub last_restart: Option<DateTime<Utc>>,
    pub health: ClientHealth,
    pub restarting: bool,
    pub consecutive_failures: u32,
}
```

- [ ] **Step 4: Implement supervisor with health computation and backoff**

In `src/client/supervisor.rs`, implement `compute_health`, `backoff_duration`, and the `ClientSupervisor` struct with its `run()` method. The supervisor loop: check converging flag, ping server, iterate clients, update state, handle auto-restart with backoff, broadcast SSE events.

Key implementation: the `run()` method spawns a `tokio::spawn` that loops on a `tokio::time::interval`, checking the `CancellationToken` via `tokio::select!`.

- [ ] **Step 5: Register module in main.rs**

Add `mod client;` to `src/main.rs`.

- [ ] **Step 6: Run tests**

Run: `cargo test -p quartermaster client::supervisor::tests`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/client/ src/main.rs
git commit -m "feat: add ClientSupervisor with health monitoring and auto-restart backoff"
```

---

### Task 6: Convergence engine

**Files:**
- Create: `src/client/converge.rs` — convergence engine
- Modify: `src/client/mod.rs` — add `pub mod converge;`
- Test: `src/client/converge.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `ContainerManager` (Task 1), `ClientsConfig` (Task 3), `SptClient` (Task 4)
- Produces:
  - `converge(container_mgr, clients_config, config, spt_dir, spt_client, converging_flag) -> Result<()>`
  - `edit_headless_amount(fika_jsonc_path: &Path, amount: u32) -> Result<()>` — text-level replacement
  - `discover_new_profiles(profiles_dir: &Path, before: &HashSet<String>) -> Vec<String>`
  - `setup_client_overlay(spt_dir: &Path, index: u32, install_dir: &Path, isolated_paths: &[String]) -> Result<()>`

- [ ] **Step 1: Write tests for `edit_headless_amount` and `discover_new_profiles`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_headless_amount_preserves_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fika.jsonc");
        std::fs::write(&path, r#"{
    // This is a comment about headless settings
    "headless": {
        "amount": 1, // number of headless clients
        "profiles": {}
    }
}"#).unwrap();

        edit_headless_amount(&path, 3).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#""amount": 3"#));
        assert!(content.contains("// This is a comment"));
        assert!(content.contains("// number of headless"));
    }

    #[test]
    fn edit_headless_amount_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fika.jsonc");
        let result = edit_headless_amount(&path, 3);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn discover_new_profiles_finds_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path();
        std::fs::write(profiles_dir.join("existing123.json"), "{}").unwrap();
        std::fs::write(profiles_dir.join("new456.json"), "{}").unwrap();

        let before: std::collections::HashSet<String> = ["existing123".to_string()].into_iter().collect();
        let new = discover_new_profiles(profiles_dir, &before);
        assert_eq!(new, vec!["new456"]);
    }

    #[test]
    fn setup_client_overlay_copies_isolated_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().join("spt");
        let install_dir = tmp.path().join("install");

        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();
        std::fs::write(install_dir.join("BepInEx/config/test.cfg"), "key=value").unwrap();

        setup_client_overlay(&spt_dir, 1, &install_dir, &["BepInEx/config".to_string()]).unwrap();

        let overlay_file = spt_dir.join("clients/1/BepInEx/config/test.cfg");
        assert!(overlay_file.exists());
        assert_eq!(std::fs::read_to_string(overlay_file).unwrap(), "key=value");
    }

    #[test]
    fn container_name_for_index() {
        assert_eq!(client_container_name(1), "fika-headless-1");
        assert_eq!(client_container_name(10), "fika-headless-10");
    }

    #[test]
    fn client_udp_port() {
        assert_eq!(client_port(25565, 1), 25565);
        assert_eq!(client_port(25565, 3), 25567);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p quartermaster client::converge::tests`
Expected: FAIL

- [ ] **Step 2b: Add test for name collision detection**

```rust
    #[test]
    fn container_name_collision_detected() {
        // This tests the logic that checks if a container exists but lacks the managed-by label.
        // The actual detection uses ContainerManager::detect_containers_by_label vs
        // ContainerManager::inspect — test the decision logic, not the API calls.
        let managed = vec!["fika-headless-1".to_string()];
        let all_matching_name = vec!["fika-headless-1".to_string(), "fika-headless-2".to_string()];
        let conflicts = find_name_conflicts(&managed, &all_matching_name, 3);
        // fika-headless-2 exists but isn't managed, fika-headless-3 doesn't exist
        assert_eq!(conflicts, vec!["fika-headless-2"]);
    }
```

- [ ] **Step 2c: Add test for isolated_paths overlay update**

```rust
    #[test]
    fn update_overlay_copies_new_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().join("spt");
        let install_dir = tmp.path().join("install");

        // Existing overlay from initial setup
        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();
        std::fs::write(install_dir.join("BepInEx/config/test.cfg"), "key=value").unwrap();
        setup_client_overlay(&spt_dir, 1, &install_dir, &["BepInEx/config".to_string()]).unwrap();

        // Now add a new isolated path
        std::fs::create_dir_all(install_dir.join("BepInEx/cache")).unwrap();
        std::fs::write(install_dir.join("BepInEx/cache/data.bin"), "cached").unwrap();
        setup_client_overlay(&spt_dir, 1, &install_dir, &["BepInEx/config".to_string(), "BepInEx/cache".to_string()]).unwrap();

        // Both paths should exist in overlay
        assert!(spt_dir.join("clients/1/BepInEx/config/test.cfg").exists());
        assert!(spt_dir.join("clients/1/BepInEx/cache/data.bin").exists());
    }
```

- [ ] **Step 3: Implement convergence helpers**

Implement `edit_headless_amount`, `discover_new_profiles`, `setup_client_overlay`, `client_container_name`, `client_port`, `find_name_conflicts`.

**`edit_headless_amount`** uses targeted string replacement (NOT full JSON parse/rewrite) to preserve JSONC comments. Strategy: find the `"amount"` key within a `"headless"` block via simple string search, then replace only the numeric value:

```rust
pub fn edit_headless_amount(path: &Path, amount: u32) -> Result<()> {
    if !path.exists() {
        bail!("Fika server mod not configured. Start the SPT server at least once to generate fika.jsonc, then retry.");
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    // Find "amount" key and replace its numeric value
    // Pattern: "amount" followed by optional whitespace, colon, optional whitespace, then digits
    let re = regex::Regex::new(r#"("amount"\s*:\s*)\d+"#)
        .expect("valid regex");

    if !re.is_match(&content) {
        bail!("could not find headless.amount in {}", path.display());
    }

    let updated = re.replace(&content, format!("${{1}}{amount}"));
    std::fs::write(path, updated.as_ref())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
```

Add `regex` to `Cargo.toml` if not already present, or use manual string search to avoid a new dependency.

**`setup_client_overlay`** handles both initial creation and updating when `isolated_paths` changes — it copies any paths that don't yet exist in the overlay, leaving existing files untouched (user may have customized them).

- [ ] **Step 4: Implement main `converge()` function**

The `converge()` function: sets the `converging` flag, detects current containers by label, checks for name conflicts with unmanaged containers, compares count, calls scale_up or scale_down as needed, updates overlays for `isolated_paths` changes on existing clients, clears the flag. Scale-up edits fika.jsonc, restarts server (with profile discovery retry — poll profiles dir for up to 30s after server ready to handle async profile generation), creates containers. Scale-down checks for in-raid clients (CLI prompts via `common::confirm()`, web handler returns error if in-raid and `force != true`), removes containers, edits fika.jsonc, restarts server.

- [ ] **Step 5: Run tests**

Run: `cargo test -p quartermaster client::converge::tests`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/client/converge.rs src/client/mod.rs
git commit -m "feat: add convergence engine for declarative client scaling"
```

---

### Task 7: CLI commands (`quma client`)

**Files:**
- Create: `src/cli/client.rs` — `ClientAction` enum and handler functions
- Modify: `src/cli/mod.rs` — add `pub mod client;`, add `Client` variant to `Command` enum
- Modify: `src/main.rs` — add `Command::Client` match arm

**Interfaces:**
- Consumes: `ContainerManager` (Task 1), `SptClient` (Task 4), `ClientsConfig` (Task 3), `ClientState` (Task 5), `converge()` (Task 6)
- Produces: `quma client {status, logs, restart, scale}` CLI commands

- [ ] **Step 1: Add `ClientAction` enum and `Client` variant**

In `src/cli/mod.rs`, add `pub mod client;` to the module declarations and add to the `Command` enum:

```rust
/// Manage Fika dedicated headless clients
Client {
    #[command(subcommand)]
    action: ClientAction,
},
```

Define `ClientAction`:

```rust
#[derive(Subcommand)]
pub enum ClientAction {
    /// Show dedicated client status
    Status {
        /// Client number for detailed view
        client: Option<u32>,
    },
    /// Stream container logs for a client
    Logs {
        /// Client number
        client: u32,
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
    /// Restart a dedicated client
    Restart {
        /// Client number
        client: u32,
    },
    /// Set the desired number of dedicated clients and converge
    Scale {
        /// Desired number of clients
        count: u32,
    },
}
```

- [ ] **Step 2: Implement `src/cli/client.rs`**

Implement `run()` function handling each `ClientAction` variant. Each checks Fika detection first (`is_fika_installed()`). `status` fetches live data from bollard + Fika API and prints a table. `logs` uses `ContainerManager::log_stream()` with the same `LogOutput` matching pattern from Task 2. `restart` calls `ContainerManager::restart()`. `scale` updates the config file and calls `converge()` — the convergence engine's scale-down path uses `common::confirm()` for in-raid client protection (e.g., "Client 3 is currently in a raid with 2 players. Remove anyway? [y/N]").

- [ ] **Step 3: Add `Command::Client` match arm to `src/main.rs`**

```rust
Command::Client { action } => {
    let ctx = cli::common::resolve_context(&cli)?;
    reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
    cli::client::run(action, &ctx).await
}
```

- [ ] **Step 4: Run `cargo check` and test**

Run: `cargo check && cargo test`
Expected: compiles, all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/cli/client.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add quma client CLI commands for dedicated client management"
```

---

### Task 8: Wire supervisor into `quma serve` startup

**Files:**
- Modify: `src/cli/serve.rs` — create supervisor, run convergence on startup
- Modify: `src/web/state.rs` — add supervisor state and converging flag to `AppState`
- Modify: `src/web/mod.rs` — accept and store supervisor state

**Interfaces:**
- Consumes: `ClientSupervisor` (Task 5), `converge()` (Task 6), `ContainerManager` (Task 1)
- Produces: supervisor running as background task during `quma serve`, shared `ClientState` accessible from `AppState`

- [ ] **Step 1: Add supervisor state to `AppState`**

```rust
pub struct AppState {
    // ... existing fields ...
    pub container_mgr: Option<Arc<crate::container::ContainerManager>>,
    pub client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    pub converging: Arc<std::sync::atomic::AtomicBool>,
    pub fika_installed: bool,
}
```

- [ ] **Step 2: Update `start_server()` signature and `serve.rs`**

Pass `ContainerManager`, client states, and converging flag through to `start_server()`. In `serve.rs`, after config load and before calling `start_server()`:
1. Create `ContainerManager`
2. If `clients.count > 0` and Fika is installed, run `converge()` and spawn `ClientSupervisor`
3. Pass supervisor state to `AppState`

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli/serve.rs src/web/state.rs src/web/mod.rs
git commit -m "feat: wire ClientSupervisor into quma serve startup with convergence"
```

---

### Task 9: Web handlers and templates for `/clients`

**Files:**
- Create: `src/web/handlers/clients.rs` — handlers for all `/clients` routes
- Modify: `src/web/handlers/mod.rs` — add `pub mod clients;`
- Modify: `src/web/mod.rs` — register `/clients` routes
- Create: `templates/clients/list.html` — overview table
- Create: `templates/clients/detail.html` — detail view
- Create: `templates/clients/partials/status.html` — HTMX status partial
- Create: `templates/partials/dashboard_clients_status.html` — dashboard widget
- Modify: `templates/partials/nav.html` — add "Clients" nav entry (conditionally shown when Fika detected)
- Modify: `templates/dashboard.html` — add clients status widget

**Interfaces:**
- Consumes: `AppState` with `client_states` (Task 8), `ContainerManager` (Task 1), `ClientsConfig` (Task 3)
- Produces: web UI for `/clients` and `/clients/{n}`, HTMX partials, SSE integration

- [ ] **Step 1: Create handler stubs**

Create `src/web/handlers/clients.rs` with handlers for each route defined in the spec. Each handler reads from `state.client_states` and renders an Askama template.

- [ ] **Step 2: Create templates**

Create the template files following existing patterns (extend `base.html`, use HTMX attributes, match existing CSS classes). The list page renders a table from `Vec<ClientState>`. The detail page shows full info for one client.

- [ ] **Step 3: Register routes in `src/web/mod.rs`**

Add routes to the authenticated scope and `/api` scope following the existing pattern.

- [ ] **Step 4: Update nav template**

Add "Clients" link to `templates/partials/nav.html`, conditionally shown when Fika is detected. Since the nav is in `base.html` and every page template extends it, add `fika_installed: bool` to `AppState` (set during startup based on `is_fika_installed()`). Each Askama template struct that extends `base.html` needs to include this field. Alternatively, add it as a shared context value via a custom Askama filter or by including it in every template struct's construction from `AppState`.

The simplest approach: store `fika_installed: bool` in `AppState` (set once at startup). Every handler that renders a full page (not a partial) already has access to `state: Data<AppState>` — pass `state.fika_installed` to the template struct. In the nav template, use `{% if fika_installed %}` to conditionally render the Clients link.

- [ ] **Step 5: Add `force` parameter to scale endpoint**

The `POST /clients/scale` handler accepts a form with `count: u32` and `force: bool` (defaults to `false`). If scale-down would remove an in-raid client and `force` is false, return an error flash listing which clients are in-raid. If `force` is true, proceed with removal. The convergence engine's `converge()` function takes a `force: bool` parameter that bypasses the in-raid check.

- [ ] **Step 6: Add dashboard widget**

Add a clients summary widget to `templates/dashboard.html` showing count of healthy/degraded/down clients.

- [ ] **Step 7: Run `cargo check` and manual test**

Run: `cargo check`
Start the dev server: `just serve`
Verify the clients page renders (even if empty when no clients configured).

- [ ] **Step 8: Commit**

```bash
git add src/web/handlers/clients.rs src/web/handlers/mod.rs src/web/mod.rs templates/
git commit -m "feat: add /clients web UI with overview table, detail view, and dashboard widget"
```

---

### Task 10: Integration testing and polish

**Files:**
- All files from previous tasks — fix any compilation issues, clippy warnings, or test failures
- Modify: `TODO.md` — update with completed item

**Interfaces:**
- Consumes: everything from Tasks 1-9
- Produces: passing test suite, clean clippy, updated TODO

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Run fmt**

Run: `cargo fmt`

- [ ] **Step 4: Update TODO.md**

Mark "Fika Dedicated Client Management" as complete in `TODO.md`. Also mark "rust-native podman/docker support (podman-api/bollard)" as complete under "To Investigate".

- [ ] **Step 5: Commit**

Stage only the specific changed files (verify with `git diff --name-only` first):

```bash
git add TODO.md src/ templates/ Cargo.toml Cargo.lock
git commit -m "chore: polish dedicated client feature — tests, clippy, TODO update"
```
