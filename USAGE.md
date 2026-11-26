# Usage Guide

This guide covers day-to-day usage of Quartermaster. For installation and build instructions, see the [README](README.md).

## Initial Setup

Bootstrap Quartermaster for your SPT server:

```bash
quma setup                          # interactive — prompts for everything
quma setup --quma-dir /path/to/dir  # explicit data directory
quma init --quma-dir /path/to/dir   # equivalent alias
```

This creates the data directory layout, pulls the SPT server container image, downloads the selected SPT version, sets up an admin account, and optionally installs Fika.

Setup flags for non-interactive use:

```bash
quma setup --quma-dir /path/to/dir \
  --admin-password "yourpassword" \
  --spt-version "latest" \
  --no-fika                        # skip Fika installation
```

Additional setup options:

| Flag | Purpose |
|------|---------|
| `--dev` | Use a separate container name for development |
| `--container-name <NAME>` | Override the container name (useful for parallel environments) |

Without an explicit container name or `--dev`, rerunning setup retains the container recorded in the installation's configuration. Fresh installations default to `spt-server`.

For a custom directory, include it in subsequent commands (or set `QUMA_DIR`):

```bash
quma --quma-dir /path/to/dir serve
quma --quma-dir /path/to/dir server start
```

The default login URL is `https://<server-host>:9190/quma/login`. TLS is enabled by default with a self-signed certificate. If the default `spt-server` container already belongs to another installation, choose a distinct `--container-name` rather than deleting it.

## Player Client Bootstrap

The **Client Setup** page (`/quma/setup`) provides Linux and Windows scripts for an existing, owned SPT client matching the running server. Invite downloads use the same implementation with the invite-scoped archive URL; their ZIP now serves the same full filtered client pack, not just Convoy. Script and archive fetches still validate the invite. After registration consumes an invite, use the authenticated Client Setup page for fresh downloads. The instance archive supplies its matching Fika/Convoy client components; a separate Fika Installer is not needed.

Close the game and launcher first. Linux requires Bash and Python 3; Windows requires Windows PowerShell 5.1 or PowerShell 7. The scripts do not acquire/install EFT or SPT, initialize Wine/Proton, install system packages, or modify player profiles.

```bash
bash quma-bootstrap.sh --spt-dir "/path/to/SPT" --dry-run
bash quma-bootstrap.sh --spt-dir "/path/to/SPT"  # review plan and confirm
# For an already-approved unattended run, add --yes.
```

Review the downloaded script first. If Windows PowerShell blocks downloaded scripts, use an explicit, process-only invocation:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\quma-bootstrap.ps1 -SptDir 'C:\Games\SPT' -DryRun
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\quma-bootstrap.ps1 -SptDir 'C:\Games\SPT'  # review plan and confirm
# For an already-approved unattended run, add -Yes.
```

This does not change the system's execution policy and cannot override organization policy. With PowerShell 7, invoke `pwsh -NoProfile -File` instead.

Omit the target argument when running from the SPT root. An already downloaded instance ZIP can be supplied with `--archive PATH` / `-ArchivePath PATH`; this is an operator-supplied trusted archive, not an arbitrary mod installer. ZIP64 archives are supported and processed with bounded buffers.

The pre-download gate reserves the advertised archive size plus a 16 MiB safety margin. After download, before staging or replacing client files, the remaining-free-space gate conservatively requires `2*E + 2*B + 2*(Lnew + Lold) + 16 MiB`: `E` is every expanded ZIP file, `B` is existing non-preserved archive-target bytes, and `Lnew`/`Lold` are the new/existing launcher-settings bytes (unchanged settings count at their existing size). This covers simultaneously staged and installed copies, backups, and atomic replacement/rollback temporary copies. The downloaded archive already occupies disk and is not counted again in this remaining-space bound.

ZIP components containing `~` are rejected portably to exclude Windows short-name aliases. On Windows, each existing destination component is also expanded with `GetLongPathNameW`; a long leaf differing from the requested leaf (ignoring case) is rejected, including custom short aliases without a tilde. Existing destination links/reparse points remain prohibited. Use long destination names rather than aliases.

Both scripts validate SPT/EFT compatibility metadata, reject unsafe ZIP paths and destination links/reparse points, verify ZIP checksums before replacing client files, and configure `SPT_Runtime/user/Launcher/LauncherSettings.json` (or the equivalent supported runtime directory). The server entry uses a scheme-less `IpAddress`; existing server IDs, other servers, preferred profiles and Linux settings are preserved. The launcher itself still performs its normal first-run/game preparation checks.

Existing editable files under `BepInEx/config/` (`.cfg`, `.ini`, `.json`, `.jsonc`, `.yaml`, `.yml`, `.toml`, `.xml`) are preserved. Missing defaults are seeded. Resource assets in that directory are updated with backups like other mod files. Files unrelated to the archive are not deleted. Changed existing files and a recovery journal are retained under `.quma-bootstrap-backups/`; a caught application failure restores replaced files, while an interrupted transaction blocks a subsequent run and identifies its backup location for recovery. Do not delete that evidence or blindly restore it over newer player changes.

A dry run does not change the client. Cancellation, malformed settings, failed downloads, incompatible versions, unsafe archives and incomplete installation all exit nonzero without a completion banner. A successful bootstrap verifies files and the saved launcher entry; it is not a native game launch, multiplayer or Wine/Proton test.

Developers can run `just test-bootstrap --require-powershell` to execute both generated scripts against disposable fixtures. On Linux, point `DOTNET_ROOT`/`DOTNET_ROOT_X64` at a runtime compatible with the installed `pwsh` apphost. Linux-hosted PowerShell tests do not establish native Windows behavior.

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

### Legacy integrity baselines

Tracked files use XXH3-128. Legacy checksums remain pending rather than being rehashed automatically on startup; NULL baselines stay unknown. `quma status --json` exposes `integrity.unverified_files` with record IDs and reasons. Back up and inspect the selected files before explicitly trusting their current bytes:

```bash
quma --quma-dir /path/to/installation rebaseline --file-id 42 --file-id 43 --trust-current
```

This records a new baseline, not a check against the old checksum. Unselected records and historical evidence remain intact. Missing, unreadable, hidden or changed files stay pending; retry the unresolved IDs after fixing the cause. See [Convoy migration and rollback](docs/convoy-client-files.md#paired-upgrade-and-rollback).

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
quma headless reconcile-exclusions 1 --check  # inspect policy for an already stopped client
quma headless reconcile-exclusions 1          # apply policy without starting the client
```

Headless clients require a matching EFT/SPT client installation in `<quma_dir>/headless/`, Fika to be installed, and a `[headless]` section in your config. The client must also have SPT's launcher-supplied assembly patches applied; downgrading EFT and extracting the SPT archive alone does not complete client preparation. See [Configuration](#headless-clients-1) below. Scaling generates dedicated Fika profiles and restarts the SPT server; schedule it when players are disconnected. NUMA-driven recreation validates the replacement profile assignment before stopping or removing the existing container. First boot allows up to 180 seconds for Fika's config generation, which runs after locale initialization. During scale-down, an unmount failure stops the operation before overlay-directory deletion; the data is preserved even if the container has already been removed.

Groups marked `exclude_headless` take effect at the next safe managed first boot, start, restart or recovery; saving a group does not hot-delete files or restart a live client. Preparation and lifecycle control share a per-client operation lock. Invalid policy, unsupported original container/backing state or failed reconciliation prevents start; busy guarded operations fail rather than bypassing preparation. The offline `reconcile-exclusions` command requires an already stopped original managed client and an existing supported database, never performs migrations or lifecycle control, and supports the global `--quma-dir` option. Its `--check` mode creates no policy state, locks, masks or mounts. See [the complete safety and database contract](docs/headless-exclusions.md). A newly installed CLI does not retrofit a running older daemon.

Managed headless clients connect through the local HTTPS proxy using their assigned profile IDs. With proxy authentication enabled, Quartermaster verifies a direct loopback connection and the live managed container's labels, configured index, name, running state, and profile assignment. A `headless_` username alone does not grant proxy access. Fika's `GET /fika/headless/client` WebSocket uses HTTP Basic credentials containing the profile ID and an empty password; both Basic and cookie identities on that route must pass the managed-container checks. Basic authentication is not enabled for other game or API routes. Duplicate Authorization headers and conflicting session-cookie values are rejected. EFT's redundant, valid session cookies are accepted only when every value agrees, then collapsed into one canonical Cookie header used for both authentication and HTTP/WebSocket forwarding. Other supported, unambiguous cookie fields are preserved; unrelated cookie ambiguity remains rejected. Player authentication remains enabled, and disabled linked accounts stay blocked. An enabled database link does not prevent a live managed client from being recognized for loopback backend URL rewriting. Managed direct-backend URLs use the actual local SPT port; `proxy_rewrite_target_port` remains the externally advertised port for ordinary players. Fika configuration uses the merged view only when the overlay is confirmed mounted; stale files under an unmounted mountpoint are ignored.

SPT's assembly prepatcher runs before session initialization, so the exact read-only `GET /singleplayer/customEnumEntries` route is public bootstrap metadata. Static prestige artwork is also public only for GET requests to `/files/prestige/{id}.png` and `/files/prestige/reward/{small,big}/{id}.png`, with exactly 24 hexadecimal characters in the filename. Other methods, arbitrary files, directory listings, and game/profile routes remain authenticated.

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

Full profile backups and profile reads use the visible runtime/mod/base union even when the server overlay is unmounted, preserving overrides, whiteouts, and opaque-directory semantics. Reads do not mount the server or materialize a copy of every profile. Symlinked profile directories (including symlinked ancestors inside an installation layer) are rejected explicitly rather than risking a partial backup; confined individual profile-file aliases remain readable. Profile writes use the writable merged view or runtime upper, rather than the immutable base, with relative and mount-table-escaped installation paths supported. Profile restoration uses a scoped writable profile directory: escaping destination aliases are rejected without overwriting their targets, while leaf aliases confined to that directory are retained. Registration discovery and raid-start snapshots use the same layered profile reader.

## Migration

Migrate from the legacy flat directory layout to the new structured layout:

```bash
quma migrate                   # migrate (interactive)
quma migrate --dry-run         # show migration plan without making changes
```

## Web UI

HTTP compression supports gzip and Brotli. The optional HTTP zstd codec is disabled; archive zstd support remains available through the patched archive dependency.

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

Config lives at `<quma_dir>/quartermaster.toml`. Supported `QUMA_*` environment overrides are listed below.

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
proxy_auth = true              # require a linked SPT session on protected proxy routes (default)
# proxy_rewrite_http_host = "play.example.com" # optional DNS hostname for player HTTPS only
tls_enabled = true             # default — generates self-signed cert if no cert/key provided
tls_cert = "/path/to/cert.pem" # optional — use your own TLS cert
tls_key = "/path/to/key.pem"   # optional
```

Set `proxy_rewrite_http_host` in TOML when the player HTTPS hostname differs from the host used for direct TCP. Supply a DNS hostname or IPv4 address, without a scheme, port, or path. If unset, HTTPS rewrites still use the hostname from `external_url`. Direct TCP rewrites and managed headless loopback addresses are unchanged.

Set `proxy_auth = false` when clients should use native SPT/Fika authentication without an additional Quartermaster session check on the proxy. Anyone who can reach the proxy can then use the routes exposed by SPT/Fika, including stock launcher profile operations, subject to the upstream server's own checks. Quartermaster's dashboard sessions, permissions, and API token checks are unchanged. Managed headless clients still receive loopback URLs.

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

Without a `[convoy]` section, or with `enabled = false`, the Convoy catalog and archive endpoints return HTTP 503. This setting does not disable the player setup ZIP.

Delivery uses protocol 3 (`/quma/convoy/v3/catalog` and generation-scoped raw objects with `xxh3_128` plus length). Upgrade the standalone client and publisher together; enabled v2 routes return `410 Gone`. Legacy client assets prevent publication until explicitly rebaselined; **Rehash and republish** cannot bypass pending legacy baselines. Retained old and candidate objects both count toward the existing storage cap. See [the file/ownership policy](docs/convoy-client-files.md).

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

Controls the full client setup archive at `/quma/setup/mods.zip`, including its cached and startup-fallback responses. The invite-gated `/quma/join/mods.zip` bootstrap archive instead selects designated bootstrap mods (Convoy). Both read runtime upper, installed mod overlay, then base installation in priority order, respecting whiteouts and opaque directories even without a running server or mounted overlay. Configured filters and force-includes remain unchanged for the full setup archive; do not force-include private server configuration in that public download:

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
