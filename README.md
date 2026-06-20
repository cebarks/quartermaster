# Quartermaster (`quma`)

A CLI and web dashboard for managing server-side mods on an [SPT](https://www.sp-tarkov.com)/[Fika](https://github.com/project-fika) dedicated server. Installs, updates, and removes mods from [SPT Forge](https://forge.sp-tarkov.com), with a web UI for server hosts and connected players.

Built for Linux hosts running SPT in a Podman container.

## Features

- **Mod management** — Install, update, and remove mods by name, slug, or Forge ID with automatic dependency resolution
- **Web dashboard** — Real-time server status, mod browsing, task progress via SSE, log streaming
- **Change queue** — Mod operations queue while the server is running, then apply on restart
- **Health checks** — Server liveness, version verification, mod load checks, file integrity (SHA256)
- **Multi-user auth** — Invite-based registration, admin/player roles, session cookies
- **Fika support** — Dedicated headless client management for Fika multiplayer
- **Container lifecycle** — Start, stop, restart the SPT server container via Podman
- **Systemd integration** — Generate and install a systemd service for the web UI

## Quick Start

```bash
# Build
cargo build --release

# Initialize for your SPT server directory
quma init /path/to/spt-server

# Start the web UI
quma serve
```

## CLI Reference

```
quma [OPTIONS] <COMMAND>

Commands:
  setup       Interactive guided setup for Fika multiplayer
  init        Initialize Quartermaster for an SPT server
  install     Install a mod and its dependencies
  update      Update installed mods (specific or all)
  remove      Remove an installed mod
  list        List installed mods
  track       Associate an unmanaged mod with a Forge entry
  check       Check all installed mods for updates
  apply       Apply pending queued operations
  status      Run health checks against SPT server and mod integrity
  server      Manage the SPT server container (start/stop/restart/logs/create)
  client      Manage Fika dedicated headless clients
  serve       Start the Quartermaster web UI
  generate    Generate configuration files (systemd service)
  invite      Generate an invite code for a player
  config      View and modify configuration

Options:
  --spt-dir <PATH>      Explicit SPT server directory
  --config <PATH>       Config file path override
  -v, --verbose         Increase verbosity (-v debug, -vv trace)
  --log-level <LEVEL>   Set log level (trace, debug, info, warn, error)
```

Mods can be referenced by name, Forge slug, or numeric Forge ID. Use `--force` on install/update/remove to bypass the change queue and apply immediately.

## Web UI

Start with `quma serve` (default: `0.0.0.0:9190`).

The dashboard provides mod management, server controls, real-time log streaming, and admin tools. Players register via invite codes; admins can manage users and approve mod requests.

Built with HTMX and server-sent events — no JavaScript build step required.

## Configuration

Config lives at `<spt_dir>/quartermaster.toml`. All settings can be overridden with `QUMA_*` environment variables.

```bash
# View current config
quma config

# Set a value
quma config set server.bind "0.0.0.0"

# Override via environment
QUMA_SPT_DIR=~/spt-server quma status
```

## Development

Requires Rust (2021 edition).

```bash
just build      # cargo build
just test       # cargo test
just lint       # cargo fmt + cargo clippy
just serve      # cargo run -- serve
just run <ARGS> # cargo run -- <ARGS>
```

## Architecture

Single Rust binary — the CLI and actix-web server share the same codebase.

| Layer | Purpose |
|-------|---------|
| `src/cli/` | One file per CLI subcommand (clap derive) |
| `src/web/` | actix-web server, HTMX templates (Askama), SSE |
| `src/db/` | SQLite via rusqlite (WAL mode) |
| `src/forge/` | HTTP client for SPT Forge API |
| `src/spt/` | SPT directory interaction, archive extraction, profiles |
| `src/ops.rs` | Core mod operations (install/update/remove) |
| `src/health.rs` | Health check system |
| `src/queue.rs` | Change queue for deferred mod operations |
| `src/container.rs` | Podman container management |

## License

[AGPL-3.0](LICENSE)
