# Fika Dedicated Client Management — Design Spec

## Overview

Add declarative management of Fika headless dedicated clients to Quartermaster. Users configure a desired client count; quma converges to that state by creating/removing Podman containers, generating Fika headless profiles, and monitoring health. Includes a bollard migration to replace the existing CLI-shelling `PodmanClient` for both server and client container management.

## Goals

- Declarative scaling: set `clients.count = N`, quma handles the rest
- Two-layer health monitoring: container state (bollard) + application state (Fika server API)
- Configurable auto-restart with exponential backoff per client
- Live status in web UI with SSE, detailed CLI output
- Replace `PodmanClient` (CLI shelling) with bollard (native Podman API via Docker-compat socket)

## Non-Goals (v1)

- Variable/dynamic scaling based on player load (future)
- Per-client config overrides (all clients share the same restart policy, image, etc.)
- Operation history / event logging for clients (deferred to operation history feature)
- Windows support

## Prerequisites

- `podman.socket` must be enabled (`systemctl --user enable --now podman.socket` for rootless)
- A working Fika client installation directory on the host
- Fika server mod installed on the SPT server

## Upgrade Path

The `[clients]` config section is entirely optional. Existing users who upgrade will have no `[clients]` section, which is equivalent to `count = 0` (disabled). No migration steps needed — all existing behavior is preserved. The bollard migration replaces the container management internals but the external behavior (CLI commands, web handlers) remains identical.

Bollard requires `podman.socket` to be enabled, which is a new prerequisite. On startup, if the socket is unavailable, quma emits a clear error with setup instructions. If container management is not needed (no `server_container` configured and `clients.count = 0`), quma should still function for mod management operations — the `ContainerManager` is lazily validated, not a hard startup gate.

---

## Architecture

### 1. ContainerManager (bollard migration)

**File**: `src/container.rs`

Replaces `PodmanClient` entirely. Wraps a `bollard::Docker` client connected via `Docker::connect_with_podman_defaults()`. Stored as `Arc<Docker>` for sharing across web `AppState` and the supervisor.

**Behavioral change from `PodmanClient`**: The current `PodmanClient::new(container_name)` is cheap and infallible — it just stores a string. `ContainerManager::new()` eagerly connects to the Podman socket, which can fail. This is intentional: we want to detect socket unavailability at startup rather than on first container operation. The `ContainerManager` is created once and shared via `AppState`, rather than created per-request. Bollard handles transient socket disconnections internally — if the socket becomes temporarily unavailable after initial connection, bollard will retry on the next API call.

**API**:

```rust
pub struct ContainerManager {
    docker: Arc<Docker>,
}

pub struct CreateContainerOpts {
    pub name: String,
    pub image: String,
    pub env: Vec<(String, String)>,     // key-value pairs: PROFILE_ID, SERVER_URL, etc.
    pub volumes: Vec<VolumeMount>,
    pub ports: Vec<PortMapping>,
    pub labels: Vec<(String, String)>,  // includes managed-by=quma
}

pub struct VolumeMount {
    pub host_path: PathBuf,
    pub container_path: String,
    pub read_only: bool,
    pub selinux: SelinuxLabel,          // Z (private) | z (shared) | None
}

pub enum SelinuxLabel {
    Private,  // :Z — private unshared label (use for per-client mounts)
    Shared,   // :z — shared label (use for shared read-only base)
    None,
}

pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: Protocol,             // Tcp | Udp
}

impl ContainerManager {
    pub fn new() -> Result<Self>;
    pub async fn start(&self, container: &str) -> Result<()>;
    pub async fn stop(&self, container: &str) -> Result<()>;
    pub async fn restart(&self, container: &str) -> Result<()>;
    pub async fn is_running(&self, container: &str) -> Result<bool>;
    pub async fn inspect(&self, container: &str) -> Result<ContainerInspectResponse>;
    pub fn stats_stream(&self, container: &str) -> impl Stream<Item = Result<Stats>>;
    pub fn log_stream(&self, container: &str, tail: usize, follow: bool) -> impl Stream<Item = Result<LogOutput>>;
    pub fn container_events(&self) -> impl Stream<Item = Result<EventMessage>>;
    pub async fn pull_image(&self, image: &str) -> Result<()>;
    pub async fn create_container(&self, opts: CreateContainerOpts) -> Result<String>;
    pub async fn remove_container(&self, container: &str) -> Result<()>;
    pub async fn detect_containers_by_label(&self, key: &str, value: &str) -> Result<Vec<String>>;
}
```

**`container_events()`** internally filters to container-type events only (bollard's `EventsOptions` with `type_=["container"]`), so callers don't receive image, volume, or network events.

**`create_container()`** always applies the label `managed-by=quma` to containers it creates. This label is used by `detect_containers_by_label()` to distinguish quma-managed containers from user-created ones.

**SELinux**: All volume mounts must specify a `SelinuxLabel`. The shared base install directory uses `:z` (shared), per-client overlay directories use `:Z` (private). This is critical for Fedora/RHEL systems with SELinux enforcing.

**Migration**: All existing `PodmanClient` callsites switch to `ContainerManager`:
- `src/cli/server.rs` — `start`, `stop`, `restart`, `logs`
- `src/cli/serve.rs` — auto-start server
- `src/web/handlers/server.rs` — `start_server`, `stop_server`, `restart_server`
- `src/server_detect.rs` — `is_server_running()` takes `&ContainerManager`

`PodmanClient` (`src/podman.rs`) is deleted after migration.

### 2. Fika Headless API Client

**File**: `src/spt/headless.rs` (new)

Types live in `src/spt/headless.rs`. HTTP methods are added to the existing `SptClient` in `src/spt/server.rs` to keep all HTTP client logic centralized.

**Endpoints**:

| Endpoint | Method | Returns |
|----------|--------|---------|
| `GET /fika/headless/get` | `headless_clients()` | `Vec<HeadlessClientStatus>` |
| `GET /fika/headless/available` | `available_headless_clients()` | `Vec<HeadlessClientStatus>` |
| `GET /fika/headless/restartafterraidamount` | `headless_restart_config()` | `HeadlessRestartConfig` |

**Types**:

```rust
pub struct HeadlessClientStatus {
    pub session_id: String,
    pub state: HeadlessState,
    pub players: Option<Vec<String>>,
    pub requester_session_id: Option<String>,
}

pub enum HeadlessState {
    Ready,
    InRaid,
}

pub struct HeadlessRestartConfig {
    pub restart_after_raids: u32,
}
```

**Correlation**: Each Podman container has a `PROFILE_ID` env var set at creation time. `ContainerManager::inspect()` reads this back from the container's environment. The profile ID is matched against `session_id` from `/fika/headless/get` to produce the combined container + application health view.

**Profile identification for convergence**: During scale-up, new headless profiles need to be detected in `<spt_dir>/SPT/user/profiles/`. Rather than relying on the password field (which may be hashed), use a diff-based approach: snapshot the profile directory before and after the server restart, and identify new files. Cross-reference newly appeared profile IDs against the Fika API response from `/fika/headless/get` after the server comes back up — profiles that appear in both the directory diff and the API response are the new headless profiles. As a fallback, check for the username prefix `headless_` which Fika uses for auto-generated headless profiles.

### 3. Configuration

**File**: `src/config.rs` — add `ClientsConfig` struct

```toml
[clients]
count = 3                          # desired number of headless clients
install_dir = "/opt/fika-client"   # base client installation directory
restart_policy = "auto"            # "auto" | "manual"
max_restart_attempts = 5           # consecutive failures before giving up
restart_backoff_cap = 300          # max backoff seconds (5 min)
base_udp_port = 25565              # clients get sequential ports
image = "ghcr.io/zhliau/fika-headless-docker:latest"

# Paths relative to install_dir that need per-client copies
isolated_paths = [
    "BepInEx/config",
]
```

```rust
pub struct ClientsConfig {
    pub count: u32,
    pub install_dir: PathBuf,
    pub restart_policy: RestartPolicy,  // Auto | Manual
    pub max_restart_attempts: u32,
    pub restart_backoff_cap: u64,       // seconds
    pub base_udp_port: u16,
    pub image: String,
    pub isolated_paths: Vec<String>,
}
```

Defaults: `count = 0` (disabled), `restart_policy = Auto`, `max_restart_attempts = 5`, `restart_backoff_cap = 300`, `base_udp_port = 25565`, `image = ghcr.io/zhliau/fika-headless-docker:latest`, `isolated_paths = ["BepInEx/config"]`.

Environment variable overrides: `QUMA_CLIENTS_COUNT`, `QUMA_CLIENTS_INSTALL_DIR`, `QUMA_CLIENTS_RESTART_POLICY`.

**Validation** (checked at startup and before convergence):
- If `count > 0` but Fika is not installed, fail with a clear error: "Fika server mod not found. Dedicated client management requires Fika." Detection: check for the existence of `<spt_dir>/SPT/user/mods/fika-server/` (the Fika server mod directory).
- If `count > 0` but `install_dir` is not set or doesn't exist, fail with a clear error.
- If `base_udp_port + count - 1 > 65535`, fail with an error explaining the port range overflow.
- If `count > 0` but `server_container` is not configured, fail — convergence needs to restart the server.
- The `[clients]` section is entirely optional — omitting it is equivalent to `count = 0`.

**Fika detection gate**: The entire dedicated client feature (convergence, supervisor, web UI clients page, CLI `quma client` commands) is gated on Fika being installed. Detection checks for `<spt_dir>/SPT/user/mods/fika-server/`. When Fika is not detected:
- `quma client status` prints "Dedicated client management requires Fika server mod."
- `quma client scale` fails with the same message.
- The web UI hides the "Clients" nav entry entirely.
- The supervisor is not spawned.
- `clients.count` in config is ignored (no convergence runs).
This is a runtime check, not a config error — a user may have `[clients]` configured but temporarily remove Fika.

**`isolated_paths` default**: `["BepInEx/config"]` is a best-effort starting point based on known Fika headless client requirements. Users should add additional paths if they observe cross-client interference (e.g., cache directories, save files). The exact set depends on the Fika version and installed mods.

### 4. Per-Client File Isolation

**Directory structure**:

```
<spt_dir>/clients/
├── 1/
│   └── BepInEx/config/    # copied from install_dir
├── 2/
│   └── BepInEx/config/
└── 3/
    └── BepInEx/config/
```

During convergence, for each new client:
1. Create `<spt_dir>/clients/<n>/`
2. For each entry in `isolated_paths`, copy from `install_dir` into the client's overlay directory
3. Patch client-specific values if needed (e.g., port bindings in config files)

Container volume mounts:
- `install_dir:/opt/tarkov:z,ro` — shared base, read-only, SELinux shared label
- `<spt_dir>/clients/<n>/<path>:/opt/tarkov/<path>:Z,rw` — one mount per isolated path, SELinux private label

If `isolated_paths` changes, the next convergence copies new entries for all existing clients.

### 5. Convergence Engine

**File**: `src/client/converge.rs`

Reconciles desired state (`clients.count`) with actual state (existing containers). Runs during `quma serve` startup, on `POST /clients/scale`, and from `quma client scale` CLI.

**Coordination with supervisor**: The convergence engine acquires a shared `Arc<AtomicBool>` (`converging`) before starting and sets it to `true`. The supervisor checks this flag at the start of each tick and skips the tick entirely while convergence is in progress. This prevents the supervisor from attempting to restart containers that are being created or removed.

**Container ownership**: The convergence engine uses `detect_containers_by_label("managed-by", "quma")` to find existing quma-managed client containers. If a container named `fika-headless-N` exists but lacks the `managed-by=quma` label, convergence treats it as a name conflict and fails with an error rather than adopting or overwriting it.

**Scale up** (need N, have M where M < N):
1. Edit `fika.jsonc` at `<spt_dir>/SPT/user/mods/fika-server/assets/configs/fika.jsonc` — set `headless.amount` to N. Use text-level replacement (matching the existing `replace_json_bool()` pattern in `setup.rs`) to preserve user comments. The `json_comments` crate (already a dependency) can be used for reading; writes use targeted string replacement on the raw file content.
2. If `fika.jsonc` does not exist, fail with a clear error: "Fika server mod not configured. Start the SPT server at least once to generate fika.jsonc, then retry."
3. Snapshot existing profile IDs from `<spt_dir>/SPT/user/profiles/`
4. Drain pending queue operations if `auto_drain_on_lifecycle` is enabled (matching the existing server restart behavior in `cli/server.rs`)
5. Restart the SPT server container
6. Wait for server ready (ping loop, same timeout pattern as `wait_for_ping()`)
7. Diff profiles directory to find newly generated profile IDs. Cross-reference with `/fika/headless/get` and the `headless_` username prefix as fallback.
8. For each new client:
   a. Create overlay directory with isolated paths
   b. Create container with `managed-by=quma` label: image, env vars (`PROFILE_ID`, `SERVER_URL`, `SERVER_PORT`), volume mounts (with SELinux labels), UDP port (`base_udp_port + n - 1`)
   c. Start container
9. Wait for clients to appear in `/fika/headless/get` (up to ~5 min per client)

**Scale down** (need N, have M where M > N):
1. Check Fika API for in-raid status of clients being removed (highest-numbered first)
2. If any target client is `IN_RAID`:
   - From CLI: warn and prompt for confirmation ("Client 3 is currently in a raid with 2 players. Remove anyway?")
   - From web UI: return an error flash and don't proceed. User must wait for the raid to end or explicitly confirm via a `force=true` parameter.
3. Stop and remove excess containers (highest-numbered first)
4. Clean up overlay directories
5. Update `headless.amount` in `fika.jsonc` to N (text-level replacement)
6. Drain queue if `auto_drain_on_lifecycle` is enabled
7. Restart the SPT server

**Recover** (containers exist but are stopped):
- Handled by the supervisor, not the convergence engine

**Container naming**: `fika-headless-1`, `fika-headless-2`, etc. Deterministic and sequential.

**UDP ports**: `base_udp_port + n - 1`. Client 1 = 25565, client 2 = 25566, etc.

### 6. ClientSupervisor

**File**: `src/client/supervisor.rs`

Background tokio task spawned during `quma serve`. Monitors all managed clients on a configurable tick interval (default 15s).

**Convergence guard**: At the start of each tick, check the shared `converging` flag (`Arc<AtomicBool>`). If `true`, skip the entire tick. This prevents conflicts with the convergence engine creating/removing containers.

**Per-tick logic**:
1. **Check server liveness first**: Ping the SPT server. If unreachable, mark all clients as `Degraded` (reason: server down), skip individual client health checks, and do NOT trigger any auto-restarts. Restarting clients when the server is down is pointless and would burn through `max_restart_attempts`.
2. For each configured client:
   a. Check container state via `ContainerManager::is_running()`
   b. If running, match container's `PROFILE_ID` (from inspect) against Fika API response
   c. Update shared `ClientState` map
   d. If unhealthy and `restart_policy == Auto` and server is up:
      - Increment consecutive failure counter
      - If `failures < max_restart_attempts`: schedule restart with exponential backoff (`min(5 * 2^failures, backoff_cap)` seconds)
      - If `failures >= max_restart_attempts`: mark as `GivenUp`, log error, emit SSE event
3. Broadcast state changes via SSE

**Backoff reset**: The failure counter resets when a client reaches `READY` state in the Fika API — not just "container running". This prevents restart loops where the container starts but the client never connects.

**Interaction with manual CLI operations**: If a user runs `quma client restart 2` while the supervisor is running, the supervisor will observe the container state change on its next tick and update accordingly. No special coordination needed — the supervisor is read-observe-react, not write-first. The failure counter for a manually restarted client resets on the next tick where the client reaches `READY`.

**Shared state**:

```rust
pub struct ClientState {
    pub index: u32,                          // 1-based client number
    pub container_name: String,
    pub container_status: ContainerStatus,   // Running | Stopped | Unknown
    pub fika_status: Option<HeadlessState>,  // Ready | InRaid | None (not connected)
    pub players: Vec<String>,
    pub cpu_percent: Option<f64>,
    pub memory_mb: Option<f64>,
    pub restart_count: u32,
    pub last_restart: Option<DateTime<Utc>>,
    pub health: ClientHealth,
    pub restarting: bool,                    // synthetic: quma issued a restart, waiting for it to come back
}

pub enum ContainerStatus {
    Running,
    Stopped,
    Unknown,
}

pub enum ClientHealth {
    Healthy,        // container running + READY or IN_RAID in Fika
    Degraded,       // container running but not connected to Fika, or server is down
    Down,           // container stopped
    GivenUp,        // exceeded max_restart_attempts
}
```

`ContainerStatus` does not include `Restarting` — Podman has no native restarting state. Instead, the `restarting` bool on `ClientState` is set synthetically by quma when it issues a restart, and cleared when the container reaches `Running` + Fika `READY` on a subsequent tick.

State stored in `Arc<RwLock<Vec<ClientState>>>`. The supervisor owns writes; web handlers and CLI read.

**Supervisor only runs during `quma serve`**. CLI commands do live checks directly against bollard + Fika API.

**Shutdown**: Graceful via `tokio_util::sync::CancellationToken`. Stops the monitoring loop; does not stop client containers (they continue running independently).

### 7. CLI Commands

**File**: `src/cli/client.rs`

New top-level `quma client` command:

```
quma client status [N]          # overview table, or detailed status for client N
quma client logs <N> [--follow] # stream container logs for client N
quma client restart <N>         # manual restart of client N
quma client scale <N>           # set clients.count = N and converge immediately
```

**`quma client status`** output (no argument):

```
CLIENT  CONTAINER          STATUS   FIKA      PLAYERS  CPU   RAM
1       fika-headless-1    running  in_raid   3        45%   2.1G
2       fika-headless-2    running  ready     0        12%   1.8G
3       fika-headless-3    stopped  —         —        —     —
```

**`quma client status 1`** — detailed view: container state, Fika status, player list (resolved to names if possible), resource usage, restart count, last restart time, configured policy.

**`quma client scale <N>`** — updates `clients.count` in `quartermaster.toml` and runs the convergence engine immediately. Requires `server_container` to be configured (convergence needs to restart the server). If `quma serve` is running simultaneously, the CLI convergence and the web server's supervisor coordinate via the shared `converging` flag — but in practice, `quma client scale` is a standalone operation. If run while `quma serve` is active, the web server picks up the new config on its next supervisor tick after convergence completes.

**Config change propagation**: The CLI writes to `quartermaster.toml` on disk. If `quma serve` is running, it does NOT automatically detect config file changes. The canonical way to change client count while the web server is running is via `POST /clients/scale` (which updates both in-memory `AppState` and the config file). If a user changes the config file manually and wants the running server to pick it up, they restart `quma serve`.

All CLI commands work standalone — they connect to bollard and the Fika API directly, no supervisor required.

### 8. Web UI

**Overview page**: `/clients` — new nav entry

- Table with columns: Client #, Container Status, Fika Status, Players, CPU, RAM, Actions (restart/stop/start)
- Health indicators: green (healthy), yellow (degraded), red (down), grey (given up)
- Scale control: input to change client count with an "Apply" button (triggers `POST /clients/scale`)
- Initial page load fetches current state via `GET /api/clients/status` (HTMX partial). Subsequent updates arrive via SSE from the supervisor. The polling endpoint serves as the initial load and as a fallback for clients without SSE support.

**Detail view**: `/clients/{n}`

- Full container info (uptime, image, ports)
- Fika status with player list
- Live resource usage (CPU, RAM via on-demand stats streaming — only active while the detail page is open)
- Log viewer with filtering and follow mode (reuses patterns from existing logs page)
- Restart history (restart count and last restart timestamp from `ClientState`)
- Manual restart/stop/start buttons

**Templates**:
- `templates/clients/list.html` — overview table
- `templates/clients/detail.html` — detail view
- `templates/clients/partials/status.html` — HTMX partial for live status updates
- `templates/partials/dashboard_clients_status.html` — dashboard widget showing client summary

**Routes**:

| Method | Path | Handler | Auth |
|--------|------|---------|------|
| GET | `/clients` | `client_list` | authenticated |
| GET | `/clients/{n}` | `client_detail` | authenticated |
| POST | `/clients/{n}/restart` | `client_restart` | admin |
| POST | `/clients/{n}/stop` | `client_stop` | admin |
| POST | `/clients/{n}/start` | `client_start` | admin |
| POST | `/clients/scale` | `client_scale` | admin |
| GET | `/api/clients/status` | `client_status_partial` | authenticated |
| GET | `/api/clients/{n}/stats` | `client_stats_stream` | authenticated |
| GET | `/api/clients/{n}/logs` | `client_logs_stream` | authenticated |

---

## Dependencies

### New Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `bollard` | Podman/Docker API client (replaces CLI shelling) |
| `tokio-util` | `CancellationToken` for supervisor graceful shutdown |

`futures-util` is already an explicit dependency in `Cargo.toml`. `json_comments` is already a dependency (used by `setup.rs` for `fika.jsonc` reading).

### Removed

| Crate/Code | Reason |
|------------|--------|
| `src/podman.rs` (`PodmanClient`) | Replaced by `ContainerManager` |

---

## File Structure

```
src/
├── container.rs              # NEW — ContainerManager (bollard wrapper)
├── client/
│   ├── mod.rs                # NEW — module root, shared types (ClientState, etc.)
│   ├── converge.rs           # NEW — convergence engine
│   └── supervisor.rs         # NEW — background health monitor
├── spt/
│   └── headless.rs           # NEW — Fika headless API types (methods added to SptClient)
├── cli/
│   └── client.rs             # NEW — quma client subcommands
├── web/
│   └── handlers/
│       └── clients.rs        # NEW — web handlers for /clients
templates/
├── clients/
│   ├── list.html             # NEW — client overview table
│   ├── detail.html           # NEW — client detail view
│   └── partials/
│       └── status.html       # NEW — HTMX status partial
└── partials/
    └── dashboard_clients_status.html  # NEW — dashboard widget
```

Modified files:
- `src/config.rs` — add `ClientsConfig`
- `src/cli/mod.rs` — add `Client` variant
- `src/cli/server.rs` — migrate to `ContainerManager`
- `src/cli/serve.rs` — start supervisor, run convergence, create `ContainerManager`
- `src/server_detect.rs` — take `&ContainerManager` instead of creating `PodmanClient`
- `src/web/mod.rs` — add client routes
- `src/web/state.rs` — add `ContainerManager`, supervisor state, `converging` flag to `AppState`
- `src/web/handlers/server.rs` — migrate to `ContainerManager`
- `templates/base.html` — add "Clients" nav entry
- `templates/dashboard.html` — add clients status widget
- `Cargo.toml` — add `bollard`, `tokio-util`

Deleted files:
- `src/podman.rs`

---

## Testing Strategy

- **ContainerManager**: Unit tests with mock/stub where possible. Integration tests against a real Podman socket for CI environments that support it (gated behind a feature flag or env var).
- **Convergence engine**: Unit tests with a mock ContainerManager and mock filesystem (tempdir). Test scale-up, scale-down, recovery, name collision detection, in-raid protection, fika.jsonc missing, and port overflow validation.
- **Supervisor**: Unit tests for health state transitions, backoff calculation, failure counter logic, convergence guard skip behavior, and server-down detection (skip client checks when server unreachable).
- **Fika API client**: Unit tests with mock HTTP responses for the `/fika/headless/*` endpoints. Test correlation logic (profile ID matching via container inspect env var).
- **CLI**: Test output formatting. Functional tests for `scale` command against a temp config file.
- **Config**: Deserialize/serialize round-trip tests for `ClientsConfig`. Env var override tests. Validation tests (port overflow, missing install_dir, missing server_container).
