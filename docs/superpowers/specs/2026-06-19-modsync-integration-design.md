# ModSync Integration Design

## Overview

Integrate ModSync management into quartermaster so that quma handles the full lifecycle: installing ModSync as a mod, managing its `config.jsonc` (syncPaths, exclusions), and keeping the config in sync with installed mod state. ModSync is pull-based — clients sync at game launch — so quma's role is ensuring the config accurately reflects what's installed.

## Background

[ModSync](https://github.com/c-orter/ModSync) is an SPT/Fika mod that synchronizes mods from server to client. The server plugin exposes HTTP endpoints on the SPT server; clients pull file hashes and downloads at startup.

Key ModSync concepts:
- **syncPaths**: Directories/files to sync, with per-path options (`name`, `enabled`, `enforced`, `silent`, `restartRequired`). Entries can be bare strings (path only, ModSync defaults apply) or objects (full control).
- **exclusions**: Glob patterns for files to skip
- **Config file**: `user/mods/Corter-ModSync/config.jsonc` (JSONC format)
- **Built-in paths**: ModSync always syncs its own updater and plugin DLL (hardcoded, not in config)
- **ModSync defaults**: `enforced: false`, `silent: false`, `restartRequired: true`, `enabled: true`

## Detection & Installation

### Detection

Mirror the existing `is_fika_installed()` pattern:

```rust
pub fn is_modsync_installed(spt_dir: &Path) -> bool {
    spt_dir.join("user/mods/Corter-ModSync").is_dir()
}
```

Check on `quma serve` startup and set `modsync_installed: bool` on `AppState`, same as `fika_installed`.

### Installation

ModSync is installed via quma like any other Forge mod. Rather than detecting ModSync by slug (fragile — slugs can change), quma runs `is_modsync_installed()` after every mod install/remove. If the check flips from false to true (or true to false), update `AppState.modsync_installed` accordingly.

When ModSync becomes installed:
1. Set `modsync_installed = true` on `AppState`
2. If `[modsync]` config exists in `quartermaster.toml`, immediately generate `config.jsonc`
3. If no `[modsync]` config, dashboard shows "ModSync: installed (not managed)" — admin enables management from settings

### Config path

Always `<spt_dir>/user/mods/Corter-ModSync/config.jsonc`.

## Configuration Model

New `[modsync]` section in `quartermaster.toml`:

```toml
[modsync]
# Global defaults applied to all synced mods
enforced = true          # default: true — force clients to match server (NOTE: diverges from ModSync's default of false)
silent = false           # default: false — prompt users before syncing
restart_required = true  # default: true — require game restart after sync

# Additional paths to sync beyond what quma auto-generates
extra_sync_paths = ["BepInEx/config"]

# Glob patterns to exclude from sync
exclusions = ["**/*.nosync", "BepInEx/plugins/spt"]

# Per-mod overrides — keyed by forge mod ID (stable identifier)
[modsync.overrides.12345]
enforced = false
silent = true
```

### Design decisions

- **Per-mod overrides keyed by Forge mod ID** — names can change, IDs are stable. Web UI displays mod name but stores the ID.
- **`extra_sync_paths`** — for paths not tied to a specific mod (e.g., `BepInEx/config`). These use the global defaults for `enforced`/`silent`/`restart_required` — no per-path overrides for extras (keep it simple, admin can adjust globals). Known v1 limitation: if the admin wants `BepInEx/config` non-enforced while plugins are enforced, they must set the global to `enforced = false` and override individual mods to `enforced = true`.
- **`exclusions`** — maps directly to ModSync's exclusions array, same glob syntax.
- **Only client/hybrid mods generate syncPaths** — server-only mods (`SPT/user/mods/`) never need syncing. Quma already knows mod type from tracked file paths (paths stored in DB use the `SPT/` prefix).
- **`enforced: true` default is deliberate** — this diverges from ModSync's own default of `false`. For a managed Fika server, the server host typically wants to guarantee mod parity. The settings page should note this divergence when first enabling management.
- **`[modsync]` is optional** — if absent, config management is disabled even if ModSync is installed. Admin must opt in.

### Rust types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncConfig {
    #[serde(default = "default_enforced")]
    pub enforced: bool,           // default: true

    #[serde(default)]
    pub silent: bool,             // default: false

    #[serde(default = "default_restart_required")]
    pub restart_required: bool,   // default: true

    #[serde(default)]
    pub extra_sync_paths: Vec<String>,

    #[serde(default)]
    pub exclusions: Vec<String>,

    // String keys because TOML requires string map keys.
    // Values are Forge mod ID as string (e.g., "12345").
    #[serde(default)]
    pub overrides: HashMap<String, ModSyncOverride>,
}

impl Default for ModSyncConfig {
    fn default() -> Self {
        Self {
            enforced: true,
            silent: false,
            restart_required: true,
            extra_sync_paths: Vec::new(),
            exclusions: Vec::new(),
            overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}
```

The `overrides` map uses `String` keys because TOML does not support integer map keys. Keys are Forge mod IDs as strings (e.g., `"12345"`), parsed to `i64` when resolving overrides during config generation.

Added to `Config` as:
```rust
#[serde(default)]
#[serde(skip_serializing_if = "Option::is_none")]
pub modsync: Option<ModSyncConfig>,
```

## Config Generation

New `src/modsync.rs` module with core logic.

### Generating syncPaths from DB state

1. Query all installed mods and their files from the DB
2. For each mod, check if it has files under `BepInEx/` — if so, it's sync-relevant
3. Group client-side files by top-level directory (e.g., `BepInEx/plugins/ModName/`, `BepInEx/patchers/SomePatcher.dll`) to produce minimal sync paths
4. For each sync path, emit a full SyncPath object (never bare strings) with:
   - `name`: set to the mod's name from DB (gives players readable sync prompts instead of raw paths)
   - `path`: the directory or file path
   - `enforced`, `silent`, `restartRequired`: from global defaults
   - `enabled`: `true` unless overridden
5. Apply per-mod overrides from `[modsync.overrides.<forge_id>]` if they exist — any non-None field replaces the default
6. Append `extra_sync_paths` entries as full SyncPath objects with global defaults (so they inherit `enforced`, etc. rather than falling back to ModSync's different defaults)
7. Add `exclusions` from config

### Writing config.jsonc

- Serialize with `serde_json::to_string_pretty`, then prepend the header comment: `// Generated by quartermaster — manual edits will be overwritten`
- ModSync's built-in paths (updater exe, plugin DLL) are hardcoded by ModSync, so quma doesn't include them
- Atomic write: write to `config.jsonc.tmp` in the same directory (`user/mods/Corter-ModSync/`), then `fs::rename` — same-directory ensures same-filesystem rename

### When regeneration triggers

- After every successful `install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id`
- After config changes via the web UI
- On `quma serve` startup if ModSync is detected and `[modsync]` config exists
- After queue drain completes

### Function signature

```rust
pub fn regenerate_if_enabled(
    spt_dir: &Path,
    config: &Config,
    db: &Database,
) -> Result<bool>  // returns true if config was written
```

## Integration Points

### ops.rs

Call `modsync::regenerate_if_enabled(spt_dir, config, db)` at the end of:
- `install_mod_from_archive`
- `update_mod_from_archive`
- `remove_mod_by_id`

Synchronous and fast (DB read + file write), no async needed.

### queue.rs

Trigger regeneration after all queued operations complete during drain.

### Web server startup

Check `is_modsync_installed()`, set `AppState.modsync_installed`. If enabled, regenerate config to ensure consistency with DB state.

## Web UI

### Dashboard badge

Small status indicator: "ModSync: active", "ModSync: installed (not managed)", or "ModSync: not installed". Links to settings.

### Settings page

Admin-only section at `/settings/modsync` (or section on existing settings page):

- **Global defaults**: enforced/silent/restart_required toggles
- **Extra sync paths**: text list, add/remove
- **Exclusions**: text list with glob support, add/remove
- **Per-mod overrides table**: lists all client/hybrid mods with toggles to override global defaults. Only shows mods with client-side files.

### Per-mod detail

On individual mod pages, if the mod has client-side files, show current sync settings (inherited defaults or overrides) with an edit link.

## Error Handling & Edge Cases

### ModSync uninstalled after config management enabled

Skip regeneration silently (debug log). `[modsync]` config stays in `quartermaster.toml` — harmless, auto-resumes if ModSync is reinstalled.

### ModSync installed or removed via quma

After every mod install/remove, `is_modsync_installed()` is rechecked. If ModSync was just installed, set `modsync_installed = true` and generate `config.jsonc` if `[modsync]` config exists. If ModSync was just removed via `remove_mod_by_id`, set `modsync_installed = false` on `AppState`. The orphaned `config.jsonc` is gone (it lived inside the mod directory which was deleted). Regeneration is skipped since `is_modsync_installed()` returns false.

### No client-side mods installed

Valid state. `config.jsonc` generated with empty `syncPaths` (plus `extra_sync_paths`) and `exclusions`. ModSync handles this gracefully.

### Config.jsonc already exists

Overwritten on first regeneration. Header comment makes this clear. Quma owns the file entirely.

### Concurrent writes

Regeneration reads DB under `Arc<Mutex<Database>>`. Worst case: two successive writes with identical content — harmless.

## Out of Scope

- Triggering client syncs (ModSync is pull-based, no push mechanism)
- Notifying players of available updates via web UI (future enhancement)
- Hash precalculation for ModSync's server plugin
- CLI commands for ModSync management (web UI is sufficient)
