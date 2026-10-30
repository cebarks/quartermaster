---
name: dev-server
description: Use when asked to run, start, test, verify, seed, or interact with the Quartermaster dev server, or when needing to confirm a UI change works in a running app.
---

# Dev Server

Run and interact with the Quartermaster dev environment. Accepts an optional mode argument; defaults to `start`.

## Target Environment

The `dev-*` recipes are self-contained: `just dev-init` bootstraps a Quartermaster data root at `.dev-server/` (`quma setup --quma-dir .dev-server`), and `just dev-serve`, `just dev-watch`, `just dev-seed`, and `just dev-cli` all depend on `dev-init` and pin `QUMA_DIR=.dev-server` on their own recipe line. Never ask for an unrelated existing SPT install in order to use these modes, and never prefix them with `QUMA_DIR=<path> …` — the recipe's own assignment wins, so the value is ignored.

`QUMA_DIR` selects a Quartermaster **data root**, not an SPT runtime directory: the directory holding `quartermaster.toml`, `quartermaster.db`, and the nested `spt-server/` tree, whose runtime directory inside it is `SPT_Runtime/` (`SPT/` on legacy installs — `src/spt/layout.rs`). The dev install therefore lives at `.dev-server/spt-server/SPT_Runtime/`. `QUMA_SPT_DIR` is a deprecated alias for the same data-root setting (it points at a legacy flat layout, where the data root is the SPT directory); prefer `QUMA_DIR`. See `QumaDirs::detect` in `src/dirs.rs` and `apply_env_overrides` in `src/config.rs`.

To run commands against an existing data root instead of `.dev-server/`, use the raw form, which does honor the selected root: `QUMA_DIR=<data-root> cargo run -- <args>`.

## Modes

Accepts: `start` (default), `verify`, `interact`, `teardown`.

## start (default)

1. `just dev-info` — **always first** — read assigned port and container name
2. `just dev-init` — bootstrap dev environment (idempotent)
3. `just dev-seed` — populate test data (not optional — always seed)
4. Start `just dev-serve` in background (use `just dev-watch` instead if actively editing source)
5. Wait for server: `curl -sfk https://localhost:<PORT>/quma/login` (retry up to 30s, `-k` for self-signed cert)
6. Report URL (`https://localhost:<PORT>/quma/`), port, and credentials

## verify

1. Check server is reachable: `curl -sfk https://localhost:<PORT>/quma/login`; if down, run `start` first
2. Login (two-step, CSRF-protected):
   - GET login page: `curl -k -c /tmp/quma-cookies https://localhost:<PORT>/quma/login -s -o /tmp/quma-login.html`
   - Extract CSRF token: `grep -oP 'name="csrf_token" value="\K[^"]+' /tmp/quma-login.html`
   - POST login: `curl -k -c /tmp/quma-cookies -b /tmp/quma-cookies -d 'username=admin&password=devdevdev&csrf_token=<TOKEN>' -L https://localhost:<PORT>/quma/login`
3. Hit pages with session cookie and check for HTTP 200: `/quma/`, `/quma/mods`, `/quma/settings`, `/quma/profiles`, `/quma/raids`, `/quma/admin`
4. Report any non-200 responses or HTML error content
5. If verifying a specific feature, navigate to that page too

All routes are under the `/quma` scope.

## interact

1. Check server is reachable; if down, run `start` first
2. Execute the requested interaction:
   - CLI: `just dev-cli <args>`
   - Database: `sqlite3 .dev-server/quartermaster.db "<query>"`
   - HTTP: `curl` to specific endpoints
3. Report results

## teardown

Teardown is scoped to the dev environment only — do NOT touch worktrees, other containers, or unrelated infrastructure.

1. Stop the background `dev-serve` process
2. Default: `just dev-reset-db` (wipe data, keep structure)
3. If full cleanup requested: `just dev-clean` (removes container + directory)

## Quick Reference

| Item | Value |
|------|-------|
| Login | `admin` / `devdevdev` (all seed users share this password) |
| Seed users | admin, ModeratorMike, TarkovChad, LootGoblin, ExtractCamper, ProfileOnlyUser |
| Dev dir / DB | `.dev-server/` / `.dev-server/quartermaster.db` |
| TLS | Self-signed cert — always use `curl -k` |

## Troubleshooting

If `dev-serve` fails: `podman info` (podman running?), `podman ps -a --filter name=<container>` (exists?), `podman stop <name> && podman rm <name>` then `just dev-init` (stuck?), `ss -tlnp | grep <PORT>` (port conflict?).

**Always `just dev-info` first** — never hardcode 9190.
