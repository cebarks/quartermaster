# BUGS


# TODO
- there should be intermediate states displayed for the status page when starting stopping or restarting the server
- server uptime
- container stats (cpu, ram, storage, networking)
- integrity check frequency should be cached, refresh configurable with a button to check on-demand

- spt files are still showing as untracked in the integrity check — `find_unmanaged_mod_dirs()` filters `BepInEx/plugins/spt` but `check_integrity_from()` in `health.rs` does not. Ideally create a default "SPT" mod entry that owns these files.

- in-app notifications dropdown in the nav bar
- stash value in admin user profile cards (requires inventory iteration + item price data)

## Dedicated Clients (post-review)
- SELinux label for shared `install_dir` mount uses `:Z` (Private) instead of `:z` (Shared) — breaks multi-client on SELinux-enforcing systems. Fix in `converge.rs:406`.
- CLI detail view (`quma client status N`) shows wrong Fika client — uses `map.iter().next()` instead of PROFILE_ID correlation via container inspect. Same issue in table view.
- Web scale handler (`POST /clients/scale`) doesn't persist config to disk — changes lost on restart. CLI `scale` command does this correctly.
- Dual converging flag (supervisor uses `AtomicBool`, convergence uses `RwLock<bool>`) — unify on `AtomicBool`.
- Convergence engine missing server restart and profile discovery — containers lack `PROFILE_ID` env var. The helper functions exist (`discover_new_profiles`) but are not wired in.
- Supervisor `restart_client` sleeps inside tick loop — blocks all other client checks during backoff (up to 5 min). Spawn restarts as independent tokio tasks instead.
- Web scale-down blocks `Ready` clients, not just `InRaid` — overly conservative, inconsistent with CLI behavior. Remove `EHeadlessStatus::Ready` from the filter in `clients.rs:367`.
- CLI status shows "connected" for all clients if any headless data exists — needs per-client PROFILE_ID correlation like the supervisor does.

# To Investigate
- proxying the spt http server itself

# Future Features

- **Player mod request/voting**: Players can suggest mods via the web UI, admin approves/rejects
- **Mod search**: `quma search` CLI command and web UI search page with HTMX live search
- **Operation history**: `operation_history` table, `quma log` command, and web UI changelog page tracking all mod operations with timestamps, who performed them, and whether `--force` was used
- **Trust mode auth**: No-password auth mode where players just pick their SPT profile — matches SPT's own security model
- **Windows support**: Native Windows process management (no Podman requirement), Windows-specific paths and process detection
- **ModSync integration**: 
    - Trigger ModSync push after mod changes
    - Manage ModSync `syncpaths` and `exclusions` config
- **Backup/restore**: Configurable auto-snapshot before risky operations (updates, removes) via `auto_backup = true`; configurable backup directory via `backup_dir`; `quma backup` / `quma restore` commands; snapshots include mod files, profiles, and configs
- **Mod profiles**: Save/load sets of mods for different playstyles
- **Raid statistics / leaderboard**: Parse SPT profile JSON for raid history, survival rate, K/D, stash value — fun leaderboard page in web UI
- **Discord integration**: Webhook notifications (server up/down, mod changes); later a full bot with slash commands (`/status`, `/mods`, `/restart`)
- **Mod configuration UI**: Web-based editor for mod config files (JSON/JSONC/CFG) — discover configs in installed mods, render editable forms or syntax-highlighted editor
- **Scheduled restarts**: Built-in cron-style scheduling for automatic server restarts (e.g., daily at 4am)
- **Server MOTD / rules page**: Admin-editable message displayed on dashboard for all players
- **REST API**: JSON API (`/api/v1/`) for external tooling, Discord bots, and monitoring integrations
- **Full SPT setup**: Download SPT release from GitHub, copy EFT files, run downgrade patcher — full zero-to-server automation
- **Multi-server**: Manage mods across multiple SPT instances
- **Setup Wizard**: on first run have a wizard that helps you configure and setup quartermaster and spt.
