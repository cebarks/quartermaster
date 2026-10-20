# Quartermaster (`quma`)

A CLI and web dashboard for managing server-side mods on an [SPT (Single Player Tushonka)](https://sp-mod.com)/[Fika](https://github.com/project-fika) dedicated server. Installs, updates, and removes mods from [SPT Forge](https://sp-mod.com), with a web UI for server hosts and connected players.

Built for Linux hosts running SPT in a Podman container.

## SPT Compatibility

The SPT server image includes .NET and ASP.NET Core 9 and 10 for legacy servers and [SPT 4.1.6](https://github.com/SP-Tushonka/build/releases/tag/4.1.6).
Setup discovers the latest available release from `SP-Tushonka/build` rather than pinning a specific version.
SPT 4.1.6 requires EFT **0.16.9.5.40743** for game and headless clients.

SPT 4.1.6 supports most **4.1.x mods**, not **4.0.x mods**. Its release notes promise profile compatibility
with **4.1.x and newer**, not 4.0.x. Check Fika and all other mods against the target SPT version before
upgrading. Back up the installation and follow the upstream upgrade instructions; updating server files
alone does not update game clients. With the current SPT Launcher, configure the remote
server address manually: the bootstrap scripts' automatic launcher-config edit targets the legacy
launcher format.

New installations preserve upstream's `SPT_Runtime/` server directory. Quartermaster also supports
legacy `SPT/` server directories; this is separate from its flat versus nested data-directory layout.
In-place updates that switch between these runtime directories are rejected before the server is
stopped or files are overwritten. Use a clean target installation and explicitly migrate compatible
mods/profiles instead. Installations containing valid server markers in both directories are rejected
as ambiguous.

Server communication requires the current **launcher v2 HTTP API** (`/launcher/v2/`); the
old launcher HTTP endpoints are not supported. Legacy directory management does not imply
legacy HTTP API support. Container health probes use HTTPS and verify SPT's generated
certificate. Launcher v2 profile registration is username/edition-based; Quartermaster
accounts retain their separate password authentication.

Release extraction and client exports use directory-scoped I/O. Unsafe archive/ZIP names
and links escaping their permitted root are rejected or omitted. Existing relative or
absolute symlinks confined to that root are preserved, including links to missing targets
created during release extraction. Archive-provided symlink entries remain rejected.
Strict client exports also omit client-named links targeting either server-runtime tree;
confined client-to-client links remain supported. Bundle exports are discovered separately
and constrained to their bundle directory. Server/client classification assumes runtime
directory names and aliases stay stable during export; it is not an atomic filesystem
snapshot against hostile concurrent renames or retargeting. Capability confinement still
prevents outside-installation reads independently of that classification prerequisite.

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

Requires Rust (2021 edition). `just audit` also requires Python 3 and cargo-audit 0.22.2; see [security audits](CONTRIBUTING.md#security-audits) for setup and the scoped risk acceptance.

```bash
just build      # cargo build
just test       # cargo test
just lint       # fmt + clippy + logging conventions + copy-paste detection
just serve      # cargo run -- serve
just run <ARGS> # cargo run -- <ARGS>
just audit      # dependency audit with tested, version-scoped policy
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

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, workflow, and code style guidelines.

## AI Disclosure

Portions of this codebase were implemented with the assistance of LLM-based tools (Claude Code). All architecture, design decisions, and direction were done by a human (Senior Product Security Engineer w/ 10+ years of coding experience), the LLM was used as an implementation aid under continuous human supervision and review.

## License

[AGPL-3.0](LICENSE)
