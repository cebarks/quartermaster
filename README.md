# Quartermaster (`quma`)

A CLI and web dashboard for managing server-side mods on an [SPT](https://www.sp-tarkov.com)/[Fika](https://github.com/project-fika) dedicated server. Installs, updates, and removes mods from [SPT Forge](https://forge.sp-tarkov.com), with a web UI for server hosts and connected players.

Built for Linux hosts running SPT in a Podman container.

## Features

- **Mod management** — Install, update, and remove mods by name, slug, or Forge ID with automatic dependency resolution
- **Web dashboard** — Real-time server status, mod browsing, task progress via SSE, log streaming
- **Change queue** — Mod operations queue while the server is running, then apply on restart
- **Health checks** — Server liveness, version verification, mod load checks, file integrity (SHA256)
- **Multi-user auth** — Invite-based registration, admin/player roles, session cookies
- **Fika support** — Dedicated headless client management and scaling for Fika multiplayer
- **HTTPS/WSS proxy** — Transparent reverse proxy to SPT server so clients connect through Quartermaster
- **NarcoNet integration** — Auto-manages NarcoNet `config.yaml` from installed mod state for client mod syncing
- **Player profiles** — View player profiles with quest, trader, and hideout progress, plus stash viewer
- **Raid statistics** — Per-raid stats tracking via proxy interception, with leaderboard
- **Server Value Modifiers (SVM)** — Browse and configure server value modifiers from the web UI
- **Server settings** — View and manage SPT server configuration
- **Mod requests** — Players can request mods; admins review and approve with a voting system
- **Mod enable/disable** — Toggle mods on and off without uninstalling
- **Backup/restore** — Per-mod and full snapshots of mod files, profiles, and config with CLI and web UI support
- **RBAC** — Role-based access control with admin and player roles
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
  setup       Bootstrap or initialize Quartermaster for an SPT server
  install     Install a mod and its dependencies
  update      Update installed mods (specific or all)
  remove      Remove an installed mod
  list        List installed mods
  check       Check all installed mods for updates
  status      Run health checks against SPT server and mod integrity
  server      Manage the SPT server container (start/stop/restart/logs)
  headless    Manage Fika headless clients
  serve       Start the Quartermaster web UI
  generate    Generate configuration files (systemd service)
  invite      Generate an invite code for a player
  backup      Backup mods, profiles, and config
  restore     Restore from a backup

Options:
  --spt-dir <PATH>      Explicit SPT server directory
  --config <PATH>       Config file path override
  -v, --verbose         Increase verbosity (-v debug, -vv trace)
  --log-level <LEVEL>   Set log level (trace, debug, info, warn, error)
```

Mods can be referenced by name, Forge slug, or numeric Forge ID. Use `--force` on install/update/remove to bypass the change queue and apply immediately.

## Web UI

Start with `quma serve` (default: `0.0.0.0:9190`).

The dashboard provides mod management, server controls, real-time log streaming, player profiles, mod request voting, and admin tools. Players register via invite codes; admins can manage users and approve mod requests. Includes a transparent HTTPS/WSS reverse proxy so game clients connect to the SPT server through Quartermaster.

Built with HTMX and server-sent events — no JavaScript build step required.

## Configuration

Config lives at `<spt_dir>/quartermaster.toml`. All settings can be overridden with `QUMA_*` environment variables.

```bash
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
just audit      # cargo audit
just changelog  # git-cliff changelog generation
```

**Local dev environment** (bootstraps a real SPT dev environment at `.dev-server/`):

```bash
just dev-init       # bootstrap SPT dev environment via quma setup
just dev-serve      # build & run web UI against .dev-server/
just dev-cli <ARGS> # run any quma command against .dev-server/
just dev-watch      # auto-rebuild & restart on file changes (needs cargo-watch)
just dev-seed       # seed dev database with test data (wipes & repopulates)
just dev-reset-db   # wipe .dev-server/ database (keeps config & structure)
just dev-clean      # remove .dev-server/ and container entirely
just dev-info       # show dev environment settings (port, container, worktree)
```

Dev recipes are worktree-aware — each git worktree gets a unique port and container name for parallel development.

## Architecture

Single Rust binary — the CLI and actix-web server share the same codebase.

| Layer | Purpose |
|-------|---------|
| `src/cli/` | One file per CLI subcommand (clap derive) |
| `src/web/` | actix-web server, HTMX templates (Askama), SSE, HTTPS/WSS proxy |
| `src/db/` | SQLite via rusqlite (WAL mode), RBAC, backup metadata |
| `src/forge/` | HTTP client for SPT Forge API |
| `src/spt/` | SPT directory interaction, archive extraction, profiles, game data |
| `src/client/` | Fika headless client supervisor and convergence |
| `src/svm/` | Server Value Modifier browsing and configuration |
| `src/ops.rs` | Core mod operations (install/update/remove) |
| `src/backup.rs` | Mod backup/restore (per-mod and full snapshots) |
| `src/health.rs` | Health check system |
| `src/queue.rs` | Change queue for deferred mod operations |
| `src/modsync.rs` | NarcoNet config.yaml auto-management |
| `src/container.rs` | Podman container management |

## License

[AGPL-3.0](LICENSE)
