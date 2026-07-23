# Usage Guide

This guide covers day-to-day usage of Quartermaster. For installation and build instructions, see the [README](README.md).

## Initial Setup

Bootstrap Quartermaster for your SPT server:

```bash
quma setup                          # interactive — prompts for everything
quma setup --quma-dir /path/to/dir  # explicit data directory
```

This creates the data directory layout, pulls the SPT server container image, downloads the selected SPT version, sets up an admin account, and optionally installs Fika.

Setup flags for non-interactive use:

```bash
quma setup --quma-dir /path/to/dir \
  --admin-password "yourpassword" \
  --spt-version "3.10.5" \
  --no-fika                        # skip Fika installation
```

Additional setup options:

| Flag | Purpose |
|------|---------|
| `--dev` | Use a separate container name for development |
| `--container-name <NAME>` | Override the container name (useful for parallel environments) |

## Managing Mods

### Installing

Mods can be referenced by name, Forge slug, numeric ID, URL, or local file path:

```bash
quma install "Big Brain"       # by name (searches Forge)
quma install big-brain         # by slug
quma install 42                # by Forge ID
quma install big-brain 1.2.0   # specific version
quma install ./mod.zip         # from local file
quma install ./mod.zip --name "My Mod"  # local file with custom name
quma install --addon music-pack  # install a Forge addon
```

Dependencies are resolved and installed automatically.

### Updating

```bash
quma update                    # update all installed mods
quma update big-brain          # update a specific mod
quma update --addon music-pack # update an addon
```

### Removing

```bash
quma remove big-brain          # remove a mod (prompts for confirmation)
quma remove big-brain -y       # skip confirmation
quma remove --addon music-pack # remove an addon
```

### Listing & Checking

```bash
quma list                      # list installed mods
quma list --json               # JSON output
quma check                     # check all installed mods for updates
quma status                    # health checks (server, mods, integrity)
quma status --json             # JSON output
```

### Change Queue

When the SPT server is running, mod operations are queued by default and applied when the server stops. Use `--force` to bypass the queue:

```bash
quma install big-brain --force   # install immediately, even if server is running
quma update big-brain --force    # update immediately
quma remove big-brain --force    # remove immediately
```

### Reindex

Rebuild the file tracking index by re-downloading archives from Forge. Useful if the database's file-to-mod mapping gets out of sync:

```bash
quma reindex                   # dry-run — shows what would change
quma reindex --apply           # apply changes
```

## Server Management

Control the SPT server container:

```bash
quma server start
quma server stop
quma server restart
quma server recreate           # stop, remove, and recreate the container
quma server logs               # tail container logs
```

## SPT Server Version Management

Manage the SPT server installation itself:

```bash
quma spt version               # show installed SPT server version
quma spt check                 # check for SPT server updates
quma spt update                # update the SPT server to the latest version
```

## Headless Clients (Fika)

Manage Fika dedicated headless clients:

```bash
quma headless status           # show client status
quma headless create           # create a new headless client
quma headless delete 1         # delete client #1
quma headless scale 3          # set desired client count
quma headless start 1          # start client #1
quma headless stop 1           # stop client #1
quma headless restart 1        # restart client #1
quma headless graceful-restart 1  # graceful restart via Fika API
quma headless rename 1         # rename a headless client (sets Fika alias)
quma headless rebuild          # tear down and recreate all client containers and overlays
quma headless logs 1           # stream client logs
```

Headless clients require Fika to be installed and a `[headless]` section in your config. See [Configuration](#headless-clients-1) below.

## Backups

```bash
quma backup                    # full snapshot (mods, profiles, config)
quma backup big-brain          # backup a specific mod
quma backup --list             # list existing backups
quma restore                   # restore from latest backup (interactive)
quma restore <backup-id>       # restore a specific backup
quma restore --latest big-brain  # restore latest backup for a specific mod
quma restore -f <backup-id>   # skip confirmation
```

## Migration

Migrate from the legacy flat directory layout to the new structured layout:

```bash
quma migrate                   # migrate (interactive)
quma migrate --dry-run         # show migration plan without making changes
```

## Web UI

Start the web dashboard:

```bash
quma serve                     # default: 0.0.0.0:9190
quma serve --port 8080         # custom port
quma serve --bind 127.0.0.1    # bind to localhost only
```

The web UI provides:

- **Dashboard** — server status, mod list, update notifications
- **Mod management** — install, update, remove, enable/disable mods via browser
- **Mod requests** — players request mods, admins review with a voting system
- **Player profiles** — quest, trader, and hideout progress, stash viewer
- **Raid statistics** — per-raid stats with leaderboard
- **Server controls** — start/stop/restart the SPT container
- **Headless management** — scale and monitor Fika headless clients
- **SVM configuration** — browse and edit Server Value Modifier settings
- **Convoy** — built-in client mod syncing (replaces NarcoNet/modsync)
- **Log viewer** — real-time log streaming via SSE
- **Admin tools** — user management, invite codes, server settings, notes

### Authentication

Players register via invite codes generated by admins:

```bash
quma invite                    # generate a one-time invite code
quma invite --expires 24h      # invite that expires in 24 hours
```

### Systemd Service

Generate and install a systemd user service for the web UI:

```bash
quma generate systemd
```

## Configuration

Config lives at `<quma_dir>/quartermaster.toml`. All settings can be overridden with `QUMA_*` environment variables.

### Core Settings

```toml
quma_dir = "/path/to/quartermaster"  # data directory (also accepts legacy "spt_dir")

# Web UI
web_bind = "0.0.0.0"           # default
web_port = 9190                # default
external_url = "https://tarkov.example.com"  # public-facing URL for proxy rewrites
server_name = "My Server"      # display name in the web UI

# Server container (set during `quma setup`)
# server_container = "spt-server"  # container name (no default — set by setup)
# server_host = "127.0.0.1"       # SPT server host (no default — auto-detected)
# server_port = 6969               # SPT server port (no default — uses SPT's 6969)
# server_image = "ghcr.io/cebarks/quartermaster/spt-server:latest"  # container image
auto_start_server = true       # start container when web UI starts (default)
on_exit = "nothing"            # nothing | stop | remove — what to do with the container on exit
container_stop_timeout = 10    # seconds to wait for container stop (default)

# Mod operations
queue_changes = true           # queue changes while server is running (default)
auto_drain_on_lifecycle = false # auto-apply queue on server stop
update_check_interval = 300    # seconds between update checks (default: 5 min)
update_disabled_mods = false   # include disabled mods in update checks (default: false)
forge_cache_ttl = 86400        # Forge response cache TTL in seconds (default: 24h)
leaderboard_min_raids = 5      # minimum raids to appear on leaderboard (default)
snapshots_enabled = true       # enable profile snapshots (default)
```

### HTTPS Proxy

Quartermaster can proxy client connections to the SPT server, enabling raid tracking and metrics:

```toml
proxy_enabled = true           # default
proxy_auth = true              # require authentication for proxy connections (default)
tls_enabled = true             # default — generates self-signed cert if no cert/key provided
tls_cert = "/path/to/cert.pem" # optional — use your own TLS cert
tls_key = "/path/to/key.pem"   # optional
```

### Scanner Guard

Rate-limits and bans IPs making excessive unhandled requests through the proxy (e.g., web scanners):

```toml
[scanner_guard]
enabled = true                 # default
threshold = 20                 # consecutive 404/405 responses before banning (default)
ban_duration = 3600            # ban duration in seconds (default: 1 hour)
```

### Backups

```toml
[backup]
auto_backup = true             # backup before mod operations (default)
backup_dir = "backups"         # relative to quma_dir (default)
max_backups = 3                # per-mod backup retention (default)
require_backup = false         # fail operations if backup fails (default: false)
```

### Convoy (Client Mod Sync)

Convoy is the built-in client mod syncing system. It replaces the legacy NarcoNet/modsync integration. Existing `[modsync]` config sections are automatically migrated to `[convoy]` on first load.

```toml
[convoy]
enabled = true                 # default
exclusions = ["**/*.nosync"]   # glob patterns to exclude from sync
```

### Headless Clients

```toml
[headless]
restart_policy = "auto"        # auto | manual (default: auto)
max_restart_attempts = 5       # before giving up (default)
restart_backoff_cap = 300      # max backoff in seconds between restarts (default)
base_udp_port = 25565          # each client gets base + index (default)
image = "ghcr.io/cebarks/quartermaster/headless:latest"  # container image
server_ready_timeout = 120     # seconds to wait for SPT server ready (default)
memory_restart_threshold = 20000  # MB — restart client when memory exceeds this (default)
isolated_paths = ["BepInEx/config"]  # paths isolated per client (default)

# Wine/Proton settings
ntsync = true                  # default
esync = false                  # default
fsync = false                  # default

# Network settings
# force_ip = "192.168.1.100"   # override IP for Fika connections
# use_upnp = false             # use UPnP for port mapping (default: false)

# CPU pinning
physical_cores_only = false    # only assign physical cores, not hyperthreads (default)

# NUMA pinning (for multi-socket servers)
numa_auto = false              # round-robin clients across NUMA nodes
# numa_node = 0               # pin all clients to a specific node
# numa_pin_memory = false      # also pin memory allocation to the NUMA node

[[headless.clients]]           # one entry per desired client
# Per-client overrides:
# image = "custom:v1"
# numa_node = 1
# cpuset_cpus = "0-7,16-23"
# cpuset_mems = "0"
# extra_isolated_paths = ["BepInEx/plugins/testing"]

[[headless.clients]]           # second client
```

### Setup ZIP

Controls what gets included in the client setup ZIP available on the join page:

```toml
[setup_zip]
exclude_server_files = true    # exclude server-only files (default)
exclude_non_essential = true   # exclude non-essential files (default)
exclude_patterns = ["**/*.pdb"]  # additional glob patterns to exclude
include_patterns = []          # force-include patterns
```

### Logging

```toml
[logging]
level = "info"                 # global level (default)

[logging.console]
enabled = true                 # default
format = "compact"             # compact | full | json (default: compact)

[logging.file]
enabled = true                 # default
path = "logs/quartermaster.log"  # relative to quma_dir (default)
format = "json"                # text | json (default: json)
level = "debug"                # default
rotation = "daily"             # none | size | daily (default: daily)
max_size_mb = 10               # for size rotation (default)
max_files = 7                  # default

[logging.web]
buffer_size = 1000             # SSE buffer (default)
level = "info"                 # default
retention_days = 7             # SQLite log retention (default)
max_entries = 100000           # max stored log entries (default)
```

### Environment Variable Overrides

The following `QUMA_*` environment variables override their corresponding config values:

| Variable | Config field | Type |
|----------|-------------|------|
| `QUMA_DIR` | `quma_dir` | path |
| `QUMA_SPT_DIR` | `quma_dir` (deprecated alias) | path |
| `QUMA_CONFIG` | config file path | path |
| `QUMA_WEB_BIND` | `web_bind` | string |
| `QUMA_WEB_PORT` | `web_port` | integer |
| `QUMA_WEB_WORKERS` | `web_workers` | integer |
| `QUMA_SERVER_CONTAINER` | `server_container` | string |
| `QUMA_SERVER_HOST` | `server_host` | string |
| `QUMA_SERVER_PORT` | `server_port` | integer |
| `QUMA_SERVER_NAME` | `server_name` | string |
| `QUMA_EXTERNAL_URL` | `external_url` | string |
| `QUMA_AUTO_START_SERVER` | `auto_start_server` | bool |
| `QUMA_ON_EXIT` | `on_exit` | nothing/stop/remove |
| `QUMA_CONTAINER_STOP_TIMEOUT` | `container_stop_timeout` | integer |
| `QUMA_UPDATE_CHECK_INTERVAL` | `update_check_interval` | integer |
| `QUMA_FORGE_CACHE_TTL` | `forge_cache_ttl` | integer |
| `QUMA_TLS_ENABLED` | `tls_enabled` | bool |
| `QUMA_TLS_CERT` | `tls_cert` | path |
| `QUMA_TLS_KEY` | `tls_key` | path |
| `QUMA_PROXY_ENABLED` | `proxy_enabled` | bool |
| `QUMA_PROXY_AUTH` | `proxy_auth` | bool |
| `QUMA_SNAPSHOTS_ENABLED` | `snapshots_enabled` | bool |
| `QUMA_LEADERBOARD_MIN_RAIDS` | `leaderboard_min_raids` | integer |
| `QUMA_LOG_LEVEL` | `logging.level` | string |
| `QUMA_LOG_CONSOLE_FORMAT` | `logging.console.format` | compact/full/json |
| `QUMA_LOG_FILE_ENABLED` | `logging.file.enabled` | bool |
| `QUMA_LOG_FILE_PATH` | `logging.file.path` | string |
| `QUMA_LOG_FILE_LEVEL` | `logging.file.level` | string |
| `QUMA_AUTO_BACKUP` | `backup.auto_backup` | bool |
| `QUMA_BACKUP_DIR` | `backup.backup_dir` | string |
| `QUMA_MAX_BACKUPS` | `backup.max_backups` | integer |
| `QUMA_REQUIRE_BACKUP` | `backup.require_backup` | bool |
| `QUMA_HEADLESS_RESTART_POLICY` | `headless.restart_policy` | auto/manual |
| `QUMA_HEADLESS_SERVER_READY_TIMEOUT` | `headless.server_ready_timeout` | integer |
| `QUMA_SCANNER_GUARD_ENABLED` | `scanner_guard.enabled` | bool |
| `QUMA_SCANNER_GUARD_THRESHOLD` | `scanner_guard.threshold` | integer |
| `QUMA_SCANNER_GUARD_BAN_DURATION` | `scanner_guard.ban_duration` | integer |

```bash
QUMA_DIR=/path/to/quma quma serve
QUMA_WEB_PORT=8080 quma serve
QUMA_LOG_LEVEL=debug quma serve
```

## Verbosity

```bash
quma -v status                 # debug logging
quma -vv status                # trace logging
quma --log-level trace status  # explicit level
quma --log-format json serve   # JSON console output
```
