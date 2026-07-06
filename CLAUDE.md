# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Quartermaster (`quma`) is a Rust CLI + web UI tool for managing server-side mods on an SPT/Fika dedicated server. It installs, updates, and removes mods from [SPT Forge](https://forge.sp-tarkov.com), with a web dashboard for server hosts and connected players. Linux-only for v1; SPT server runs in a Podman container.

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
just audit          # cargo audit
just changelog      # git-cliff changelog generation
just changelog-preview  # preview unreleased changes only
just release-dry-run    # dist build (dry run)
```

**Additional recipes:**

```bash
just build-headless     # build the headless client container image
just dev-install-tools  # install dev tools (cargo-watch for auto-reload)
just check-logging      # validate logging conventions via scripts/check-logging.sh
just cpd                # jscpd copy-paste detection
just install-hooks      # set up git hooks for local CI linting
```

**Local dev environment** (SPT dev environment at `.dev-server/`, bootstrapped via `quma setup`):

```bash
just dev-init       # bootstrap SPT dev environment at .dev-server/ via quma setup
just dev-serve      # build & run web UI against .dev-server/
just dev-cli <ARGS> # run any quma command against .dev-server/
just dev-watch      # auto-rebuild & restart dev server on file changes (needs cargo-watch)
just dev-seed       # seed dev database with test data (wipes & repopulates)
just dev-reset-db   # wipe .dev-server/ database (keeps config & structure)
just dev-clean      # remove .dev-server/ and container entirely
just dev-info       # show dev environment settings (port, container, worktree)
```

**Worktree-safe parallel dev environments**: The `dev-*` recipes auto-detect git worktrees and derive unique port/container names so multiple agents can work in parallel without conflicts:
- **Main repo**: port 9190, container `spt-server-dev`
- **Worktrees**: deterministic port 9191-9289, container `spt-server-<worktree-name>`
- Override with `QUMA_DEV_PORT` / `QUMA_DEV_CONTAINER` env vars if needed
- `.dev-server/`, `target/`, and database files are relative paths — already isolated per worktree
- Run `just dev-info` to check the current settings

Run a single test: `cargo test <test_name>` or `cargo test -p quartermaster <test_name>`

**Environment for testing**: Set `QUMA_SPT_DIR=~/spt-server` to point at the local SPT install. The database lives at `<spt_dir>/quartermaster.db`, config at `<spt_dir>/quartermaster.toml`.

## Architecture

Single Rust binary — the CLI and actix-web server share the same codebase. The web server is just the `serve` subcommand.

### Core Layers

- **`src/cli/`** — One file per CLI subcommand (clap derive). Each command's `run()` function is the entry point. `common.rs` holds `CliContext` (spt_dir, config, db, forge client) and shared helpers like `resolve_mod()` for resolving user input to Forge mod IDs.
- **`src/web/`** — actix-web server. `mod.rs` defines all routes and middleware wiring. `state.rs` defines `AppState` (shared via `web::Data`). Handlers live in `web/handlers/` (one file per page group: admin, auth, backup, clients, common, dashboard, join, logs, metrics, mods, modsync, profiles, queue, raids, requests, server, settings, setup, svm, tasks). `common.rs` has shared helpers (`ForgeSearchResult`, `forge_search`). Authentication uses `RequireAuth` middleware with RBAC permission checks per-handler via `require_permission(&user, Permission::X)`. Supporting modules: `sse.rs` (SSE broadcast), `flash.rs` (flash messages), `template_filters.rs` (Askama filters), `update_cache.rs` (Forge update cache), `raid_tracker.rs` (per-raid stats via proxy interception), `csrf.rs` (CSRF token protection), `nav.rs` (navigation helpers), `error.rs` (error rendering), `install.rs` (shared download/extract/record for mods+requests), `invite.rs` (invite code handling), `tasks.rs` (background task management), `mod_zip_cache.rs` (cached mod ZIP for join page).
- **`src/db/`** — SQLite via rusqlite (WAL mode, `busy_timeout=5000`). `schema.rs` runs migrations from `migrations/` directory; each migration is wrapped in a transaction (`unchecked_transaction`) that includes the version bump. `mods.rs` has mod CRUD, `addons.rs` has addon CRUD, `users.rs` has user/invite operations, `raids.rs` has raid and kill CRUD, `requests.rs` has mod request/voting operations, `backups.rs` has backup metadata CRUD, `rbac.rs` has role-based access control queries, `logs.rs` has log storage and querying for the SQLite log viewer. Database is wrapped in `Arc<parking_lot::Mutex<Database>>` for web access.
- **`src/forge/`** — HTTP client for SPT Forge API (`https://forge.sp-tarkov.com/api/v0`). `client.rs` is the reqwest-based client, `models.rs` defines API response types. Key quirk: `fika_compatibility` is a boolean on mod objects but a string enum on version objects.
- **`src/spt/`** — SPT directory interaction. `detect.rs` auto-detects SPT installs and reads version info from `core.json`. `mods.rs` handles archive extraction (ZIP/7z), file hashing, and mod file management. Both ZIP and 7z extraction reject symlink entries (tested via `zip_rejects_symlink` and `sevenz_rejects_symlink_entry`). `profiles.rs` reads SPT player profiles. `server.rs` handles SPT server HTTP communication (HTTPS with self-signed certs, zlib compression disabled via `responsecompressed: 0` header).
- **`src/ops.rs`** — Core mod operations: `install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id`. Both install and update extract to a `tempfile::tempdir()` staging directory before committing to the DB and moving files into place. Async updates use `apply_mod_update` with a `pending_updates` marker for crash recovery (`recover_pending_updates` runs on startup).
- **`src/backup.rs`** — Mod backup/restore system: per-mod and full snapshots of mod files, profiles, and config. Used by CLI `backup`/`restore` commands and web backup handler.
- **`src/health.rs`** — Health check system: server liveness, version verification, mod load verification, file integrity (SHA256).
- **`src/container.rs`** — Podman container management for SPT server lifecycle via bollard. Default SPT server image: `ghcr.io/zhliau/fika-spt-server-docker:latest`. Default headless client image: `localhost/fika-headless:latest` (configurable via `headless.image` in config).
- **`src/queue.rs`** — Change queue: mod operations are queued when SPT server is running, applied when stopped.
- **`src/server_detect.rs`** — Server running detection (Podman inspect or HTTP ping fallback).
- **`src/logging/`** — Structured logging with tracing. `mod.rs` has `LogBroadcast` (tokio broadcast + ring buffer), tracing subscriber setup, and per-layer target filtering. `compact.rs` is a custom compact console formatter. `writer.rs` is an async SQLite log writer for the log viewer. Supports console, file (with rotation), SQLite persistence, and web broadcast (SSE). Web log viewer caps DOM at 2000 entries with `trimOldEntries()` and disconnects SSE on hidden tabs.
- **`src/config.rs`** — Config types (serde TOML), env var overrides (`QUMA_*` prefix), and config resolution logic.
- **`src/modsync.rs`** — NarcoNet integration: regenerates `config.yaml` from installed mod state so clients auto-sync.
- **`src/headless_sync.rs`** — Syncs Fika-managed files (`BepInEx/plugins/Fika/`, `Fika.Headless/`) to headless client overlays.
- **`src/numa.rs`** — NUMA topology detection for container CPU pinning.
- **`src/tls.rs`** — TLS certificate loading/generation for the HTTPS proxy.
- **`src/invite.rs`** — Invite code generation and expiry parsing.
- **`src/client/`** — Fika headless client management. `supervisor.rs` runs the convergence loop, `converge.rs` handles container creation/scaling/overlay setup. Exit watchers cache restart policy/backoff values at spawn time (config changes to those require supervisor restart).
- **`src/spt/headless.rs`** — SPT server API types for headless client queries.
- **`src/spt/game_data.rs`** — Loads quest/trader/hideout metadata from SPT data files for profile display.
- **`src/svm/`** — Server Value Modifier (SVM) support. `metadata.rs` defines SVM categories and parameter metadata, `config.rs` handles reading/writing SVM config files.

### Web UI Stack

- **Templates**: Askama (compile-time checked) in `templates/`. Base layout in `base.html`, page templates extend it. Partials in `templates/partials/` and `templates/mods/partials/` for HTMX swap targets.
- **Frontend**: HTMX for interactivity (no JS build step). SSE for real-time updates (task progress, log streaming). Static assets (CSS, htmx.min.js, sse.js) embedded via rust-embed from `src/assets/`.
- **Sessions**: Signed cookies via actix-session (`CookieSessionStore`), 7-day TTL, SameSite=Strict, HttpOnly.
- **Rate limiting**: actix-governor on `/login` POST and `/register` (5 req/min/IP).
- **CSRF**: Token-based protection in `web/csrf.rs`.
- **HTTPS/WSS Proxy**: `web/proxy.rs` and `web/proxy_ws.rs` provide a transparent reverse proxy to the SPT server, letting clients connect through Quartermaster. `web/proxy_metrics.rs` tracks request counts and latencies.

### Key Patterns

- **CLI context resolution**: Most commands go through `cli::common::resolve_context()` which detects the SPT dir, loads config with env overrides, opens the DB, and creates the Forge client.
- **Web async DB access**: Database calls in web handlers use `web::block(move || { ... })` since rusqlite is synchronous. The DB is behind `Arc<parking_lot::Mutex<Database>>`.
- **Mod resolution**: Users can reference mods by name, slug, or numeric Forge ID. `common::resolve_mod()` handles disambiguation.
- **Archive extraction**: `spt::mods::extract_mod()` inspects archive directory structure to determine mod type (server mod → `SPT/user/mods/`, client mod → `BepInEx/plugins/`, hybrid → both).

### Database Migrations

SQL files in `migrations/` are numbered sequentially (001, 002, ...). The `schema::run_migrations()` function applies them in order, tracking applied migrations in a `schema_migrations` table.

## SPT Server Communication

The SPT server runs HTTPS on port 6969 (default) with a self-signed TLS certificate. Key endpoints:
- `GET /launcher/ping` → `"pong!"` (liveness)
- `GET /launcher/server/version` → version string
- `GET /launcher/server/loadedServerMods` → map of loaded mod metadata

Send `responsecompressed: 0` header to get raw JSON instead of zlib-compressed responses. TLS verification is disabled (self-signed cert).

## Git Workflow

**Always use a worktree for changes.** Never commit directly to main. Use `EnterWorktree` to create an isolated worktree before making any code changes — this keeps main clean and enables parallel work across multiple agents. Use relative paths inside worktrees (never hardcode absolute paths from the main repo). The `dev-*` recipes are worktree-aware and will auto-derive unique ports and container names per worktree.

## Autonomy — Subagent-Driven Development

When executing the `superpowers:subagent-driven-development` skill (or any SDD workflow), operate fully autonomously without asking for confirmation. This includes:
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

## AI Disclosure

This project uses LLM-based tools (Claude Code) for implementation assistance. All architecture, design, and technical direction are human-driven — the LLM operates as an implementation aid under continuous supervision and review. This CLAUDE.md file itself is how the LLM receives project context and conventions.
