# Logging System Improvements — Design Spec

## Overview

Overhaul quartermaster's logging system to provide comprehensive diagnostic visibility, user-configurable output, CLI verbosity controls, and a web UI for viewing both application and server container logs.

**Current state:** 17 log statements confined to web handlers, no CLI verbosity flags, no config options, no web UI log viewer. Log level controlled only via `RUST_LOG` env var.

## 1. Config Schema

New `[logging]` section in `quartermaster.toml`:

```toml
[logging]
level = "info"                  # global default: trace|debug|info|warn|error

[logging.console]
enabled = true                  # can disable console output entirely
format = "text"                 # "text" or "json"

[logging.file]
enabled = false                 # off by default
path = "quartermaster.log"     # relative to spt_dir, or absolute
format = "json"                 # "text" or "json"
rotation = "none"               # "none", "size", or "daily"
max_size_mb = 10                # only used when rotation = "size"
max_files = 5                   # kept for both rotation modes

[logging.web]
buffer_size = 1000              # ring buffer entries for the web UI log viewer
```

### Defaults & Behavior

- All fields have serde defaults. Omitting `[logging]` entirely preserves current behavior.
- `level` sets the floor. CLI flags can only increase verbosity at runtime.
- `file.path` resolves relative to `spt_dir` when not absolute.
- Env var overrides are limited to the most commonly adjusted settings:
  - `QUMA_LOG_LEVEL` — overrides `logging.level`
  - `QUMA_LOG_FILE_PATH` — overrides `logging.file.path`
  - `QUMA_LOG_FILE_ENABLED` — overrides `logging.file.enabled` (set to `true`/`false`)
  - Other logging config (format, rotation, buffer_size) is only configurable via the TOML file.
- `RUST_LOG` is still supported as a legacy escape hatch. It sits between the config file and `QUMA_LOG_LEVEL` in the priority chain: config file < `RUST_LOG` < `QUMA_LOG_LEVEL` < `-v`/`-vv` < `--log-level`. When `RUST_LOG` is set, it replaces the entire `EnvFilter` string (supporting full tracing filter syntax like `info,hyper=warn`).
- Sensitive values (`forge_token`, `session_secret`) are never logged at any level.
- Use `#[serde(skip_serializing_if = "LoggingConfig::is_default")]` on the `logging` field so the `[logging]` table only appears in saved config files when the user has explicitly configured it.

## 2. CLI Flags

New global flags on the `Cli` struct:

| Flag | Effect |
|------|--------|
| `-v` | Sets `quartermaster=debug` (crate-scoped) |
| `-vv` | Sets `quartermaster=trace` (crate-scoped) |
| `--log-level <level>` | Sets level globally (all crates) |

### Priority Chain (highest wins)

1. `--log-level <level>` CLI flag
2. `-v` / `-vv` CLI flags
3. `QUMA_LOG_LEVEL` env var
4. `RUST_LOG` env var (legacy, supports full tracing filter syntax)
5. `[logging] level` in config file
6. Hardcoded default: `info` globally, `debug` for `quartermaster` crate

When both `-v` and `--log-level` are present, `--log-level` wins (explicit > shorthand). `-v` is a no-op when the effective level is already `debug` or lower. These flags affect runtime only — they do not modify the config file.

## 3. Logging Coverage Expansion

### `src/podman.rs` — container management

- `debug!` on every podman command invocation (command + args)
- `trace!` on raw command stdout/stderr output
- `warn!` on non-zero exit codes that are handled (not fatal)
- `error!` on command failures that propagate up

### `src/ops.rs` — mod operations

- `info!` on mod lifecycle events (install started/completed, update started/completed, removal)
- `debug!` on file operations (extraction paths, file counts, symlinks)
- `trace!` on individual file-level operations during archive extraction
- `warn!` on recoverable issues (missing optional files, fallback behavior)

### `src/config.rs` — config loading

- `debug!` on config file path resolution (which path was chosen and why)
- `debug!` on env var overrides being applied (which var, what it overrode)
- `trace!` on full parsed config (redacting `forge_token` and `session_secret`)

### `src/cli/` — command execution

- `debug!` on command dispatch (which subcommand, resolved spt_dir)
- `println!` retained for user-facing CLI output — logging is for diagnostics, not UX

### `src/web/` — expand existing coverage

- `debug!` on state changes (queue operations, config updates via UI)
- `trace!` on request-level detail beyond what `TracingLogger` already captures

### Level Philosophy

- `info` — what happened (operator-facing)
- `debug` — how it happened (developer debugging)
- `trace` — everything (deep diagnostics)

## 4. In-Memory Broadcast + Ring Buffer Architecture

### Subscriber Stack

```
┌─────────────────────────────────────────────────────┐
│                 tracing subscriber                   │
│                                                     │
│  ┌───────────┐  ┌───────────┐  ┌────────────────┐  │
│  │ fmt layer │  │ fmt layer │  │ broadcast layer│  │
│  │ (console) │  │  (file)   │  │  (web UI)      │  │
│  └─────┬─────┘  └─────┬─────┘  └───────┬────────┘  │
│        │              │                │            │
│        ▼              ▼                ▼            │
│     stderr        log file      broadcast channel   │
│                                   + ring buffer     │
└─────────────────────────────────────────────────────┘
```

### LogEntry Struct

```rust
struct LogEntry {
    timestamp: DateTime<Utc>,
    level: Level,
    target: String,               // module path
    message: String,
    fields: HashMap<String, serde_json::Value>, // structured tracing fields
}
```

Fields are collected from `tracing::Event` via a custom `tracing::field::Visit` implementation that records each field into the `HashMap<String, serde_json::Value>`.

### Broadcast Layer

A custom `tracing::Layer` implementation:

- On each event, serializes to `LogEntry` and sends to `tokio::sync::broadcast::Sender`
- Simultaneously pushes to a `VecDeque<LogEntry>` behind `Arc<std::sync::RwLock<>>` (the ring buffer — must be `std::sync::RwLock`, not `tokio::sync::RwLock`, because `Layer::on_event()` is synchronous)
- Ring buffer evicts oldest entries when it exceeds `buffer_size` (configurable, default 1000)
- Slow/absent receivers get `RecvError::Lagged` — they catch up from the ring buffer

### Web API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/logs` | GET | Log viewer page with tabs |
| `/api/logs/app?limit=N` | GET | Last N entries from ring buffer (JSON, default limit=100) |
| `/api/logs/app/stream` | GET | SSE stream from broadcast channel |
| `/api/logs/server` | GET | Recent container logs via `podman logs --tail N` |
| `/api/logs/server/stream` | GET | SSE stream via `podman logs --follow` |

### SSE Stream Behavior

SSE is implemented via the `actix-web-lab` crate (`Sse` type), since Actix-web 4 has no built-in SSE support.

- Events sent as `data: {json LogEntry}\n\n`
- On `RecvError::Lagged(n)`, sends `event: lagged` so the UI can display "missed N entries"
- Enable `.with_keep_alive(Duration::from_secs(15))` — keepalive writes are what trigger disconnect detection in Actix-web

### Container Log Streaming

- Spawns `podman logs --follow --tail 0 <container>` as a child process
- A spawned tokio task reads stdout/stderr and sends lines via an `mpsc::Sender` to the SSE response
- When `tx.send()` returns `Err` (receiver dropped on client disconnect), the task kills the child process — this is necessary because Actix-web does not reliably drop the `Stream` on client disconnect
- Returns appropriate error/status when container isn't running

## 5. Web UI Log Viewer

### Page Layout

New `/logs` page accessible from main navigation.

**Tab bar** at top: "Application Logs" | "Server Logs"

### Controls (per tab)

- **Follow toggle** — switches between static and live tail
  - Off (default): shows last N entries from JSON endpoint, manual refresh button
  - On: connects to SSE endpoint, new entries append at bottom, auto-scrolls
- **Level filter** — button group to show/hide by level (error/warn/info/debug/trace). App Logs tab only.
- **Search** — text filter across message and fields (client-side filtering)
- **Clear** — clears displayed log (UI-only, doesn't affect ring buffer or files)

### Log Entry Rendering

- Timestamp, colored level badge, target module, message
- Structured fields as `key=value` pairs, muted style
- Monospace font, one line per entry (wrapping for long messages)
- Color coding: red (error), yellow (warn), default (info), grey (debug/trace)

### Server Logs Tab Differences

- No level filter (container logs are plain text, not structured)
- Search works via plain text matching
- Follow toggle uses SSE from `podman logs --follow`
- Shows status indicator when container isn't running

### Implementation

Plain HTML + vanilla JS, consistent with existing Askama template + HTMX patterns. The `/logs` page template is a struct with `#[derive(Template)]` and `#[template(path = "logs.html")]`, matching the compile-time template approach used throughout the project. SSE via `EventSource` API.

## 6. Subscriber Initialization

### Problem

The current subscriber is set up in `main()` before CLI parsing, so config and CLI flags aren't available yet. `tracing::subscriber::set_global_default` can only be called once.

### Solution: `reload::Layer`

1. **Early bootstrap** — before CLI parsing, set up subscriber with `reload::Layer` wrapping the `EnvFilter`. Console fmt layer + broadcast layer active at `info` default. File layer initialized as `Option<Box<dyn Layer<S>>>` = `None` inside a `reload::Layer`.

2. **Reconfigure after resolution** — once config + CLI flags are resolved:
   - Update the `EnvFilter` via the reload handle to the effective log level
   - Swap file layer from `None` to `Some(file_layer)` if `file.enabled`
   - Console and file format (text vs json): since `fmt::Layer` bakes the format type into a compile-time generic parameter, use `Box<dyn Layer<S>>` type erasure — construct either a text or json layer, box it, and reload via the handle

**Constraint:** Per-layer filters (`.with_filter()`) must NOT be applied to any layer wrapped in `reload::Layer` — this causes a runtime panic because the reloaded `Filtered` layer has no `FilterId`. Use global filtering via `EnvFilter` exclusively.

3. **Pass state to web server** — if running `serve` command, pass broadcast `Sender` + ring buffer `Arc` to `AppState`

### Initialization Sequence

```
main()
  → set up reload-capable subscriber (console fmt + broadcast layer, info default)
  → parse CLI args
  → load config
  → resolve effective log level (config → env → CLI flags)
  → update filter via reload handle
  → conditionally enable file layer based on config
  → if `serve` command: pass broadcast Sender + ring buffer Arc to AppState
  → run command
```

## 7. New Dependencies

| Crate | Purpose |
|-------|---------|
| `tracing-appender` | File output with daily rotation |
| `rolling-file` | Size-based log file rotation (`tracing-appender` only supports time-based) |
| `actix-web-lab` | SSE support for Actix-web 4 (no built-in SSE in actix-web 4) |

`chrono` and `serde_json` are already in the dependency tree. `tokio::sync::broadcast` is available via `tokio`.

## 8. Testing Strategy

- **Config parsing:** unit tests for `LoggingConfig` deserialization, defaults, env var overrides
- **CLI flag resolution:** unit tests for the priority chain (config < env < -v < --log-level)
- **Broadcast layer:** integration test — emit events, verify they arrive in the ring buffer and on the broadcast channel
- **Web endpoints:** integration tests against the JSON endpoints (ring buffer contents, container log fallback)
- **SSE streaming:** integration tests via `actix-web::test::TestServer` — connect to SSE endpoint, emit log events, verify `data:` lines in the response stream. Manual browser testing for UX verification.
