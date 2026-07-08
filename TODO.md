# TODO

## Top Priority
- move groups config to tab on mods page


## Bugs
- config editor flash message displays twice after save (once in base.html layout, once in template)
- SVM preset upload size limit too low (256 KB `FormConfig` on `/svm/preset/import`)
- SPT profile generation on account creation doesn't work
    - account creation dropdown missing SPT dev profiles (toggleable?)
- mod requests list shouldn't include already installed mods
- server-wide stats page has no PMC/Scav raid breakdown (per-user profile already tracks both)

## Core Architecture
- consolidate remaining mod management logic (`web/install.rs` shared helper exists, but queue apply still has its own path)
- stop using container image for spt-server, just run it natively?
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)
- build simpler, lighterweight container for headless, run spt server natively

## Headless Client
- SELinux `label=disable` still applied when GPU devices are present (volumes use `:z` shared label otherwise — #232)
- shared RW volume mount for base game dir (`converge.rs`)
- too-many-arguments on convergence functions (clippy lint suppressed)
- image name should be per-client configurable
- ensure headless + spt-server images have been pulled on startup
- better health client detection
- don't delete headless overlay by default, allow selection of which existing, not already in use overlay to use on new client creation (or when editing a client)
- be able to `podman rm` and re-init the client without wiping anything else
- set fika headless profiles early and use the same profiles for headless forever
- supervisor exit watchers cache restart policy/backoff values at spawn time — config changes require supervisor restart (`supervisor.rs`)
- headless recent raid stats should be linked to that information to the existing raids list

## Robustness
- no mutual exclusion on server start/stop/restart (`server.rs`)
- TOCTOU on duplicate mod install/update check (`mods.rs` — task-manager dedup mitigates double-click, but concurrent installs from different paths can still race past the pre-spawn check)
- no limit on concurrent SSE connections (`sse.rs`)
- unbounded zlib decompression — potential bomb (`raid_tracker.rs`, `proxy.rs`)
- silent cascade removal of reverse dependencies during queue apply (`queue.rs`)
- proxy buffers entire request body in memory (global 64 MB `PayloadConfig` caps it, but no proxy-specific limit — `proxy.rs`)
- SSE has no keepalive/heartbeat — proxies may close idle connections (`sse.rs`)

## Security
- proxy has no authentication — unauthenticated access to SPT server API (`proxy.rs`)
- `update_status_partial` serves privileged data to all authenticated users (`mods.rs`)
- profile/raid data visible to all authenticated users
- no mechanism to sync non-admin role permissions on upgrade (`sync_builtin_role_permissions` only covers admin)

## Web UI / Frontend

### UX Improvements
- clean up mod file list, add collapsable folder tree, file viewer (editor too maybe; need to investigate)
- 404 page
- cleanup tab and folder structure

### Accessibility / Responsive
- no responsive design (zero `@media` queries)
- no ARIA attributes (zero `aria-` in templates)
- no global HTMX error handling

## Features
- configurable backups
- custom headless instances
- MCP server?
- chatbot to help configure server?
- Quartermaster client app/launcher (tauri)
- fika.jsonc: set client force ip (needs research first if this is the right approach)
- better fika integration
    - general players list
    - expose config ui for `SPT/user/mods/fika-server/assets/configs/fika.jsonc`
- stand up server from predefined server config (storing settings in github without database)
- last logged for players (both into webui and into spt)
- display profile id on profile page
- SVM presets list should refresh from disk on page load
- user sorting
- better metrics: dynamic `by prefix` sorting, graphs
- profile editor
    - quests
    - items (scan for broken; remove; move)
- MOTD
- better formatting for SVM editor: section breakdown with header toggles, field name vs subtext, default value shown, download/upload preset, preset toggle
- discord integration:
    - use discord member list to define SVM AI PMC Names

### Convoy
- user config file sync

## Invites
- infinite use invite codes (no multi-use support, only single-use)

## Stash
- UX needs rework
- currency items (USD, EUR) displayed as roubles instead of as currency balances
