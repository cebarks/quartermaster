# AGENTS.md

This file provides guidance to AI coding agents (Pi, Claude Code, Codex CLI, GitHub Copilot, Cursor, and similar) when working with code in this repository. It follows the tool-neutral `AGENTS.md` standard (https://agents.md/); Claude Code sessions reach it through the `CLAUDE.md` import shim in this directory.

## Project Overview

Quartermaster (`quma`) is a Rust CLI + web UI tool for managing server-side mods on an SPT/Fika dedicated server. It installs, updates, and removes mods from [SP Mods](https://sp-mod.com) — the successor to SPT Forge — with a web dashboard for server hosts and connected players. Linux-only for v1; SPT server runs in a Podman container.

**Binary name**: `quma`

## Build & Development Commands

```bash
just build          # cargo build
just check          # cargo check
just test           # cargo test
just clippy         # cargo clippy -- -D warnings
just fmt            # cargo fmt
just lint           # just fmt + just clippy + just check-logging + just cpd
just run <ARGS>     # cargo run -- <ARGS>
just serve          # cargo run -- serve (starts web UI on 0.0.0.0:9190)
just audit          # dependency audit gate: scripts/test_audit.py + scripts/audit.py (needs Python 3 and cargo-audit 0.22.2)
just changelog      # git-cliff changelog generation
just changelog-preview  # preview unreleased changes only
just release-dry-run    # dist build (dry run)
```

**Additional recipes:**

```bash
just dev-install-tools  # install dev tools (cargo-watch for auto-reload)
just check-logging      # validate logging conventions via scripts/check-logging.sh
just cpd                # jscpd copy-paste detection
just install-hooks      # set up git hooks for local CI linting
just build-spt-server   # build the SPT server container image
just build-headless     # build the headless client container image
just sync-svm-metadata  # check SVM metadata coverage against upstream C# models
```

**Local dev environment** (SPT dev environment at `.dev-server/`, bootstrapped via `quma setup`):

```bash
just dev-check       # verify prerequisites (podman, fuse-overlayfs, fusermount3, cargo, sqlite3)
just dev-init        # bootstrap SPT dev environment at .dev-server/ via quma setup (includes Fika)
just dev-serve       # build & run web UI against .dev-server/
just dev-cli <ARGS>  # run any quma command against .dev-server/
just dev-watch       # auto-rebuild & restart dev server on file changes (needs cargo-watch)
just dev-seed        # seed dev database with test data (wipes & repopulates)
just dev-reset-db    # wipe .dev-server/ database (keeps config & structure)
just dev-clean       # remove .dev-server/, container, and any headless containers
just dev-headless-init [PATH]  # configure headless client (default: ~/Games/SPTarkov/)
just dev-headless-clean        # stop headless containers and remove overlay data
just dev-info        # show dev environment settings (port, container, overlay, headless)
```

**Worktree-safe parallel dev environments**: The `dev-*` recipes auto-detect git worktrees and derive unique port/container names so multiple agents can work in parallel without conflicts:
- **Main repo**: port 9190, container `spt-server-dev`
- **Worktrees**: deterministic port 9191-9289, container `spt-server-<worktree-name>`
- Override with `QUMA_DEV_PORT` / `QUMA_DEV_CONTAINER` env vars if needed
- `.dev-server/`, `target/`, and database files are relative paths — already isolated per worktree
- Run `just dev-info` to check the current settings

Run a single test: `cargo test <test_name>` or `cargo test -p spt-quartermaster <test_name>`

**Environment for testing**: Set `QUMA_DIR=~/spt-server` (or the legacy `QUMA_SPT_DIR`) to point at the local quma install. The database lives at `<quma_dir>/quartermaster.db`, config at `<quma_dir>/quartermaster.toml`.

## Architecture

Single Rust binary — the CLI and actix-web server share the same codebase. The web server is just the `serve` subcommand.

### Core Layers

- **`src/cli/`** — One file per CLI subcommand (clap derive). Each command's `run()` function is the entry point. `common.rs` holds `CliContext` (`dirs: QumaDirs`, `spt_info`, config, db, forge client, optional container manager) and shared helpers like `resolve_mod()` for resolving user input to Forge mod IDs. `spt.rs` handles `quma spt {version, check, update}` for SPT server version management.
- **`src/web/`** — actix-web server. `mod.rs` defines all routes and middleware wiring. `state.rs` defines `AppState` (shared via `web::Data`). Handlers live in `web/handlers/` (one file per feature area: admin, auth, backup, clients, configs, convoy, dashboard, fika_settings, give_items, headless_api, join, logs, metrics, mods, notes, profiles, queue, raids, requests, server, settings, setup, svm, tasks). `handlers/common.rs` has shared helpers (`ForgeSearchResult`, `forge_search`). `auth.rs` has password hashing (argon2), session helpers, and the `RequireAuth` middleware with RBAC permission checks via `require_permission(&user, Permission::X)`. Supporting modules: `api_auth.rs` (API token auth — generates tokens to `<quma_dir>/spt-server/.quma/api-token`, constant-time comparison middleware), `sse.rs` (SSE broadcast), `flash.rs` (flash messages), `template_filters.rs` (Askama filters), `update_cache.rs` (Forge update cache), `raid_tracker.rs` (per-raid stats via proxy interception), `csrf.rs` (CSRF token protection), `nav.rs` (navigation helpers), `error.rs` (error rendering), `install.rs` (shared download/extract/record for mods+requests), `invite.rs` (invite code handling), `tasks.rs` (background task management), `mod_zip_cache.rs` (cached mod ZIP for join page), `integrity_cache.rs` (cached file integrity check results), `scanner_guard.rs` (IP-based rate-limit/ban for unhandled proxy requests).
- **`src/db/`** — SQLite via rusqlite (WAL mode, `busy_timeout=5000`). `schema.rs` holds the ordered `MIGRATIONS` list of `include_str!` scripts from the `migrations/` directory; each applied migration runs in a transaction (`unchecked_transaction`) that also bumps the SQLite `user_version` pragma. `mods.rs` has mod CRUD, `addons.rs` has addon CRUD, `users.rs` has user/invite operations, `raids.rs` has raid and kill CRUD, `requests.rs` has mod request/voting operations, `backups.rs` has backup metadata CRUD, `rbac.rs` has role-based access control queries, `logs.rs` has log storage and querying for the SQLite log viewer, `notes.rs` has admin/shared notes CRUD, `headless_stats.rs` has headless client session stats, `convoy.rs` has convoy catalog CRUD, `tests.rs` has shared test helpers. Database is wrapped in `Arc<parking_lot::Mutex<Database>>` for web access.
- **`src/forge/`** — HTTP client for the SPT Forge successor API (`https://sp-mod.com/api/v0`). `client.rs` is the reqwest-based client, `models.rs` defines API response types, `cache.rs` provides an in-memory response cache (mini-moka), `rate_limit.rs` spaces out requests. Key quirk: `fika_compatibility` is a boolean on mod objects but a string enum on version objects.
- **`src/fika/`** — Fika integration. `client.rs` is the HTTP client for the Fika API (items, send items, headless management). `config.rs` reads/writes fika.jsonc CST (preserving comments). `stats.rs` defines headless session statistics types.
- **`src/spt/`** — SPT directory interaction. `detect.rs` auto-detects SPT installs and reads the SPT version from `SPT.Server.deps.json` and the EFT version from `SPT_Data/configs/core.json`. `layout.rs` resolves the server runtime directory — `SPT_Runtime/` for current installs, `SPT/` for legacy ones — and rejects installations where both trees carry server markers. `mods.rs` handles archive extraction (ZIP/7z), file hashing (xxHash3-64), and mod file management. Archive-provided symlink entries are rejected in both formats (tested via `zip_rejects_symlink` and `sevenz_rejects_symlink_entry`), while existing links confined to the target root are preserved. `scoped.rs` is the `cap_std` directory-scoped I/O layer used by release extraction, convoy downloads, and mod-ZIP exports so archive/ZIP names cannot escape their permitted root. `profiles.rs` reads SPT player profiles. `server.rs` handles SPT server HTTP communication (HTTPS with self-signed certs, zlib compression disabled via `responsecompressed: 0` header). `releases.rs` fetches SPT server releases from the `SP-Tushonka/build` GitHub repo, downloads `.7z` archives, and extracts them.
- **`src/ops.rs`** — Core mod operations: `install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id` plus the addon equivalents. Install and update extract into a staging tempdir created inside the Quartermaster data directory (`staging_tempdir`) so the swap stays on one filesystem; `cleanup_staging` removes leftovers from a previous crash. Async updates use `apply_mod_update` / `apply_addon_update` with a `pending_updates` marker for crash recovery (`recover_pending_updates` runs on startup).
- **`src/backup.rs`** — Mod backup/restore system: per-mod and full snapshots of mod files, profiles, and config. Used by CLI `backup`/`restore` commands and web backup handler.
- **`src/health.rs`** — Health check system: server liveness, version verification, mod load verification, file integrity (xxHash3-64, cached through `web/integrity_cache.rs`).
- **`src/config_mgmt/`** — Configuration management with git-backed history. `git.rs` provides `ConfigHistoryRepo` for tracking mod config changes in a local git repo.
- **`src/container.rs`** — Podman container management for SPT server lifecycle via bollard. Default SPT server image: `ghcr.io/cebarks/quartermaster/spt-server:latest`. Default headless client image: `ghcr.io/cebarks/quartermaster/headless:latest` (configurable via `headless.image` in config).
- **`src/queue.rs`** — Change queue: mod operations are queued when SPT server is running, applied when stopped.
- **`src/server_detect.rs`** — Server running detection (Podman inspect or HTTP ping fallback).
- **`src/logging/`** — Structured logging with tracing. `mod.rs` has `LogBroadcast` (tokio broadcast + ring buffer), tracing subscriber setup, and per-layer target filtering. `compact.rs` is a custom compact console formatter. `writer.rs` is an async SQLite log writer for the log viewer. Supports console, file (with rotation), SQLite persistence, and web broadcast (SSE). Web log viewer caps DOM at 2000 entries with `trimOldEntries()` and disconnects SSE on hidden tabs.
- **`src/config.rs`** — Config types (serde TOML), env var overrides (`QUMA_*` prefix), and config resolution logic.
- **`src/convoy/`** — Convoy mod distribution (replaces deprecated modsync/NarcoNet). `catalog.rs` builds the mod catalog, `download.rs` handles mod file downloads, `migrate.rs` migrates from the old modsync config format.
- **`src/headless_sync.rs`** — Syncs Fika-managed files (`BepInEx/plugins/Fika/`, `Fika.Headless/`) to headless client overlays.
- **`src/numa.rs`** — NUMA topology detection for container CPU pinning.
- **`src/tls.rs`** — TLS certificate loading/generation for the HTTPS proxy.
- **`src/dirs.rs`** — `QumaDirs` struct: resolves the data layout (`spt-server/`, `headless/`, `overlays/`) with legacy flat-layout migration, and exposes layout-aware accessors (`server_runtime_dir()`, `server_mods_dir()`, profile/overlay paths) that follow the `SPT_Runtime` versus legacy `SPT` runtime tree.
- **`src/invite.rs`** — Invite code generation and expiry parsing.
- **`src/client/`** — Fika headless client management. `supervisor.rs` runs the convergence loop, `converge.rs` handles container creation/scaling/overlay setup. Exit watchers read restart policy/backoff values from config on each iteration, so config changes take effect without restarting the supervisor.
- **`src/headless/`** — Headless client service layer. `service.rs` defines `HeadlessService` (scaling, lifecycle actions, status). `operations.rs` provides `OperationTracker` for async operation tracking. `error.rs` defines `HeadlessError`.
- **`src/spt/headless.rs`** — SPT server API types for headless client queries.
- **`src/spt/game_data.rs`** — Loads quest/trader/hideout metadata from SPT data files for profile display.
- **`src/svm/`** — Server Value Modifier (SVM) support. `metadata.rs` defines SVM categories and parameter metadata, `config.rs` handles reading/writing SVM config files.

### Web UI Stack

- **Templates**: Askama (compile-time checked) in `templates/`. Base layout in `base.html`, page templates extend it. Partials in `templates/partials/` and `templates/mods/partials/` for HTMX swap targets.
- **Frontend**: HTMX for interactivity (no JS build step). SSE for real-time updates (task progress, log streaming). Static assets (CSS, htmx.min.js, sse.js, svm-form.js, svm-search.js) embedded via rust-embed from `src/assets/`.
- **Sessions**: Signed cookies via actix-session (`CookieSessionStore`), 7-day TTL, SameSite=Strict, HttpOnly.
- **Rate limiting**: actix-governor — global rate limit (5 req/12s burst) plus a separate search rate limit (10 req/6s burst).
- **CSRF**: Token-based protection in `web/csrf.rs`.
- **HTTPS/WSS Proxy**: `web/proxy.rs` and `web/proxy_ws.rs` provide a transparent reverse proxy to the SPT server, letting clients connect through Quartermaster. `web/proxy_metrics.rs` tracks request counts and latencies.

### Key Patterns

- **CLI context resolution**: Most commands go through `cli::common::resolve_context()` which detects the Quartermaster data directory (`QumaDirs::detect()`), loads config with env overrides, opens the DB, and creates the Forge client.
- **Web async DB access**: Database calls in web handlers use `web::block(move || { ... })` since rusqlite is synchronous. The DB is behind `Arc<parking_lot::Mutex<Database>>`.
- **Mod resolution**: Users can reference mods by name, slug, or numeric Forge ID. `common::resolve_mod()` handles disambiguation.
- **Archive extraction**: `spt::mods::detect_mod_type()` inspects archive entry paths to determine mod type (server mod → `SPT/user/mods/` or `SPT_Runtime/user/mods/`, client mod → `BepInEx/plugins/`, both → hybrid, neither → ambiguous). `extract_mod()` stages into a directory and `extract_mod_for_layout()` rewrites server paths into the target installation's runtime layout.
- **Runtime layout**: `spt::layout::detect_layout()` decides `SPT_Runtime` versus legacy `SPT` from the server markers; code that touches server files goes through `QumaDirs`/`layout` helpers instead of hardcoding `SPT/`. Ambiguous installs (markers in both trees) are rejected, as are in-place updates that would switch layouts — checked before the server is stopped or files are overwritten.

### Database Migrations

SQL files in `migrations/` are numbered sequentially (001, 002, ...) and applied in order by `schema::run_migrations()`, which steps the `user_version` pragma forward. A new file is only run once it is appended to the `MIGRATIONS` const in `src/db/schema.rs` — the filename list and the const must match.

## SPT Compatibility

Server releases are discovered from the `SP-Tushonka/build` GitHub repo rather than pinned (`spt::releases`); each release is paired with a required EFT build for game and headless clients, which `quma spt version` and `quma spt check` report. The README's SPT Compatibility section carries the current version/EFT combination and the upgrade guidance.

New installations keep upstream's `SPT_Runtime/` server directory; legacy `SPT/` directories are still supported. That runtime tree is independent of Quartermaster's own flat-versus-nested data layout (`src/dirs.rs`).

## SPT Server Communication

The SPT server runs HTTPS on port 6969 (default) with a self-signed TLS certificate and requires the **launcher v2 HTTP API** (`/launcher/v2/`); the legacy launcher endpoints are not supported. Key endpoints (responses wrap the payload in a `Response` field):
- `GET /launcher/v2/ping` → `"Pong!"` (liveness)
- `GET /launcher/v2/version` → version string
- `GET /launcher/v2/mods` → map of loaded mod metadata
- `POST /launcher/v2/register` → profile registration by username/edition, returns the profile ID

Send `responsecompressed: 0` header to get raw JSON instead of zlib-compressed responses. Quartermaster's client accepts the self-signed cert; container health probes verify it over HTTPS.

## Git Workflow

**Always use a worktree for changes.** Never commit directly to main. Create an isolated git worktree before making any code changes — with your agent's worktree tool or `git worktree add` directly — this keeps main clean and enables parallel work across multiple agents. Put worktrees outside the repository working tree (a sibling or scratch directory), never nested inside it: nested checkouts get picked up by file watchers, ignore rules, and agent context discovery that assume one tree per repo. Use relative paths inside worktrees (never hardcode absolute paths from the main repo). The `dev-*` recipes are worktree-aware and will auto-derive unique ports and container names per worktree.

## Agent Skills

Project skills use the Agent Skills layout in `.agents/skills/` — one `SKILL.md` per skill, discovered by walking up from the working directory to the repository root. The `dev-server` skill there drives this repository's dev environment. Add new project skills as `.agents/skills/<name>/SKILL.md`, not under a tool-specific directory.

## Autonomy — Subagent-Driven Development

When executing the `subagent-driven-development` skill (or any SDD workflow — some host tools namespace the same skill, e.g. `superpowers:subagent-driven-development`), operate fully autonomously without asking for confirmation. This includes:
- Creating and removing git worktrees
- All git operations (add, commit, push, checkout, branch, merge, rebase, reset, stash, cherry-pick)
- Running builds, tests, lints, and the binary
- Deleting or overwriting files as needed during implementation
- Force-pushing feature branches (never force-push main)
- Cleaning up worktrees and temporary branches after completion

Do not pause for confirmation on any of these during SDD execution. The review checkpoint built into the skill is sufficient oversight.

## Forge API Quirks

- `fika_compatibility` is a **boolean** on mod objects, but a **string enum** (`"compatible"`, `"incompatible"`, `"unknown"`) on version objects.
- `include=versions` on list endpoint returns abbreviated versions (last 6, no `link`/`content_length`/`fika_compatibility`).
- `include=versions` on single-mod endpoint returns full versions (last 10, all fields).
- Dedicated versions endpoint (`GET /mod/{id}/versions`) supports filtering and pagination.
- Dependency endpoints (`GET /mods/dependencies`, `GET /addons/dependencies`) require a concrete installed `spt_version`; omitting it returns HTTP 400 `VALIDATION_FAILED`. Current responses group dependency arrays by the requested `id:version` pair — `DependencyData` in `forge/client.rs` flattens the groups and still accepts the legacy flat array.

## AI Disclosure

This project uses LLM-based tools (Claude Code, and since 2026 Pi) for implementation assistance. All architecture, design, and technical direction are human-driven — the LLM operates as an implementation aid under continuous supervision and review. This AGENTS.md file itself is how the agent receives project context and conventions.
