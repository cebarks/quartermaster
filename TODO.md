# TODO

## Top Priority
- SVM preset upload size limit
- mod forge link in mod info page doesn't always work (i.e. `https://tarkov.grovest.io/quma/mods/48`)
- unknown mods show as fika incompatible
- expected dependencies not shown in the queue

## Bugs
- canceling an install queue item puts it back in requests (if it was previously requested)
- multi-tab pages active tab highlighting is broken on nav
- account creation dropdown missing SPT dev profiles
- can't rekove already approved mods that haven't been installed

## Security
- `update_role_permissions` returns ambiguous `Ok(0)` for two different guard conditions (`rbac.rs`)
- `mod_backups_partial` readable by any authenticated user (`backup.rs` — mutating actions check `ModsUpdate`, but data is served without permission check)
- cookie-based sessions can't be individually revoked

## Core Architecture
- consolidate all mod management logic from all paths (web handlers bypass `ops.rs` in places)
- stop using container image for spt-server, just run it natively
- swap to using fika-installer in the headless client
- ~~config save ceremony repeated 8+ times~~ (reduced — `AppState::persist_config` used by settings+modsync; clients.rs 3× remain due to tokio::spawn constraints)
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)

- search handlers duplicated between mods and requests (`mods.rs`, `requests.rs`)
- refactor mod group implementation to be it's own system outside of narconet. narconet uses app-wide groups

## Headless Client
- convergence restarts SPT server without warning users (`converge.rs`)
- ~~SELinux fully disabled for ntsync (`converge.rs` — `label=disable` is overkill)~~ (partially done — #232, volumes use `:z` shared label; `label=disable` still applied when GPU devices are present)
- shared RW volume mount for base game dir (`converge.rs`)
- unauthenticated GitHub API requests (`converge.rs`)
- non-contiguous index handling after middle deletion (`converge.rs`)
- duplicate reqwest clients for GitHub (`converge.rs`)
- `client_port` integer overflow on large index (`converge.rs`)
- too-many-arguments on convergence functions (clippy lint suppressed)
- name a headless client (changes in-game profile name, also shows name in headless control panel)
- image name should be per-client configurable
- ensure headless + spt-server images have been pulled on startup
- better health client detection
- show active headless' in raid on dashboard, with player profile names, not ids
- don't delete headless overlay by default, allow selection of which existing, not already in use overlay to use on new client creation (or when editting a client)

## Robustness
- no mutual exclusion on server start/stop/restart (`server.rs`)
- TOCTOU on duplicate mod install/update check (`mods.rs` — task-manager dedup mitigates double-click, but concurrent installs from different paths can still race past the pre-spawn check)
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
- undefined CSS variables: `--primary`, `--error` (no fallbacks); `--warning-bg` has inline fallback but is never defined in `:root`
- clipboard API fails silently on HTTP
- no global HTMX error handling
- no responsive design
- no ARIA attributes
- toast messages auto-fade too fast for errors
- template duplication: ~~request cards~~, ~~client status table (3×)~~ (dead `clients/list.html` deleted), raid outcome badges (4×)
- tab implementation inconsistency — 4 different patterns across pages
- template duplication: client status table (3×), raid outcome badges (4×)
- clean up mod file list, add collapsable folder tree, file viewer (editor too maybe; need to investigate)

## Features
- automatic mod config backup via git
- fika.jsonc: set client force ip (needs research first if this is the right approach)
- better fika integration
    - pre-load raid on headless from webui
    - active players list on dashboard
    - general players list
- stand up server from predefined server config (storing settings in github without database) 
- last logged for players (both into webui and into spt)
- display profile id on profile page
- SVM presets list should refresh from disk on page load
- server notes page
- user sorting
- better metrics: dynamic `by prefix` sorting, graphs
- profile editor
- broadcast message via server to all clients (https://github.com/cebarks/fika-scripts)
- MOTD
- NarcoNet: better default sizes for extra/exclusions text areas
- better formatting for SVM editor: section breakdown with header toggles, field name vs subtext, default value shown, download/upload preset, preset toggle
- discord integration:
    - use discord member list to define SVM AI PMC Names
- fika config option interface

## Invites
- infinite use invite codes (no multi-use support, only single-use)

## Stash
- UX needs rework
- currency items (USD, EUR) displayed as roubles instead of as currency balances

---

## From Reviews

Items below are sourced from review/audit documents. See the linked file for full context.

### Headless Client Review (`HEADLESS_REVIEW.md`)

**Medium:**
- convergence restarts SPT server without warning users (`converge.rs:778-798`)
- ~~SELinux fully disabled for ntsync (`converge.rs:1017` — `label=disable` is overkill)~~ (partially done — #232)
- no overlay cleanup on scale-down (`converge.rs:841` — orphan dirs accumulate)
- supervisor exit watchers cache restart policy/backoff values at spawn time — config changes require supervisor restart (`supervisor.rs`)

**Low:**
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
- `update_role_permissions` returns ambiguous `Ok(0)` for two different guard conditions (F4, `rbac.rs:297`)

**Low:**
- `update_status_partial` serves privileged data to all authenticated users (F5, `mods.rs`)
- `mod_backups_partial` readable by any authenticated user (F9, `backup.rs` — mutating actions check `ModsUpdate`)

**Info:**
- cookie-based sessions can't be individually revoked (F13)
- profile/raid data visible to all authenticated users (F14)
- no mechanism to sync role permissions on upgrade (F15)

### Web Review (`WEB_REVIEW_2026-06-26.md`)

**Critical/High:**
- proxy buffers entire request body with no size limit (1.5, `proxy.rs:41`)
- ZIP archive for join page built entirely in memory (1.6, `join.rs:441`)

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
- undefined CSS variables: `--primary`, `--error` (no fallbacks); `--warning-bg` has inline fallback (5.2)
- clipboard API fails silently on HTTP (5.5)
- no global HTMX error handling (5.6)
- no responsive design (5.7)
- no ARIA attributes (5.8)
- toast messages auto-fade too fast for errors (5.10)

**Robustness:**
- no mutual exclusion on server start/stop/restart (6.1, `server.rs`)
- TOCTOU on duplicate mod install/update check (6.2, `mods.rs` — task-manager dedup mitigates double-click, but different request paths can still race)
- config file race in headless client background tasks (6.4, `clients.rs:466`)
- no limit on concurrent SSE connections (6.7, `sse.rs`)
