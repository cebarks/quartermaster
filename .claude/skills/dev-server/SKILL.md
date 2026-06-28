---
name: dev-server
description: Use when asked to run, start, test, verify, seed, or interact with the Quartermaster dev server, or when needing to confirm a UI change works in a running app.
---

# Dev Server

Run and interact with the Quartermaster dev environment. Accepts an optional mode argument; defaults to `start`.

## SPT Install Discovery

On first invocation, if `QUMA_SPT_DIR` is not set (environment or `.claude/settings.json`): ask the user for their SPT install path, validate it contains `SPT/`, and save to `.claude/settings.json` under `"env"`. The `dev-*` recipes override this with `.dev-server/`.

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
