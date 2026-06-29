# TODO
- bootstrap script should include all mods at the time of generation
- use gamescope instead of xvfb in headless container

## Core Architecture
- consolidate all mod management logic from all paths (web handlers bypass `ops.rs` in places)
- stop using container image for spt-server, just run it natively
- swap to using fika-installer in the headless client

## Web UI
- password boxes should have eye to toggle visibility

## Status Page
- no action to remove untracked directories (displayed but not actionable)
- untracked file list only shows directory-level summary, not individual files

## Profiles
- spt launcher profiles missing (only game profiles from `SPT/user/profiles/` are read)

## Invites
- infinite use invite codes (no multi-use support, only single-use)

## Raids
- `src/db/raids.rs` has ~14 stale `#[allow(dead_code)]` annotations

## Stash
- UX needs rework
- currency items (USD, EUR) displayed as roubles instead of as currency balances

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
