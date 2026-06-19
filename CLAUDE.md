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
just lint           # just fmt + just clippy
just run <ARGS>     # cargo run -- <ARGS>
just serve          # cargo run -- serve (starts web UI on 0.0.0.0:9190)
```

Run a single test: `cargo test <test_name>` or `cargo test -p quartermaster <test_name>`

**Environment for testing**: Set `QUMA_SPT_DIR=~/spt-server` to point at the local SPT install. The database lives at `<spt_dir>/quartermaster.db`, config at `<spt_dir>/quartermaster.toml`.

## Architecture

Single Rust binary — the CLI and actix-web server share the same codebase. The web server is just the `serve` subcommand.

### Core Layers

- **`src/cli/`** — One file per CLI subcommand (clap derive). Each command's `run()` function is the entry point. `common.rs` holds `CliContext` (spt_dir, config, db, forge client) and shared helpers like `resolve_mod()` for resolving user input to Forge mod IDs.
- **`src/web/`** — actix-web server. `mod.rs` defines all routes and middleware wiring. `state.rs` defines `AppState` (shared via `web::Data`). Handlers live in `web/handlers/` (one file per page group: auth, dashboard, mods, queue, server, status, logs, tasks). Authentication uses `RequireAuth` middleware with admin checks per-handler via `require_admin()`.
- **`src/db/`** — SQLite via rusqlite (WAL mode, `busy_timeout=5000`). `schema.rs` runs migrations from `migrations/` directory. `mods.rs` has mod CRUD, `users.rs` has user/invite operations. Database is wrapped in `Arc<parking_lot::Mutex<Database>>` for web access.
- **`src/forge/`** — HTTP client for SPT Forge API (`https://forge.sp-tarkov.com/api/v0`). `client.rs` is the reqwest-based client, `models.rs` defines API response types. Key quirk: `fika_compatibility` is a boolean on mod objects but a string enum on version objects.
- **`src/spt/`** — SPT directory interaction. `detect.rs` auto-detects SPT installs and reads version info from `core.json`. `mods.rs` handles archive extraction (ZIP/7z), file hashing, and mod file management. `profiles.rs` reads SPT player profiles. `server.rs` handles SPT server HTTP communication (HTTPS with self-signed certs, zlib compression disabled via `responsecompressed: 0` header).
- **`src/ops.rs`** — Core mod operations: `install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id`. These coordinate between db, filesystem, and archive extraction.
- **`src/health.rs`** — Health check system: server liveness, version verification, mod load verification, file integrity (SHA256).
- **`src/podman.rs`** — Podman container management for SPT server lifecycle.
- **`src/queue.rs`** — Change queue: mod operations are queued when SPT server is running, applied when stopped.
- **`src/server_detect.rs`** — Server running detection (Podman inspect or HTTP ping fallback).
- **`src/logging.rs`** — Structured logging with tracing. Supports console, file (with rotation), and web broadcast (SSE to browser). `LogBroadcast` uses a tokio broadcast channel + ring buffer.
- **`src/config.rs`** — Config types (serde TOML), env var overrides (`QUMA_*` prefix), and config resolution logic.
- **`src/error.rs`** — `QumaError` enum via thiserror for domain-specific errors.

### Web UI Stack

- **Templates**: Askama (compile-time checked) in `templates/`. Base layout in `base.html`, page templates extend it. Partials in `templates/partials/` and `templates/mods/partials/` for HTMX swap targets.
- **Frontend**: HTMX for interactivity (no JS build step). SSE for real-time updates (task progress, log streaming). Static assets (CSS, htmx.min.js, sse.js) embedded via rust-embed from `src/assets/`.
- **Sessions**: Signed cookies via actix-session (`CookieSessionStore`), 7-day TTL, SameSite=Strict, HttpOnly.
- **Rate limiting**: actix-governor on `/login` POST and `/register` (5 req/min/IP).
- **CSRF**: Token-based protection in `web/csrf.rs`.

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
