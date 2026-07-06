# TODO

## Top Priority
- !!! mod requests/queue/installed lifecycle review
    - canceling an install queue item puts it back in requests (if it was previously requested)
- !!! Notes page
- SVM preset upload size limit
- mod config management
- move groups config to tab on mods page

## Triage
- fika settings

## Bugs
- account creation dropdown missing SPT dev profiles
- can't rekove already approved mods that haven't been installed
- mod requests list shouldn't include already installed mods
- numbers on the stats board (particularly map popularity and total raids) seem wrong
- /quma/raids returns 500 when no raid data exists (handler doesn't handle empty state)

## Core Architecture
- consolidate all mod management logic from all paths (web handlers bypass `ops.rs` in places)
- stop using container image for spt-server, just run it natively?
- ~~config save ceremony repeated 8+ times~~ (reduced — `AppState::persist_config` used by settings+modsync; clients.rs 3× remain due to tokio::spawn constraints)
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)
- refactor mod group implementation to be it's own system outside of narconet. narconet uses app-wide groups
- build simpler, lighterweight container

## Headless Client
- ~~SELinux fully disabled for ntsync (`converge.rs` — `label=disable` is overkill)~~ (partially done — #232, volumes use `:z` shared label; `label=disable` still applied when GPU devices are present)
- shared RW volume mount for base game dir (`converge.rs`)
- too-many-arguments on convergence functions (clippy lint suppressed)
- name a headless client (changes in-game profile name, also shows name in headless control panel)
- image name should be per-client configurable
- ensure headless + spt-server images have been pulled on startup
- better health client detection
- don't delete headless overlay by default, allow selection of which existing, not already in use overlay to use on new client creation (or when editting a client)
- be able to `podman rm` and re-init the client without wiping anything else
- set fika headless profiles early and use the same profiles for headless forever

## Robustness
- no mutual exclusion on server start/stop/restart (`server.rs`)
- TOCTOU on duplicate mod install/update check (`mods.rs` — task-manager dedup mitigates double-click, but concurrent installs from different paths can still race past the pre-spawn check)
- no limit on concurrent SSE connections (`sse.rs`)
- unbounded zlib decompression — potential bomb (`raid_tracker.rs`, `proxy.rs`)
- silent cascade removal of reverse dependencies during queue apply (`queue.rs`)
- proxy buffers entire request body with no size limit (`proxy.rs`)
- SSE has no keepalive/heartbeat — proxies may close idle connections (`sse.rs`)

## Web UI / Frontend

### UX Improvements
- clean up mod file list, add collapsable folder tree, file viewer (editor too maybe; need to investigate)
- 404 page

### Accessibility / Responsive
- no responsive design (zero `@media` queries)
- no ARIA attributes (zero `aria-` in templates)
- no global HTMX error handling

## Features
- automatic mod config backup via git
- Quartermaster client app (tauri)
- fika.jsonc: set client force ip (needs research first if this is the right approach)
- better fika integration
    - general players list
    - expose config ui for `SPT/user/mods/fika-server/assets/configs/fika.jsonc`
- stand up server from predefined server config (storing settings in github without database) 
- last logged for players (both into webui and into spt)
- display profile id on profile page
- SVM presets list should refresh from disk on page load
- server notes page
- user sorting
- better metrics: dynamic `by prefix` sorting, graphs
- profile editor
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

### Headless Client

**Medium:**
- supervisor exit watchers cache restart policy/backoff values at spawn time — config changes require supervisor restart (`supervisor.rs`)

**Low:**
- web handler boilerplate in client lifecycle handlers (`clients.rs`)

### Permissions

**High:**
- proxy has no authentication — unauthenticated access to SPT server API (`proxy.rs`)

**Low:**
- `update_status_partial` serves privileged data to all authenticated users (`mods.rs`)

**Info:**
- profile/raid data visible to all authenticated users
- no mechanism to sync role permissions on upgrade

### Web

**Critical/High:**
- proxy buffers entire request body with no size limit (`proxy.rs`)

**Architecture/Performance:**
- install logic duplicated between mods and requests handlers
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)
