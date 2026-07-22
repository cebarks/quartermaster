# Quartermaster (`quma`)

A CLI and web dashboard for managing server-side mods on an [SPT](https://www.sp-tarkov.com)/[Fika](https://github.com/project-fika) dedicated server. Installs, updates, and removes mods from [SPT Forge](https://forge.sp-tarkov.com), with a web UI for server hosts and connected players.

Built for Linux hosts running SPT in a Podman container.

## Features

- **Mod management** — Install, update, and remove mods and addons by name, slug, or Forge ID with automatic dependency resolution
- **Web dashboard** — Real-time server status, mod browsing, task progress via SSE, log streaming
- **Change queue** — Mod operations queue while the server is running, then apply on restart
- **Health checks** — Server liveness, version verification, mod load checks, file integrity (SHA256)
- **Multi-user auth** — Invite-based registration, admin/player roles, session cookies
- **Fika support** — Dedicated headless client management, scaling, and Fika settings configuration
- **HTTPS/WSS proxy** — Transparent reverse proxy to SPT server so clients connect through Quartermaster
- **Convoy mod distribution** — Built-in mod distribution system for syncing mods to connected clients (replaces NarcoNet/modsync)
- **Player profiles** — View player profiles with quest, trader, and hideout progress, plus stash viewer
- **Raid statistics** — Per-raid stats tracking via proxy interception, with leaderboard
- **Server Value Modifiers (SVM)** — Browse and configure server value modifiers from the web UI
- **Settings management** — View and manage Quartermaster configuration from the web UI
- **Mod requests** — Players can request mods; admins review and approve with a voting system
- **Mod enable/disable** — Toggle mods on and off without uninstalling
- **Backup/restore** — Per-mod and full snapshots of mod files, profiles, and config with CLI and web UI support
- **Join page** — Client bootstrapping with mod archive download and setup scripts
- **Give Items** — Admin tool to send items to players via the Fika API
- **Fika settings** — Edit Fika server configuration (fika.jsonc) from the web UI
- **Mod config management** — Git-backed config history with diff viewer
- **Notes** — Shared admin/player notes with pinning and visibility controls
- **Reindex** — Rebuild file tracking from Forge archives when tracking gets out of sync
- **RBAC** — Role-based access control with admin and player roles
- **Container lifecycle** — Start, stop, restart the SPT server container via Podman
- **Systemd integration** — Generate and install a systemd service for the web UI

## Quick Start

```bash
# Build
cargo build --release

# Initialize for your SPT server directory
quma setup /path/to/spt-server

# Start the web UI
quma serve
```

## CLI Reference

```
quma [OPTIONS] <COMMAND>

Commands:
  setup       Bootstrap or initialize Quartermaster for an SPT server
  install     Install a mod and its dependencies
  update      Update installed mods
  remove      Remove an installed mod
  list        List installed mods
  check       Check all installed mods for updates
  reindex     Rebuild file tracking index by re-downloading archives from Forge
  status      Run health checks against SPT server and mod integrity
  server      Manage the SPT server container
  headless    Manage Fika headless clients
  serve       Start the Quartermaster web UI
  generate    Generate configuration files
  invite      Generate an invite code for a player
  backup      Backup mods, profiles, and config
  restore     Restore from a backup
  spt         Manage the SPT server installation (version, check, update)
  apply       Apply queued mod operations
  migrate     Migrate from legacy directory layout to new layout

Options:
  --quma-dir <QUMA_DIR>      Explicit Quartermaster data directory
  --config <CONFIG>          Config file path override
  -v, --verbose              Increase verbosity (-v debug, -vv trace)
  --log-level <LOG_LEVEL>    Set log level (trace, debug, info, warn, error)
  --log-format <LOG_FORMAT>  Console log format (compact, full, json)
  -V, --version              Print version
```

Mods can be referenced by name, Forge slug, or numeric Forge ID. Use `--force` on install/update/remove to bypass the change queue and apply immediately.

## Web UI

Start with `quma serve` (default: `0.0.0.0:9190`).

The dashboard provides mod management, server controls, real-time log streaming, player profiles, mod request voting, and admin tools. Players register via invite codes; admins can manage users and approve mod requests. Includes a transparent HTTPS/WSS reverse proxy so game clients connect to the SPT server through Quartermaster.

Built with HTMX and server-sent events — no JavaScript build step required.

## Configuration

Config lives at `<spt_dir>/quartermaster.toml`. All settings can be overridden with `QUMA_*` environment variables. See [USAGE.md](USAGE.md) for the full configuration reference.

```bash
# Override via environment
QUMA_DIR=~/spt-server quma status
```

## Documentation

- [USAGE.md](USAGE.md) — Full CLI reference, web UI guide, and configuration reference
- [CONTRIBUTING.md](CONTRIBUTING.md) — Development setup, workflow, and code style
- [docs/forge-api-notes.md](docs/forge-api-notes.md) — Undocumented Forge API quirks and behaviors
- [docs/spt-remote-client-connectivity.md](docs/spt-remote-client-connectivity.md) — SPT remote client connectivity investigation and proxy setup

## Development

Requires Rust (2021 edition).

```bash
just build      # cargo build
just test       # cargo test
just lint       # fmt + clippy + logging conventions + copy-paste detection
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
| `src/forge/` | HTTP client for SPT Forge API, response cache |
| `src/fika/` | Fika API client, fika.jsonc config, headless session stats |
| `src/spt/` | SPT directory interaction, archive extraction, profiles, game data |
| `src/client/` | Fika headless client supervisor and convergence |
| `src/headless/` | Headless client service layer (scaling, lifecycle, operation tracking) |
| `src/svm/` | Server Value Modifier browsing and configuration |
| `src/config_mgmt/` | Git-backed mod config history and diffing |
| `src/ops.rs` | Core mod operations (install/update/remove) |
| `src/backup.rs` | Mod backup/restore (per-mod and full snapshots) |
| `src/health.rs` | Health check system |
| `src/queue.rs` | Change queue for deferred mod operations |
| `src/convoy/` | Convoy mod distribution catalog, downloads, and migration |
| `src/container.rs` | Podman container management |
| `src/dirs.rs` | Directory layout resolution (`QumaDirs`) with legacy migration |
| `src/headless_sync.rs` | Fika headless file sync to client overlays |
| `src/numa.rs` | NUMA-aware container CPU pinning |
| `src/logging/` | Structured logging (console, file, SQLite, SSE broadcast) |
| `src/config.rs` | Config types, TOML serialization, `QUMA_*` env overrides |

## AI Disclosure

Portions of this codebase were implemented with the assistance of LLM-based tools (Claude Code). All architecture, design decisions, and direction were done by a human (Senior Product Security Engineer w/ 10+ years of coding experience), the LLM was used as an implementation aid under continuous human supervision and review.

## License

[AGPL-3.0](LICENSE)
