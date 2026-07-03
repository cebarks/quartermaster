# TODO

## Top Priority
- bootstrap script should include all mods at the time of generation
- SVM preset upload/max size limit
- mod forge link in mod info page doesn't always work (i.e. `https://tarkov.grovest.io/quma/mods/48`)
- download mods on setup page should be async, not blocking while it generates the ZIP. We should also cache the generated ZIP until there are mod changes.

## Bugs
- canceling an install queue item puts it back in requests (if it was previously requested)
- CLI `delete` cleans wrong overlay directory (`cli/headless.rs` — uses `spt_dir/clients/` instead of `install_dir/.quma/clients/`)
- scale-down in-raid check blocks on Ready clients (`clients.rs` — remove `Ready` from fika_status match, only `InRaid` should block)
- NarcoNet tab selection broken on nav
- fix account registration not actually creating a new SPT profile (not hitting spt server endpoint correctly); also account creation dropdown missing SPT dev profiles
- magic number 236 still used for SVM mod ID in `mods.rs` — `SVM_FORGE_ID` constant exists in `svm/mod.rs` but isn't used at all call sites

## Security
- proxy has no authentication — unauthenticated access to SPT server API (`proxy.rs`)
- `dismiss_task` has no permission check — any user can dismiss any task (`tasks.rs`)
- CSRF token not rotated after use (`csrf.rs`)
- per-worker rate limiting, not global (`mod.rs`)
- `update_role_permissions` returns ambiguous `Ok(0)` for two different guard conditions (`rbac.rs`)
- `client_detail` page accessible without `HeadlessManage` permission (`clients.rs`)
- `mod_backups_partial` accessible without `ModsUpdate` permission (`backup.rs`)
- `admin_exists()` uses hardcoded role name instead of checking `UsersManage` permission (`users.rs`)
- no FK constraint on `users.role` → `roles.name` (`migrations/001`)
- `role_permissions.permission` column is unconstrained (`migrations/001`)
- session secret is ephemeral if not configured
- cookie-based sessions can't be individually revoked
- no mechanism to sync role permissions on upgrade

## Core Architecture
- consolidate all mod management logic from all paths (web handlers bypass `ops.rs` in places)
- stop using container image for spt-server, just run it natively
- swap to using fika-installer in the headless client
- install logic duplicated between mods and requests handlers
- config save ceremony repeated 8+ times
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)
- search handlers duplicated between mods and requests (`mods.rs`, `requests.rs`)

## Headless Client
- convergence restarts SPT server without warning users (`converge.rs`)
- SELinux fully disabled for ntsync (`converge.rs` — `label=disable` is overkill)
- no overlay cleanup on scale-down (orphan dirs accumulate)
- shared RW volume mount for base game dir (`converge.rs`)
- unauthenticated GitHub API requests (`converge.rs`)
- non-contiguous index handling after middle deletion (`converge.rs`)
- duplicate reqwest clients for GitHub (`converge.rs`)
- `client_port` integer overflow on large index (`converge.rs`)
- web handler boilerplate in client lifecycle handlers (`client_restart`/`client_stop`/`client_start` near-identical)
- too-many-arguments on convergence functions (clippy lint suppressed)

## Robustness
- no mutual exclusion on server start/stop/restart (`server.rs`)
- TOCTOU on duplicate mod install/update check (`mods.rs`)
- filesystem/DB atomicity gap in mod operations (partially fixed — install path uses staging dir, update path still has gap)
- config file race in headless client background tasks (`clients.rs`)
- no limit on concurrent SSE connections (`sse.rs`)
- no upper bound on headless client scale count (`clients.rs`)
- unbounded zlib decompression — potential bomb (`raid_tracker.rs`, `proxy.rs`)
- silent cascade removal of reverse dependencies during queue apply (`queue.rs`)
- proxy buffers entire request body with no size limit (`proxy.rs`)
- SSE has no keepalive/heartbeat — proxies may close idle connections (`sse.rs`)

## Web UI / Frontend
- password boxes should have eye to toggle visibility
- `render_user_row` reloads ALL profile stats for one row (`admin.rs`)
- static assets served with no caching
- no download size limit on Forge downloads (`forge/client.rs`)
- metrics page polls every 1 second
- missing CSS class definitions: `.badge-error`, `.status-dot.degraded`, `.badge-primary`, `.form-control`, `.alert-danger`
- undefined CSS variables: `--primary`, `--error`, `--warning-bg`
- clipboard API fails silently on HTTP
- no global HTMX error handling
- no responsive design
- no ARIA attributes
- toast messages auto-fade too fast for errors
- tab implementation inconsistency — 4 different patterns across pages
- template duplication: request cards, client status table (3×), raid outcome badges (4×)

## Features
- fika.jsonc: set client force ip
- last logged for players (both into webui and into spt)
- "culture center" profile page investigation
- display profile id on profile page
- SVM presets list should refresh from disk on page load
- server notes page
- user sorting
- narconet setting preview
- better metrics: dynamic `by prefix` sorting, graphs
- profile editor
- broadcast message via server to all clients (https://github.com/cebarks/fika-scripts)
- MOTD
- NarcoNet: better default sizes for extra/exclusions text areas
- better formatting for SVM editor: section breakdown with header toggles, field name vs subtext, default value shown, download/upload preset, preset toggle
- riusep discord member list to define SVM AI PMC Names
- `quma headless build` CLI command — clone gitlab.com/claudeoris/spt-builds and run podman build to produce localhost/fika-headless:latest

---

## From Reviews

Items below are sourced from review/audit documents. See the linked file for full context.

### Headless Client Review (`HEADLESS_REVIEW.md`)

**High:**
- CLI `delete` cleans wrong overlay directory (`cli/headless.rs:361` — uses `spt_dir/clients/` instead of `install_dir/.quma/clients/`)
- scale-down in-raid check blocks on Ready clients in web UI (`clients.rs:358` — remove `Ready` from match)

**Medium:**
- convergence restarts SPT server without warning users (`converge.rs:778-798`)
- SELinux fully disabled for ntsync (`converge.rs:1017` — `label=disable` is overkill)
- no overlay cleanup on scale-down (`converge.rs:841` — orphan dirs accumulate)
- supervisor doesn't pick up config changes at runtime (`supervisor.rs:19`)

**Low:**
- scopeguard race on `restarting` flag (`supervisor.rs:449`)
- shared RW volume mount for base game dir (`converge.rs:920`)
- unauthenticated GitHub API requests (`converge.rs:189,249`)
- non-contiguous index handling after middle deletion (`converge.rs:581`)
- duplicate reqwest clients for GitHub (`converge.rs:189,249`)
- web handler boilerplate in client lifecycle handlers (`clients.rs:79-306`)
- too-many-arguments on convergence functions

### Permissions Audit (`PERMISSIONS_AUDIT.md`)

**High:**
- proxy has no authentication — unauthenticated access to SPT server API (F1, `proxy.rs:20`)

**Medium:**
- `dismiss_task` has no permission check — any user can dismiss any task (F2, `tasks.rs`)
- inconsistent admin handler authorization — some rely only on scoped middleware (F3, `admin.rs`)
- `update_role_permissions` returns ambiguous `Ok(0)` for two different guard conditions (F4, `rbac.rs:297`)

**Low:**
- `update_status_partial` serves privileged data to all authenticated users (F5, `mods.rs`)
- no FK constraint on `users.role` → `roles.name` (F6, `migrations/001`)
- `admin_exists()` uses hardcoded role name instead of checking `users.manage` permission (F7, `users.rs:133`)
- `client_detail` page accessible without `HeadlessManage` permission (F8, `clients.rs`)
- `mod_backups_partial` accessible without `ModsUpdate` permission (F9, `backup.rs`)
- `role_permissions.permission` column is unconstrained (F11, `migrations/001`)

**Info:**
- session secret is ephemeral if not configured (F12)
- cookie-based sessions can't be individually revoked (F13)
- profile/raid data visible to all authenticated users (F14)
- no mechanism to sync role permissions on upgrade (F15)

### Web Review (`WEB_REVIEW_2026-06-26.md`)

**Critical/High:**
- stuck "restarting" transition state on queue drain error (1.3, `server.rs:143,168`)
- `expect()` panic on TOCTOU in `client_scale` (1.4, `clients.rs:333,408`)
- proxy buffers entire request body with no size limit (1.5, `proxy.rs:41`)
- ZIP archive for join page built entirely in memory (1.6, `join.rs:441`)

**Security:**
- path traversal via PHPSESSID cookie in raid tracker (2.2, `raid_tracker.rs:65`)
- no password complexity requirements (2.7, `auth.rs`)
- per-worker rate limiting, not global (2.8, `mod.rs:105`)

**Architecture/Performance:**
- install logic duplicated between mods and requests handlers (3.2)
- config save ceremony repeated 8+ times (3.3)
- `render_user_row` reloads ALL profile stats for one row (3.4, `admin.rs:780`)
- `WebError` always returns HTML even for API endpoints (3.5, `error.rs`)
- blocking filesystem reads on async runtime (3.6, `settings.rs`, `svm.rs`)
- static assets served with no caching (4.1)
- no download size limit on Forge downloads (4.4, `forge/client.rs:191`)
- metrics page polls every 1 second (4.5)

**Frontend/UX:**
- missing CSS class definitions: `.badge-error`, `.status-dot.degraded`, `.badge-primary`, `.form-control`, `.alert-danger` (5.1)
- undefined CSS variables: `--primary`, `--error`, `--warning-bg` (5.2)
- clipboard API fails silently on HTTP (5.5)
- no global HTMX error handling (5.6)
- no responsive design (5.7)
- no ARIA attributes (5.8)
- log viewer unbounded DOM growth in follow mode (5.9)
- toast messages auto-fade too fast for errors (5.10)

**Robustness:**
- no mutual exclusion on server start/stop/restart (6.1, `server.rs`)
- TOCTOU on duplicate mod install/update check (6.2, `mods.rs:832`)
- filesystem/DB atomicity gap in mod operations (6.3, `ops.rs:35`)
- config file race in headless client background tasks (6.4, `clients.rs:466`)
- migrations not wrapped in transactions (6.6, `schema.rs:8`)
- no limit on concurrent SSE connections (6.7, `sse.rs`)
- missing symlink check in 7z extraction (6.11, `spt/mods.rs:324`)
- invite endpoints missing permission checks (6.12, `admin.rs:465`)
