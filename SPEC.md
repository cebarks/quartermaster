# Quartermaster (`quma`) — Technical Specification

## Overview

Quartermaster is a Rust CLI + web UI tool for managing server-side mods on an SPT/Fika dedicated server. It complements ModSync (which handles client-side file synchronization) by giving the server host a single tool to install, update, and remove mods from the [SPT Forge](https://forge.sp-tarkov.com), with a web dashboard for both the host and connected players.

**Binary name**: `quma`

---

## Architecture

Single Rust binary. The CLI and web server share the same codebase and can run concurrently (the web server is just another subcommand).

### Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| CLI framework | `clap` (derive) | Standard for Rust CLIs |
| HTTP server | `actix-web` | Built-in static files, cookies, sessions; `web::block()` for sync ops |
| Templating | `askama` + `askama_web` (feature `actix-web-4`) | Compile-time checked, type-safe templates with actix-web integration |
| Frontend interactivity | HTMX | Server-rendered HTML, no JS build step |
| Static asset embedding | `rust-embed` + `actix-web-rust-embed-responder` | Single binary deployment, served via actix responder with caching |
| HTTP client | `reqwest` | Async, TLS, cookie support |
| Database | `rusqlite` (SQLite, WAL mode) | Zero-config, single-file, concurrent reads; called via `web::block()` |
| Serialization | `serde` + `serde_json` | Standard |
| Password hashing | `argon2` | Memory-hard, recommended for passwords |
| CLI progress bars | `indicatif` | Download and install progress for multi-mod operations |
| Async runtime | `tokio` | Required by actix-web/reqwest |

### Directory Layout (project)

```
quartermaster/
├── Cargo.toml
├── SPEC.md
├── src/
│   ├── main.rs              # Entry point, clap CLI dispatch
│   ├── cli/                  # CLI command handlers
│   │   ├── mod.rs
│   │   ├── setup.rs
│   │   ├── init.rs
│   │   ├── install.rs
│   │   ├── update.rs
│   │   ├── remove.rs
│   │   ├── list.rs
│   │   ├── track.rs
│   │   ├── check.rs
│   │   ├── apply.rs
│   │   ├── status.rs
│   │   ├── server.rs
│   │   ├── serve.rs
│   │   ├── generate.rs
│   │   ├── invite.rs
│   │   └── config.rs
│   ├── forge/                # Forge API client
│   │   ├── mod.rs
│   │   ├── client.rs         # HTTP client, auth, pagination
│   │   ├── models.rs         # API response types
│   │   └── endpoints.rs      # Typed endpoint wrappers
│   ├── db/                   # SQLite database layer
│   │   ├── mod.rs
│   │   ├── schema.rs         # Table definitions, migrations
│   │   ├── mods.rs           # Mod CRUD operations
│   │   └── users.rs          # User/session operations
│   ├── spt/                  # SPT directory interaction
│   │   ├── mod.rs
│   │   ├── detect.rs         # Auto-detect SPT install
│   │   ├── profiles.rs       # Read user/profiles/ for auth
│   │   └── mods.rs           # Read/write to mod directories
│   ├── web/                  # Web UI (actix-web)
│   │   ├── mod.rs
│   │   ├── routes.rs         # Route definitions
│   │   ├── handlers/         # Request handlers
│   │   │   ├── mod.rs
│   │   │   ├── auth.rs
│   │   │   ├── dashboard.rs
│   │   │   └── mods.rs
│   │   ├── middleware.rs      # Auth middleware (actix-web middleware), session extraction
│   │   └── state.rs          # AppState via web::Data (db handle, forge client, config)
│   ├── templates/            # Askama HTML templates
│   │   ├── base.html
│   │   ├── login.html
│   │   ├── register.html
│   │   ├── dashboard.html
│   │   ├── mods/
│   │   │   ├── list.html
│   │   │   ├── detail.html
│   │   │   └── partials/     # HTMX swap targets
│   │   │       ├── dependency_tree.html
│   │   │       └── update_badges.html
│   │   ├── queue.html
│   │   └── status.html
│   ├── assets/               # CSS, icons (embedded via rust-embed)
│   │   ├── style.css
│   │   └── htmx.min.js
│   └── config.rs             # Configuration types, file I/O
├── migrations/               # SQL migration files
│   ├── 001_initial.sql
│   └── ...
└── tests/
    ├── forge_api.rs
    ├── install_flow.rs
    └── ...
```

---

## SPT Directory Detection

Quartermaster needs to know where the SPT server is installed. Detection strategy (in order):

1. **Explicit flag**: `quma --spt-dir /path/to/spt <command>`
2. **Environment variable**: `QUMA_SPT_DIR=/path/to/spt`
3. **Current directory**: Walk up from `cwd` looking for the SPT directory signature:
   - `SPT.Server.exe` in the directory
   - `SPT_Data/Server/configs/core.json` exists
   - `user/mods/` subdirectory exists
   - `BepInEx/plugins/` subdirectory exists

**Target: SPT 4.0+ only.** Pre-4.0 (Aki-era) installs are not supported. **Linux only for v1** (Windows support is post-v1). SPT server runs in a Podman container.

### Config File

Config file location: `<spt_root>/quartermaster.toml` (lives next to the SPT server by default).

Override the config file location via:
- Flag: `quma --config /path/to/quartermaster.toml <command>`
- Environment variable: `QUMA_CONFIG=/path/to/quartermaster.toml`

Priority: CLI flag > env var > default (`<spt_root>/quartermaster.toml`).

### SPT Directory Signature

A valid SPT 4.0+ install has:
```
<spt_root>/
├── SPT.Server.exe
├── SPT_Data/
│   └── Server/
│       └── configs/
│           └── core.json       # Contains sptVersion, compatibleTarkovVersion
├── user/
│   ├── mods/                   # Server-side mods go here
│   └── profiles/               # Player profile JSON files
├── BepInEx/
│   └── plugins/                # Client-side mods go here
```

### SPT Version Detection

Read `SPT_Data/Server/configs/core.json` and parse the `sptVersion` field. This is the canonical version source for SPT 4.0+. The file also contains `compatibleTarkovVersion` (the EFT client build this SPT targets), which is displayed in `quma status` and the web UI status page.

---

## Forge API Integration

Base URL: `https://forge.sp-tarkov.com/api/v0`

### Authentication

The Forge API supports optional authentication via Bearer token. Some endpoints may require it in the future. Quartermaster stores the token in config if the user provides one via `quma config set forge_token <token>`.

### Key Endpoints Used

| Endpoint | Used By | Purpose |
|----------|---------|---------|
| `GET /mods?query=<q>` | `install` | Resolve mod name to ID (disambiguation) |
| `GET /mod/{id}?include=versions` | `track`, `install` | Mod details with recent versions |
| `GET /mod/{id}/versions?filter[spt_version]=<v>` | `install` | Find compatible version for current SPT |
| `GET /mods/dependencies` | `install` | Resolve full dependency tree |
| `GET /mods/updates` | `check`, `update` | Check installed mods for available updates |

### Mod Resolution

When the user specifies a mod by name (not ID), Quartermaster resolves it:

1. Search Forge with `query=<name>`
2. If exactly one result, use it
3. If multiple results, show a disambiguation list
4. If zero results, report not found

Mods can also be specified by Forge mod ID (numeric) or slug (URL-safe name).

### API Type Notes

- **`fika_compatibility`**: On mod objects this is a **boolean**. On version objects it's a **string enum** (`"compatible"`, `"incompatible"`, `"unknown"`). The Forge client models must handle both representations — use a unified `FikaCompat` enum internally and convert from either source.
- **Version includes differ by endpoint**:
  - `include=versions` on `GET /mods` (list): returns last 6 versions, **abbreviated** (no `link`, `content_length`, or `fika_compatibility`).
  - `include=versions` on `GET /mod/{id}` (single): returns last 10 versions, **full fields** including `link`, `content_length`, `fika_compatibility`. No filtering or nested includes.
  - `GET /mod/{id}/versions` (dedicated): paginated (max 50/page), full fields, supports filtering (SPT version, Fika compat, date ranges) and nested includes (`dependencies`, `virus_total_links`).

---

## Database Schema (SQLite)

All connections set `PRAGMA busy_timeout = 5000` to handle concurrent access from the CLI and web server without immediate `SQLITE_BUSY` errors.

```sql
-- Installed mods tracking
CREATE TABLE installed_mods (
    id INTEGER PRIMARY KEY,
    forge_mod_id INTEGER NOT NULL,
    forge_version_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    slug TEXT,
    version TEXT NOT NULL,           -- SemVer string
    installed_at TEXT NOT NULL,      -- ISO 8601
    updated_at TEXT,
    UNIQUE(forge_mod_id)
);

-- Individual files installed by each mod
CREATE TABLE installed_files (
    id INTEGER PRIMARY KEY,
    mod_id INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,         -- Relative to SPT root (e.g. 'user/mods/SAIN/SAIN.dll')
    file_hash TEXT,                  -- SHA256 for integrity checks
    file_size INTEGER,
    UNIQUE(file_path)                -- Enforces no cross-mod file conflicts
);

-- Dependency relationships between installed mods
CREATE TABLE mod_dependencies (
    id INTEGER PRIMARY KEY,
    mod_id INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    depends_on_mod_id INTEGER NOT NULL REFERENCES installed_mods(id),
    version_constraint TEXT,         -- SemVer constraint from Forge
    UNIQUE(mod_id, depends_on_mod_id)
);

-- Web UI users
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    spt_profile_id TEXT NOT NULL,    -- Matches SPT profile AID
    password_hash TEXT,              -- Argon2 hash, NULL in trust mode
    role TEXT NOT NULL DEFAULT 'player',  -- 'admin' or 'player'
    created_at TEXT NOT NULL
);

-- Invite codes
CREATE TABLE invite_codes (
    id INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,       -- e.g. 'quma-a8f3x2'
    created_by INTEGER REFERENCES users(id),
    used_by INTEGER REFERENCES users(id),
    created_at TEXT NOT NULL,
    used_at TEXT,
    expires_at TEXT                  -- Optional expiry
);

-- Pending mod operations (queued while SPT server is running)
CREATE TABLE pending_operations (
    id INTEGER PRIMARY KEY,
    action TEXT NOT NULL,            -- 'install', 'update', 'remove'
    forge_mod_id INTEGER NOT NULL,
    forge_version_id INTEGER,       -- NULL for removals
    mod_name TEXT NOT NULL,
    metadata TEXT,                   -- JSON blob: resolved dependency tree (install), target version info (update), etc.
    queued_at TEXT NOT NULL,         -- ISO 8601
    queued_by TEXT                   -- Username if from web UI, NULL if from CLI
);

```

Database file location: `<spt_root>/quartermaster.db`

---

## CLI Commands

### `quma setup`

Interactive guided setup for configuring an existing SPT install for Fika multiplayer. See "Initial Fika Setup" section for full details.

### `quma init [path]`

Initialize Quartermaster for an SPT server.

- If `path` is provided, use it as the SPT directory
- Otherwise, auto-detect from cwd (see SPT Directory Detection)
- Validate SPT 4.0+ directory structure
- Read SPT version from `SPT_Data/Server/configs/core.json` (`sptVersion` field)
- Create config file at `<spt_root>/quartermaster.toml`
- Create SQLite database
- Scan `user/mods/` and `BepInEx/plugins/` for pre-existing mods and attempt to match them to Forge entries (best-effort — unmatched mods are listed as "unmanaged")
- If no admin user exists, prompt to create one (select SPT profile, set password)

### `quma install <mod> [version]`

Install a mod and its dependencies.

1. Resolve `<mod>` to Forge mod ID
2. If `[version]` not specified, pick latest version compatible with current SPT version
3. Fetch full version details via `GET /mod/{id}/versions` — check `fika_compatibility` field on the selected version. If `"incompatible"`, warn the user and require `--force` or explicit confirmation to proceed. If `"unknown"`, note it but don't block.
4. Call `GET /mods/dependencies` with the mod:version pair to get full dependency tree
5. Display dependency tree to user, showing what will be installed (and what's already installed), with Fika compat status per mod
6. Prompt for confirmation
7. If SPT server is running and `--force` not passed, queue the operation (see Change Queue) and stop here
8. Download each mod (version record has `link` and `content_length` fields)
9. Extract to correct directory based on mod type:
   - Server mods → `user/mods/<mod_name>/`
   - Client mods → `BepInEx/plugins/<mod_name>/`
10. Record in SQLite `installed_mods`, `installed_files` (every extracted file with path, hash, size), and `mod_dependencies`

**Mod type detection**: SPT 4.0+ uses C# DLLs for both server and client mods — the only differentiator is the install directory. Detection is based on archive directory structure (see Archive Handling section).

### `quma update [mod]`

Update mods.

- If `<mod>` specified: update that specific mod
- If no argument: update all installed mods
- Uses `GET /mods/updates` with installed mod:version pairs and current SPT version
- Response categorizes mods as: `updated` (safe to update), `blocked` (dependency conflict), `up_to_date`, `incompatible`
- Show what will be updated, prompt for confirmation
- If SPT server is running and `--force` not passed, queue the operation (see Change Queue) and stop here
- Download, extract, replace old files
- Update SQLite records

### `quma remove <mod>`

Remove an installed mod.

1. Resolve `<mod>` to installed mod
2. Check reverse dependencies — if other installed mods depend on this one, warn and list them
3. Offer options: remove just this mod (may break dependents), remove this mod and all dependents, cancel
4. If SPT server is running and `--force` not passed, queue the operation (see Change Queue) and stop here
5. Delete all files tracked in `installed_files` for this mod; remove empty parent directories
6. Remove from SQLite `installed_files`, `installed_mods`, and `mod_dependencies`

### `quma list`

List installed mods.

- Output: table — name, installed version, latest available version, file count, install date
- Flag: `--json` for machine-readable output
- Shows update availability indicator (checkmark, arrow, or warning)

### `quma track <path> <forge_mod_id>`

Associate an unmanaged mod with a Forge entry so Quartermaster can manage it.

- `<path>` — relative path from SPT root to the mod directory (e.g. `user/mods/SAIN`)
- `<forge_mod_id>` — Forge mod ID or slug
- Fetches mod info from Forge to determine current version
- Scans the directory and records all files in `installed_files`
- Creates an `installed_mods` entry

### `quma check`

Check all installed mods for updates.

- Uses `GET /mods/updates`
- Output: categorized list — available updates, blocked updates (with reason), up-to-date mods, incompatible mods
- Exit code: 0 if all up-to-date, 1 if updates available

### `quma apply`

Apply pending queued operations.

- Drains `pending_operations` table, applying each in order
- If SPT server is running, refuses unless `--force` is passed
- Shows a summary of operations being applied
- Each applied operation is removed from `pending_operations`

### `quma status`

Run health checks against the SPT server and local mod integrity. See "Health Checks" section for full details.

- Flag: `--json` for machine-readable output
- Exit codes: 0 (all pass), 1 (server down), 2 (mod issues)

### `quma server start|stop|restart|logs`

Manage the SPT server Podman container. See "Server Lifecycle (Podman)" section for full details.

- `start` — start container, wait for ping (drains queue if `auto_drain_on_lifecycle` enabled)
- `stop` — graceful shutdown
- `restart` — stop → start (drains queue between if `auto_drain_on_lifecycle` enabled; `--drain`/`--skip-queue` to override)
- `logs [--follow]` — tail container logs

### `quma serve`

Start the Quartermaster web UI server.

- Default bind: `0.0.0.0:9190`
- Flag: `--bind <addr>`, `--port <port>`
- If no admin user exists, prompt to create one first
- Runs until interrupted (Ctrl+C)

### `quma generate systemd`

Emit a systemd service file for `quma serve`.

- Outputs to stdout by default (pipe to file: `quma generate systemd > /etc/systemd/system/quartermaster.service`)
- Flag: `--install` to write directly to `/etc/systemd/system/` and enable the service (requires root)
- Includes: working directory, ExecStart path, restart policy (`on-failure`), `After=network.target`
- Uses the current config (bind address, port, SPT dir) to populate the service file

### `quma invite`

Generate an invite code for a player.

- Generates a code like `quma-a8f3x2` (prefix + 6 random alphanumeric chars)
- Flag: `--expires <duration>` (e.g. `24h`, `7d`; default: no expiry)
- Stores in SQLite `invite_codes`
- Prints the code and the registration URL (`http://<host>:<port>/register?code=<code>`)

### `quma config`

View and modify configuration.

- `quma config` — show current config
- `quma config set <key> <value>` — set a config value
- `quma config get <key>` — get a config value

Config keys:
| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `spt_dir` | path | inferred from config location | Explicit SPT server directory (optional) |
| `forge_token` | string | none | Forge API auth token (optional) |
| `web_port` | u16 | 9190 | Web UI port |
| `web_bind` | string | `0.0.0.0` | Web UI bind address |
| `queue_changes` | bool | `true` | Queue mod operations when SPT server is running |
| `auto_drain_on_lifecycle` | bool | `false` | Auto-drain pending queue on `server start/stop/restart` |
| `server_container` | string | none | Podman container name/ID for SPT server |
| `server_host` | string | from `http.json` or `127.0.0.1` | SPT server host for health checks |
| `server_port` | u16 | from `http.json` or `6969` | SPT server port for health checks |
| `session_secret` | string | auto-generated | Secret key for signed session cookies |

Config file: `<spt_root>/quartermaster.toml` (override with `--config` or `QUMA_CONFIG`)

---

## Web UI

### Routes

```
GET  /                        # Dashboard
GET  /login                   # Login page
POST /login                   # Authenticate
GET  /register?code=<code>    # Registration page (requires invite code)
POST /register                # Create account
POST /logout                  # Log out

# Mod management (admin only)
GET  /mods                    # Installed mods list
POST /mods/install            # Install a mod
POST /mods/:id/update         # Update a specific mod
POST /mods/:id/remove         # Remove a mod
POST /mods/update-all         # Update all mods

# Server lifecycle (admin only)
POST /server/start            # Start SPT server container
POST /server/stop             # Stop SPT server container
POST /server/restart          # Restart (stop → drain queue → start)

# Queue management (admin only)
POST /queue/:id/cancel        # Cancel a pending operation
POST /queue/apply             # Apply all pending operations

# Mod details (all authenticated users)
GET  /mods/:id                # Mod detail page

# Player-accessible
GET  /status                  # Server health, mod integrity, version info (auto-refreshing)
GET  /queue                   # Pending queued operations (if any)

# HTMX API (returns HTML partials)
GET  /api/mods/check-updates  # Update badges partial
GET  /api/mods/dep-tree?mod=&ver=  # Dependency tree partial
GET  /api/status              # Health check partial (polled by /status page)
```

### Authentication Flow

**Auth flow**:
1. Admin runs `quma invite`, gets code like `quma-a8f3x2`
2. Admin shares code with player
3. Player visits `/register?code=quma-a8f3x2`
4. Registration page shows a dropdown of SPT profiles (read from `user/profiles/`)
5. Player selects their profile, sets a password
6. Account created, player redirected to login
7. Login with SPT username + password
8. Session stored as a signed cookie via actix-web's session middleware (`actix-session` with `CookieSessionStore`; signed, `SameSite=Strict`, `HttpOnly`; secret key persisted in config)

**Rate limiting**: `/login` and `/register` are rate-limited to 5 requests per minute per IP via `actix-governor`. Failed invite code attempts also count against this limit.

**Admin setup**:
- First run of `quma serve` (or during `quma init`) detects no admin exists
- Prompts to select an SPT profile and set a password
- That user becomes the admin

### Roles and Permissions

| Action | Admin | Player |
|--------|-------|--------|
| View dashboard | Yes | Yes |
| View mod details | Yes | Yes |
| View server status | Yes | Yes |
| Install mods | Yes | No |
| Update mods | Yes | No |
| Remove mods | Yes | No |
| Generate invite codes | Yes | No |
| Start/stop/restart server | Yes | No |
| View pending queue | Yes | Yes |
| Apply/cancel queued operations | Yes | No |

### HTMX Interactions

- **Install flow**: Admin provides a Forge mod ID/slug, `hx-get="/api/mods/dep-tree?mod=<id>&ver=<ver>"` shows the dependency tree in a modal/panel. Confirm button POSTs the install.
- **Update badges**: Dashboard periodically polls `hx-get="/api/mods/check-updates" hx-trigger="every 60s"` to show update availability

---

## Mod Installation Details

### Archive Handling

Forge mod downloads are ZIP archives. Extraction logic:

1. Download ZIP to a temp directory
2. Inspect archive directory structure to determine mod type and extraction target:
   - Archive contains paths starting with `user/mods/` → **server mod** → extract relative to SPT root
   - Archive contains paths starting with `BepInEx/plugins/` → **client mod** → extract relative to SPT root
   - Archive contains both `user/` and `BepInEx/` paths → **hybrid mod** → extract relative to SPT root, preserving directory structure
   - Archive is a flat directory with DLLs and no path hints → **ambiguous** → prompt user to confirm target directory (`user/mods/` or `BepInEx/plugins/`)
   - Archive may contain a top-level directory matching the mod name — strip it if so (common pattern)
   
   Note: SPT 4.0+ uses C# DLLs for both server and client mods. There is no file-type distinction — only the install directory matters.
3. Verify extraction succeeded
4. Clean up temp files

### Update Strategy

Updates replace mod files in-place:
1. Download new version to temp
2. Delete all files tracked in `installed_files` for this mod; remove empty parent directories
3. Extract new version
4. Update `installed_mods`, replace `installed_files` entries with new file set (paths, hashes, sizes)

No rollback mechanism in v1. If something goes wrong, the user reinstalls.

### Unmanaged Mods

Mods installed manually (not via Quartermaster) are detected during `quma init` and `quma list`:
- Scan `user/mods/` and `BepInEx/plugins/` for files/directories not tracked in the database
- Show them as "unmanaged" in the mod list and web UI dashboard
- User can manually associate an unmanaged mod with a Forge entry via `quma track <path> <forge_mod_id>` — this adds the mod to `installed_mods` and `installed_files` so Quartermaster can manage updates and removal going forward

---

## Change Queue

Mod operations (`install`, `update`, `remove`) check whether the SPT server is running before applying changes to disk. Detection uses Podman container state (see Server Lifecycle section).

### Behavior

- **Server stopped**: Operations apply immediately to disk.
- **Server running**: Operations are written to `pending_operations` instead of applied. User sees "Change queued — will apply when server is stopped."
- **`--force` flag**: Bypasses the queue and applies immediately even if the server is running. Prints a warning about potential instability.
- **`queue_changes = false` in config**: Disables the queue system entirely — all operations apply immediately regardless of server state (equivalent to always passing `--force`, without the warning).

### Draining the Queue

- **Automatic**: `quma update` drains pending operations before checking for new updates (server must be stopped). If `auto_drain_on_lifecycle = true` in config, `quma server start|stop|restart` also drains before proceeding.
- **Explicit**: `quma apply` drains the queue manually. Fails with a warning if the server is still running (unless `--force`).
- **Web UI**: Queued operations are visible on the dashboard. Admin can apply or cancel individual queued operations.


---

## Server Lifecycle (Podman)

Quartermaster manages the SPT server as a Podman container. The container name/ID is stored in `quartermaster.toml` as `server_container`, detected during `quma setup`, or set via `quma config set server_container <name>`.

### Commands

**`quma server start`** — `podman start <container>`
- If `auto_drain_on_lifecycle = true`, drains pending operations before starting
- Waits for `/launcher/ping` to respond (default 60s timeout, configurable via `--timeout <seconds>`), reports ready or failed

**`quma server stop`** — `podman stop <container>`
- Sends SIGTERM, waits for graceful shutdown

**`quma server restart`** — stop, optionally drain, start
- If `auto_drain_on_lifecycle = true`: stop → apply queued mod changes → start
- If disabled: stop → start (use `quma apply` separately to drain)
- Flag: `--drain` to force drain regardless of config, `--skip-queue` to skip regardless of config

**`quma server logs [--follow]`** — `podman logs --tail 100 [-f] <container>`

**`quma server status`** — alias for `quma status` (same health checks)

### Server Detection

For v1, "is the server running?" checks the Podman container state:
- `podman inspect --format '{{.State.Status}}' <container>` → `running`, `stopped`, `exited`, etc.
- This replaces the process-detection approach (no `SPT.Server.exe` process scanning needed on Linux)
- Falls back to `/launcher/ping` if no container is configured (e.g., user runs the server outside Podman)

### Integration with Change Queue

The change queue's server-running detection uses the Podman container state:
1. If `server_container` is configured: check `podman inspect` state
2. If not configured: attempt `/launcher/ping` on `server.host`:`server.port` (falling back to `http.json` values, then defaults `127.0.0.1:6969`)
3. If neither works: assume server is stopped (user's responsibility)

`quma server restart` is the recommended way to apply mod updates — it handles the full stop → drain → start cycle automatically.

---

## Configuration

### Config File

`<spt_root>/quartermaster.toml` (default, lives next to the SPT server):

```toml
spt_dir = ""               # Optional explicit SPT path; if empty, inferred from config file location
forge_token = ""           # Optional Forge API token
queue_changes = true       # Queue mod operations when SPT server is running
auto_drain_on_lifecycle = false  # Auto-drain queue on server start/stop/restart
session_secret = ""        # Auto-generated on first run, used for signed cookies
server_container = ""      # Podman container name or ID for the SPT server
server_host = ""           # SPT server host; if empty, read from http.json or default 127.0.0.1
server_port = 0            # SPT server port; if 0, read from http.json or default 6969
web_bind = "0.0.0.0"       # Web UI bind address
web_port = 9190            # Web UI port
```

If `spt_dir` is empty or absent, the SPT directory is inferred as the parent directory of the config file. If set, it overrides that inference — useful when the config file lives in a different location than the SPT server.

### Environment Variables

Config values can be overridden by environment variables prefixed with `QUMA_`:
- `QUMA_SPT_DIR` — SPT server directory (also determines default config file location)
- `QUMA_CONFIG` — Config file path override
- `QUMA_FORGE_TOKEN`
- `QUMA_WEB_PORT`
- `QUMA_WEB_BIND`
- `QUMA_SERVER_CONTAINER`
- `QUMA_SERVER_HOST`
- `QUMA_SERVER_PORT`

Priority: CLI flags > env vars > config file > defaults.

---

## Error Handling

- All user-facing errors use `anyhow` with context chains
- Forge API errors are mapped to human-readable messages (rate limit, not found, auth required, server error)
- Network errors suggest checking connectivity and retrying
- File system errors (permission denied, disk full) are reported with the affected path
- Database errors trigger a suggestion to run `quma init` if the DB is missing/corrupted

---

## Initial Fika Setup

`quma setup` automates configuring an existing SPT installation for Fika multiplayer. Assumes SPT 4.0+ is already installed (via the official SPT Installer or manually).

### Prerequisites

- A working SPT 4.0+ installation (server starts, can reach main menu in single-player)
- `.NET 9` runtime installed (required on host if running SPT natively; included in container images if using Podman)

### `quma setup`

Interactive guided setup:

1. **Detect/confirm SPT directory** — auto-detect or prompt for path
2. **Validate SPT install** — check directory signature, read `core.json` for SPT version
3. **Configure Podman container** — auto-detect SPT server containers via `podman ps -a` (look for containers with SPT-related images or mounts pointing to the detected SPT directory). If found, confirm with user. If not found, prompt for container name/ID. Store as `server_container` in config.
4. **Configure networking** — edit `SPT_Data/Server/configs/http.json`:
   - Set `ip` to `0.0.0.0` (bind all interfaces for LAN/remote play)
   - Confirm/change port (default 6969)
   - Warn about firewall: TCP 6969 inbound required
5. **Install Fika** — download and install Fika from the Forge (mod ID 2326, GUID `com.fika.core`):
   - Server component → `user/mods/fika-server/`
   - Client component → `BepInEx/plugins/Fika.Core.dll`
   - Resolve and install Fika's dependencies
6. **First boot** — if Podman container is configured, run `quma server start` to boot the server and generate `fika.jsonc`; otherwise instruct user to start the server manually. Wait for `/launcher/ping` to confirm it's up.
7. **Fika config** — after first boot, offer to configure key `fika.jsonc` settings interactively:
   - `friendlyFire` (default: true)
   - `forceSaveOnDeath` (default: true)
   - `sharedQuestProgression` (default: false)
8. **Network guidance** — print summary:
   - SPT server: TCP 6969
   - Fika P2P raids: UDP 25565 (needed by whoever hosts a raid)
   - Suggest UPnP or VPN as alternatives to port forwarding
9. **Run `quma init`** — create `quartermaster.toml`, set up database, create admin user

Flags:
- `--non-interactive` — accept all defaults, skip prompts (for scripted deployments)
- `--skip-fika` — set up networking only, don't install Fika (for single-player server management)

### What `quma setup` Does NOT Do

- Copy EFT game files (use the SPT Installer)
- Run the downgrade patcher (use the SPT Installer)
- Install SPT itself (download from `sp-tarkov/build` releases)
- Manage the EFT client installation
- Configure the headless Fika client (future scope)

---

## Health Checks

`quma status` checks SPT server health, mod integrity, and version consistency. Also powers the `/status` web UI page.

### SPT Server Communication

The SPT server runs HTTPS on port 6969 (default) with a self-signed TLS certificate. All responses are zlib-compressed by default — Quartermaster sends `responsecompressed: 0` header to get raw JSON. TLS verification is skipped (self-signed cert).

### Checks Performed

**Liveness** — `GET /launcher/ping`
- Expected: `"pong!"`
- Reports: up/down, response time in ms
- If down: report the error (connection refused → server not running, timeout → server hung)

**Version verification** — `GET /launcher/server/version`
- Compare server's reported version against `sptVersion` from `core.json`
- Flag mismatch (could indicate stale server process running old binary)

**Mod load verification** — `GET /launcher/server/loadedServerMods` (server-side mods only; client-side BepInEx plugins load on the game client and are not visible to this endpoint)
- Returns map of mod names → metadata for all server mods the SPT server actually loaded
- Compare against `installed_mods` database:
  - Installed but not loaded → mod failed to load (crash, missing dependency, incompatible)
  - Loaded but not tracked → unmanaged mod, offer to track it
- Report load failures with mod name for easy debugging

**DB/disk sync** — local check, no server needed
- For each file in `installed_files`: verify the file exists on disk and optionally check SHA256 hash
- Scan `user/mods/` and `BepInEx/plugins/` for files/directories not tracked in the database
- Detect cross-mod file conflicts (files overwritten by manual installs)
- Report: missing files, modified files (hash mismatch), orphaned files/directories (untracked)

**SPT version compatibility** — `GET /mods/updates` on Forge API
- Check all installed mods against the current SPT version
- Report any mods that are now incompatible (e.g., after an SPT update)

### CLI Output

```
$ quma status

SPT Server
  Status:     running (responded in 12ms)
  Version:    4.0.13 (matches core.json)
  EFT Build:  0.16.9-40087
  Address:    https://127.0.0.1:6969

Mods (14 installed, 14 loaded)
  All installed mods loaded successfully.
  2 updates available (run `quma check` for details)

Integrity (247 tracked files)
  All mod files present on disk, hashes match.
  3 untracked files in user/mods/some-manual-mod/
```

### Web UI `/status` Page

Displays the same information as `quma status`, auto-refreshing via HTMX polling. Available to all authenticated users (not admin-only).

### Exit Codes

- `0` — all checks pass
- `1` — server is down or unreachable
- `2` — mod issues detected (load failures, missing files, incompatible mods)

Useful for scripting: `quma status && echo "all good"` or monitoring cron jobs.

---

## Future Scope (Not in v1)

- **Mod search**: `quma search` CLI command and web UI search page with HTMX live search
- **Mod info**: `quma info <mod>` CLI command for detailed Forge mod information
- **Operation history**: `operation_history` table, `quma log` command, and web UI changelog page tracking all mod operations with timestamps, who performed them, and whether `--force` was used
- **Trust mode auth**: No-password auth mode where players just pick their SPT profile — matches SPT's own security model
- **Windows support**: Native Windows process management (no Podman requirement), Windows-specific paths and process detection
- **Player mod request/voting**: Players can suggest mods via the web UI, admin approves/rejects
- **Fika Dedicated Client Management**: Full management of headless clients (health checks, automatic restarts, resource usage)
- **ModSync integration**: 
    - Trigger ModSync push after mod changes
    - Manage ModSync `syncpaths` and `exclusions` config
- **Backup/restore**: Configurable auto-snapshot before risky operations (updates, removes) via `auto_backup = true`; configurable backup directory via `backup_dir`; `quma backup` / `quma restore` commands; snapshots include mod files, profiles, and configs
- **Mod profiles**: Save/load sets of mods for different playstyles
- **Multi-server**: Manage mods across multiple SPT instances
- **Raid statistics / leaderboard**: Parse SPT profile JSON for raid history, survival rate, K/D, stash value — fun leaderboard page in web UI
- **Discord integration**: Webhook notifications (server up/down, mod changes); later a full bot with slash commands (`/status`, `/mods`, `/restart`)
- **Mod configuration UI**: Web-based editor for mod config files (JSON/JSONC/CFG) — discover configs in installed mods, render editable forms or syntax-highlighted editor
- **Scheduled restarts**: Built-in cron-style scheduling for automatic server restarts (e.g., daily at 4am)
- **Server MOTD / rules page**: Admin-editable message displayed on dashboard for all players
- **REST API**: JSON API (`/api/v1/`) for external tooling, Discord bots, and monitoring integrations
- **Full SPT setup**: Download SPT release from GitHub, copy EFT files, run downgrade patcher — full zero-to-server automation
