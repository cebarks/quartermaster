# TODO

## Top Priority
- mod requests expansion isn't clear, add a little arrow to the row to show that it can be expanded
- videos/screenshots and README cleanup
- zip/url mod update flow
- github release support

## Quick Wins
- infinite use invite codes (no multi-use support, only single-use)

## Bugs
- server-wide stats page has no PMC/Scav raid breakdown (per-user profile already tracks both)
- restarting a headless client from the main headless page causes you to end up at that headless' info page
- headless client start/stop/restart buttons on `/quma/headless` go past the card length
- currency items (USD, EUR) displayed as roubles instead of as currency balances (Stash)
- typo in update changelog: versions url should use `/#versions` hash anchor, not `/versions` path segment (current URL 404s on Forge)
- no auto-refresh when scaling/converging clients
- typo in update changelog: versions url should be like this `https://forge.sp-tarkov.com/mod/2310/wtt-commonlib/#versions` (`#version` is the important part)

## Convoy
- user config file sync
- user specific mods
- optional mod selection (server-side `tier` field exists on `CatalogGroup`, but no client-side UI for players to select/deselect optional groups)
- player sync status should be able to know if the last "up to date" sync was for the current catalog or not

## Core Architecture
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)

## Headless Client
- too-many-arguments on convergence functions (clippy lint suppressed)
- better health client detection
- don't delete headless overlay by default, allow selection of which existing, not already in use overlay to use on new client creation (or when editing a client)
- be able to `podman rm` and re-init the client without wiping anything else
- set fika headless profiles early and use the same profiles for headless forever
- supervisor exit watchers cache restart policy/backoff values at spawn time — config changes require supervisor restart (`supervisor.rs`)
- headless recent raid stats should be linked to the existing raids list
- add client should add a new row to the list with the client name column editable, and a save or cancel button replacing the existing buttons in the actions column
- headless client actions
- container cpu stats don't work (`cpu_percent` is never populated — always `None`; memory stats work)
- ability to pull files to overlay in webui

## Robustness
- no mutual exclusion on server start/stop/restart (`server.rs`)
- TOCTOU on duplicate mod install/update check (`mods.rs` — task-manager dedup mitigates double-click, but concurrent installs from different paths can still race past the pre-spawn check)
- no limit on concurrent SSE connections (`sse.rs`)
- unbounded zlib decompression — potential bomb (`raid_tracker.rs`, `proxy.rs`)
- proxy buffers entire request body in memory (global 64 MB `PayloadConfig` caps it, but no proxy-specific limit — `proxy.rs`)
- SSE has no keepalive/heartbeat — proxies may close idle connections (`sse.rs`)

## Security
- profile/raid data visible to all authenticated users
- no mechanism to sync non-admin role permissions on upgrade (`sync_builtin_role_permissions` only covers admin)
- convoy group handlers (`groups_partial`, `new_group_card`, `save_groups`) use `Permission::ModsInstall` instead of `Permission::ConvoyManage` — privilege downgrade (`mods.rs`)

## Web UI / Frontend

### UX Improvements
- clean up mod file list, add collapsable folder tree, file viewer (editor too maybe; need to investigate)
- 404 page (catch-all route goes to proxy, not styled error template)
- cleanup tab and folder structure
- move most things from polling to pushing
- Stash UX needs rework

### Accessibility / Responsive
- no responsive design (zero `@media` queries)
- no ARIA attributes (zero `aria-` in templates)
- no global HTMX error handling

## Features
- implement `https://db.sp-tarkov.com/search` like functionality (give-items has local item search, but no standalone general-purpose item browser page)
- better SVM editor: default values shown alongside current, file-based preset upload (section tabs, header toggles, field name/subtext, preset toggle/export already done)
- last logged for players (both into webui and into spt)
- user sorting
- better metrics: dynamic `by prefix` sorting, graphs
- profile editor
    - quests
    - items (scan for broken; remove; move)
- MOTD
- discord integration:
    - use discord member list to define SVM AI PMC Names
- RaidReview support
- container deployment (for Quartermaster itself, not SPT server)
- full server folder backups (current backups only cover quma-managed artifacts: mods, addons, profiles, config)
