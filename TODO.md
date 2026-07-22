# TODO

## Top Priority
- mod requests expansion isn't clear, add a little arrow to the row to show that it can be expanded
- videos/screenshots and README cleanup
- zip/url mod update flow
- mod queue should download and make a mod staged and ready to be copied,

## Quick Wins
- mod update card has no link to the forge versions page that has the changelog on it
- SVM preset upload size limit too low (256 KB `FormConfig` on `/svm/preset/import`)
- rejecting an approved mod leaves behind an empty row
- display profile id on profile page
- infinite use invite codes (no multi-use support, only single-use)
- SVM presets list should refresh from disk on page load
- check for updates button on `/quma/mods`

## Bugs
- config editor flash message displays twice after save (once in base.html layout, once in template)
- SPT profile generation on account creation doesn't work
    - account creation dropdown missing SPT dev profiles (toggleable?)
- server-wide stats page has no PMC/Scav raid breakdown (per-user profile already tracks both)
- no auto-refresh when scaling/converging/restarting clients
- restarting a headless client from the main headless page causes you to end up at that headless' info page
- headless client start/stop/restart buttons on `/quma/headless` go past the card length
- currency items (USD, EUR) displayed as roubles instead of as currency balances (Stash)
- deleting a user doesn't actually take affect
- requesting a mod that's already installed fail

## Convoy
- user config file sync
- user specific mods
- optional mod selection
- player sync status should be able to know if the last "up to date" sync was for the current catalog or not

## Core Architecture
- `WebError` always returns HTML even for API endpoints (`error.rs`)
- blocking filesystem reads on async runtime (partially fixed — `svm::save_section` uses `web::block`, many others don't)

## Headless Client
- SELinux `label=disable` still applied when GPU devices are present (volumes use `:z` shared label otherwise — #232)
- shared RW volume mount for base game dir (`converge.rs`)
- too-many-arguments on convergence functions (clippy lint suppressed)
- ensure headless + spt-server images have been pulled on startup
- better health client detection
- don't delete headless overlay by default, allow selection of which existing, not already in use overlay to use on new client creation (or when editing a client)
- be able to `podman rm` and re-init the client without wiping anything else
- set fika headless profiles early and use the same profiles for headless forever
- supervisor exit watchers cache restart policy/backoff values at spawn time — config changes require supervisor restart (`supervisor.rs`)
- headless recent raid stats should be linked to the existing raids list
- add client should add a new row to the list with the client name column editable, and a save or cancel button replacing the existing buttons in the actions column
- headless client actions
- container cpu/memory stats doesn't work
- ability to pull files to overlay in webui
- headless mods/config don't stay in sync with quartermaster

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
- 404 page
- cleanup tab and folder structure
- move most things from polling to pushing
- Stash UX needs rework

### Accessibility / Responsive
- no responsive design (zero `@media` queries)
- no ARIA attributes (zero `aria-` in templates)
- no global HTMX error handling

## Features
- replace mongoid's with actual name across whole app
- implement `https://db.sp-tarkov.com/search` like functionality except based on the modded local database
- windows support
- configurable backups
- custom headless instances
- fika.jsonc: set client force ip (needs research first if this is the right approach)
- better fika integration
    - all players list
    - online players
- last logged for players (both into webui and into spt)
- user sorting
- better metrics: dynamic `by prefix` sorting, graphs
- profile editor
    - quests
    - items (scan for broken; remove; move)
- MOTD
- better formatting for SVM editor: section breakdown with header toggles, field name vs subtext, default value shown, download/upload preset, preset toggle
- discord integration:
    - use discord member list to define SVM AI PMC Names
- RaidReview support
- container deployment
- full server folder backups 
