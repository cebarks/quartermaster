# ModSync Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate ModSync config management into quartermaster so that `config.jsonc` is automatically generated from installed mod state, with per-mod overrides and a web UI settings page.

**Architecture:** New `src/modsync.rs` module owns all ModSync logic (detection, config generation, file writing). Config types are added to `src/config.rs`. The `ops.rs` functions and web handlers call `modsync::regenerate_if_enabled()` after every mod operation. A web UI settings page at `/modsync` exposes global defaults, per-mod overrides, extra sync paths, and exclusions.

**Tech Stack:** Rust, serde/serde_json (config.jsonc generation), Askama templates (web UI), HTMX (frontend interactivity), actix-web handlers

## Global Constraints

- Rust edition 2021, `cargo clippy -- -D warnings` must pass
- All config types must implement `Debug, Clone, Serialize, Deserialize, PartialEq`
- Web templates use Askama with compile-time checking — templates that don't match their struct fields cause build failures
- TOML does not support integer map keys — use `HashMap<String, _>` for override maps
- All web handler tests use the existing `Database::open_in_memory()` pattern
- ModSync config.jsonc path is always `<spt_dir>/user/mods/Corter-ModSync/config.jsonc`
- Run `just lint` (fmt + clippy) before each commit

---

### Task 1: Config Types

Add `ModSyncConfig` and `ModSyncOverride` types to `src/config.rs`, add the `modsync` field to `Config`, and add the `is_modsync_installed()` detection function.

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Produces: `ModSyncConfig`, `ModSyncOverride` types, `Config.modsync: Option<ModSyncConfig>`, `is_modsync_installed(spt_dir: &Path) -> bool`

- [ ] **Step 1: Write tests for config types**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/config.rs`:

```rust
#[test]
fn modsync_config_defaults() {
    let config: Config = toml::from_str("").expect("empty config");
    assert!(config.modsync.is_none());
}

#[test]
fn modsync_config_full_deserialization() {
    let toml_str = r#"
[modsync]
enforced = false
silent = true
restart_required = false
extra_sync_paths = ["BepInEx/config", "BepInEx/patchers"]
exclusions = ["**/*.nosync", "BepInEx/plugins/spt"]

[modsync.overrides.12345]
enforced = true
silent = false

[modsync.overrides.67890]
enabled = false
"#;
    let config: Config = toml::from_str(toml_str).expect("should parse");
    let ms = config.modsync.unwrap();
    assert!(!ms.enforced);
    assert!(ms.silent);
    assert!(!ms.restart_required);
    assert_eq!(ms.extra_sync_paths, vec!["BepInEx/config", "BepInEx/patchers"]);
    assert_eq!(ms.exclusions, vec!["**/*.nosync", "BepInEx/plugins/spt"]);
    assert_eq!(ms.overrides.len(), 2);

    let o1 = &ms.overrides["12345"];
    assert_eq!(o1.enforced, Some(true));
    assert_eq!(o1.silent, Some(false));
    assert_eq!(o1.restart_required, None);
    assert_eq!(o1.enabled, None);

    let o2 = &ms.overrides["67890"];
    assert_eq!(o2.enabled, Some(false));
    assert_eq!(o2.enforced, None);
}

#[test]
fn modsync_config_minimal_with_defaults() {
    let toml_str = "[modsync]\n";
    let config: Config = toml::from_str(toml_str).expect("should parse");
    let ms = config.modsync.unwrap();
    assert!(ms.enforced);       // default: true
    assert!(!ms.silent);        // default: false
    assert!(ms.restart_required); // default: true
    assert!(ms.extra_sync_paths.is_empty());
    assert!(ms.exclusions.is_empty());
    assert!(ms.overrides.is_empty());
}

#[test]
fn modsync_config_skip_serializing_when_none() {
    let config = Config::default();
    let serialized = toml::to_string_pretty(&config).unwrap();
    assert!(
        !serialized.contains("[modsync]"),
        "None modsync should not be serialized"
    );
}

#[test]
fn modsync_config_roundtrip() {
    let mut config = Config::default();
    config.modsync = Some(ModSyncConfig {
        enforced: false,
        silent: true,
        ..ModSyncConfig::default()
    });
    let serialized = toml::to_string_pretty(&config).unwrap();
    let reloaded: Config = toml::from_str(&serialized).unwrap();
    assert_eq!(config.modsync, reloaded.modsync);
}

#[test]
fn modsync_detection() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(!is_modsync_installed(tmp.path()));
    std::fs::create_dir_all(tmp.path().join("user/mods/Corter-ModSync")).unwrap();
    assert!(is_modsync_installed(tmp.path()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p quartermaster modsync_config -- --nocapture 2>&1 | head -40`

Expected: compilation errors — types don't exist yet.

- [ ] **Step 3: Implement config types and detection function**

Add the following to `src/config.rs`, above the `Config` struct definition:

```rust
use std::collections::HashMap;

fn default_enforced() -> bool {
    true
}

fn default_restart_required() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncConfig {
    #[serde(default = "default_enforced")]
    pub enforced: bool,

    #[serde(default)]
    pub silent: bool,

    #[serde(default = "default_restart_required")]
    pub restart_required: bool,

    #[serde(default)]
    pub extra_sync_paths: Vec<String>,

    #[serde(default)]
    pub exclusions: Vec<String>,

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

Add the `modsync` field to the `Config` struct:

```rust
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modsync: Option<ModSyncConfig>,
```

Add detection function near `is_fika_installed`:

```rust
pub fn is_modsync_installed(spt_dir: &Path) -> bool {
    spt_dir.join("user/mods/Corter-ModSync").is_dir()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p quartermaster modsync_config -- --nocapture && cargo test -p quartermaster modsync_detection -- --nocapture`

Expected: all 6 new tests pass.

- [ ] **Step 5: Run lint**

Run: `just lint`

Expected: no warnings or errors.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat(modsync): add config types and detection function"
```

---

### Task 2: Core ModSync Module — Config Generation

Create `src/modsync.rs` with the config generation logic: building syncPaths from DB state, applying overrides, writing config.jsonc atomically.

**Files:**
- Create: `src/modsync.rs`
- Modify: `src/main.rs` (add `mod modsync;`)

**Interfaces:**
- Consumes: `Config.modsync: Option<ModSyncConfig>`, `Database::list_mods()`, `Database::get_files_for_mod()`, `is_modsync_installed()`
- Produces: `modsync::regenerate_if_enabled(spt_dir: &Path, config: &Config, db: &Database) -> Result<bool>`, `modsync::generate_config(config: &ModSyncConfig, db: &Database) -> Result<ModSyncOutput>`, `modsync::modsync_config_path(spt_dir: &Path) -> PathBuf`

- [ ] **Step 1: Write tests for config generation**

Create `src/modsync.rs` with tests at the bottom:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::config::{is_modsync_installed, Config, ModSyncConfig, ModSyncOverride};
use crate::db::Database;

/// A single syncPath entry in ModSync's config.jsonc.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncPathEntry {
    path: String,
    name: String,
    enabled: bool,
    enforced: bool,
    silent: bool,
    restart_required: bool,
}

/// The full ModSync config.jsonc structure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModSyncOutputConfig {
    sync_paths: Vec<SyncPathEntry>,
    exclusions: Vec<String>,
}

pub fn modsync_config_path(spt_dir: &Path) -> PathBuf {
    spt_dir.join("user/mods/Corter-ModSync/config.jsonc")
}

/// Generate ModSync config from DB state + quartermaster config.
fn generate_config(ms_config: &ModSyncConfig, db: &Database) -> Result<ModSyncOutputConfig> {
    todo!()
}

/// Write a ModSyncOutputConfig to config.jsonc atomically.
fn write_config(config_path: &Path, output: &ModSyncOutputConfig) -> Result<()> {
    todo!()
}

/// Regenerate config.jsonc if ModSync is installed and [modsync] config is present.
/// Returns true if the config was written, false if skipped.
pub fn regenerate_if_enabled(spt_dir: &Path, config: &Config, db: &Database) -> Result<bool> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModSyncConfig;
    use crate::db::Database;

    fn setup_db_with_client_mod(db: &Database) -> i64 {
        let mod_id = db
            .insert_mod(100, 200, "SAIN", Some("sain"), "3.2.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/SAIN.dll",
            Some("abc123"),
            Some(1024),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/SAIN/config.json",
            Some("def456"),
            Some(256),
        )
        .unwrap();
        mod_id
    }

    fn setup_db_with_server_mod(db: &Database) -> i64 {
        let mod_id = db
            .insert_mod(200, 300, "ServerMod", Some("server-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/ServerMod/package.json",
            Some("aaa111"),
            Some(512),
        )
        .unwrap();
        mod_id
    }

    fn setup_db_with_hybrid_mod(db: &Database) -> i64 {
        let mod_id = db
            .insert_mod(300, 400, "HybridMod", Some("hybrid-mod"), "2.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/HybridMod/hybrid.dll",
            Some("bbb222"),
            Some(2048),
        )
        .unwrap();
        db.insert_file(
            mod_id,
            "SPT/user/mods/HybridMod/package.json",
            Some("ccc333"),
            Some(128),
        )
        .unwrap();
        mod_id
    }

    #[test]
    fn generate_config_client_mod_creates_sync_path() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins/SAIN");
        assert_eq!(output.sync_paths[0].name, "SAIN");
        assert!(output.sync_paths[0].enforced);   // global default
        assert!(!output.sync_paths[0].silent);     // global default
        assert!(output.sync_paths[0].restart_required);
        assert!(output.sync_paths[0].enabled);
    }

    #[test]
    fn generate_config_server_only_mod_excluded() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_server_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db).unwrap();

        assert!(output.sync_paths.is_empty());
    }

    #[test]
    fn generate_config_hybrid_mod_only_syncs_client_paths() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_hybrid_mod(&db);

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins/HybridMod");
        assert_eq!(output.sync_paths[0].name, "HybridMod");
    }

    #[test]
    fn generate_config_per_mod_override_applied() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db); // forge_mod_id = 100

        let mut ms_config = ModSyncConfig::default();
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(false),
                silent: Some(true),
                restart_required: None,
                enabled: None,
            },
        );

        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert!(!output.sync_paths[0].enforced); // overridden
        assert!(output.sync_paths[0].silent);    // overridden
        assert!(output.sync_paths[0].restart_required); // global default (not overridden)
    }

    #[test]
    fn generate_config_disabled_mod_override() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db); // forge_mod_id = 100

        let mut ms_config = ModSyncConfig::default();
        ms_config.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
            },
        );

        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert!(!output.sync_paths[0].enabled);
    }

    #[test]
    fn generate_config_extra_sync_paths_included() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.extra_sync_paths = vec!["BepInEx/config".to_string()];

        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        assert_eq!(output.sync_paths[0].path, "BepInEx/config");
        assert_eq!(output.sync_paths[0].name, "BepInEx/config");
        assert!(output.sync_paths[0].enforced); // uses global defaults
    }

    #[test]
    fn generate_config_exclusions_passed_through() {
        let db = Database::open_in_memory().unwrap();

        let mut ms_config = ModSyncConfig::default();
        ms_config.exclusions = vec!["**/*.nosync".to_string(), "BepInEx/plugins/spt".to_string()];

        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.exclusions, vec!["**/*.nosync", "BepInEx/plugins/spt"]);
    }

    #[test]
    fn generate_config_multiple_mods_sorted() {
        let db = Database::open_in_memory().unwrap();
        // Insert two client mods
        let mod1 = db
            .insert_mod(100, 200, "Zebra", Some("zebra"), "1.0.0")
            .unwrap();
        db.insert_file(mod1, "BepInEx/plugins/Zebra/z.dll", Some("aaa"), Some(100))
            .unwrap();

        let mod2 = db
            .insert_mod(101, 201, "Alpha", Some("alpha"), "1.0.0")
            .unwrap();
        db.insert_file(mod2, "BepInEx/plugins/Alpha/a.dll", Some("bbb"), Some(100))
            .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 2);
        // Should be sorted by path for deterministic output
        assert_eq!(output.sync_paths[0].path, "BepInEx/plugins/Alpha");
        assert_eq!(output.sync_paths[1].path, "BepInEx/plugins/Zebra");
    }

    #[test]
    fn generate_config_global_defaults_applied() {
        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let ms_config = ModSyncConfig {
            enforced: false,
            silent: true,
            restart_required: false,
            ..ModSyncConfig::default()
        };
        let output = generate_config(&ms_config, &db).unwrap();

        assert!(!output.sync_paths[0].enforced);
        assert!(output.sync_paths[0].silent);
        assert!(!output.sync_paths[0].restart_required);
    }

    #[test]
    fn generate_config_mod_with_patcher_files() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(500, 600, "PatcherMod", Some("patcher-mod"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/patchers/PatcherMod.dll",
            Some("ppp111"),
            Some(512),
        )
        .unwrap();

        let ms_config = ModSyncConfig::default();
        let output = generate_config(&ms_config, &db).unwrap();

        assert_eq!(output.sync_paths.len(), 1);
        // Single file patcher — sync path is the file itself
        assert_eq!(output.sync_paths[0].path, "BepInEx/patchers/PatcherMod.dll");
    }

    #[test]
    fn write_config_creates_jsonc_with_header() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.jsonc");

        let output = ModSyncOutputConfig {
            sync_paths: vec![SyncPathEntry {
                path: "BepInEx/plugins/Test".to_string(),
                name: "Test".to_string(),
                enabled: true,
                enforced: true,
                silent: false,
                restart_required: true,
            }],
            exclusions: vec!["**/*.nosync".to_string()],
        };

        write_config(&config_path, &output).unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.starts_with("// Generated by quartermaster"));
        // Should be valid JSON after removing the comment line
        let json_part: String = content.lines().skip(1).collect::<Vec<_>>().join("\n");
        let parsed: serde_json::Value = serde_json::from_str(&json_part).unwrap();
        assert!(parsed["syncPaths"].is_array());
        assert_eq!(parsed["syncPaths"][0]["path"], "BepInEx/plugins/Test");
        assert_eq!(parsed["syncPaths"][0]["name"], "Test");
    }

    #[test]
    fn regenerate_if_enabled_skips_when_no_modsync_config() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let config = Config::default(); // modsync: None

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(!result);
    }

    #[test]
    fn regenerate_if_enabled_skips_when_modsync_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());
        // Don't create the Corter-ModSync directory

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(!result);
    }

    #[test]
    fn regenerate_if_enabled_writes_when_configured_and_installed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("user/mods/Corter-ModSync")).unwrap();

        let db = Database::open_in_memory().unwrap();
        setup_db_with_client_mod(&db);

        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());

        let result = regenerate_if_enabled(tmp.path(), &config, &db).unwrap();
        assert!(result);

        let config_path = modsync_config_path(tmp.path());
        assert!(config_path.exists());
    }
}
```

- [ ] **Step 2: Register the module**

Add to `src/main.rs` (near the other `mod` declarations):

```rust
mod modsync;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p quartermaster modsync::tests -- --nocapture 2>&1 | head -30`

Expected: tests fail because `generate_config`, `write_config`, and `regenerate_if_enabled` are all `todo!()`.

- [ ] **Step 4: Implement generate_config**

Replace the `todo!()` in `generate_config`:

```rust
fn generate_config(ms_config: &ModSyncConfig, db: &Database) -> Result<ModSyncOutputConfig> {
    let mods = db.list_mods()?;
    let mut sync_paths: Vec<SyncPathEntry> = Vec::new();

    for m in &mods {
        let files = db.get_files_for_mod(m.id)?;
        let client_files: Vec<&str> = files
            .iter()
            .filter(|f| f.file_path.starts_with("BepInEx/"))
            .map(|f| f.file_path.as_str())
            .collect();

        if client_files.is_empty() {
            continue;
        }

        let forge_id_str = m.forge_mod_id.to_string();
        let overrides = ms_config.overrides.get(&forge_id_str);

        for dir in deduplicate_to_directories(&client_files) {
            sync_paths.push(SyncPathEntry {
                path: dir.clone(),
                name: m.name.clone(),
                enabled: overrides.and_then(|o| o.enabled).unwrap_or(true),
                enforced: overrides.and_then(|o| o.enforced).unwrap_or(ms_config.enforced),
                silent: overrides.and_then(|o| o.silent).unwrap_or(ms_config.silent),
                restart_required: overrides
                    .and_then(|o| o.restart_required)
                    .unwrap_or(ms_config.restart_required),
            });
        }
    }

    for extra in &ms_config.extra_sync_paths {
        sync_paths.push(SyncPathEntry {
            path: extra.clone(),
            name: extra.clone(),
            enabled: true,
            enforced: ms_config.enforced,
            silent: ms_config.silent,
            restart_required: ms_config.restart_required,
        });
    }

    sync_paths.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(ModSyncOutputConfig {
        sync_paths,
        exclusions: ms_config.exclusions.clone(),
    })
}

/// Given a list of file paths under BepInEx/, deduplicate to the minimal
/// set of directories. If multiple files share a common parent like
/// `BepInEx/plugins/SAIN/`, use the directory. If a file is alone at a
/// level (e.g., `BepInEx/patchers/Mod.dll`), use the file path directly.
fn deduplicate_to_directories(paths: &[&str]) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut dirs: BTreeSet<String> = BTreeSet::new();

    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        // BepInEx/<category>/<mod_name_or_file>/...
        // If 3+ parts and the 3rd part has no extension, treat first 3 as directory
        // If 3 parts and the last has an extension, it's a standalone file
        if parts.len() >= 3 {
            let third = parts[2];
            if parts.len() == 3 && third.contains('.') {
                // Standalone file like BepInEx/patchers/Mod.dll
                dirs.insert(path.to_string());
            } else {
                // Directory like BepInEx/plugins/SAIN
                dirs.insert(format!("{}/{}/{}", parts[0], parts[1], third));
            }
        }
    }

    dirs.into_iter().collect()
}
```

- [ ] **Step 5: Implement write_config**

Replace the `todo!()` in `write_config`:

```rust
fn write_config(config_path: &Path, output: &ModSyncOutputConfig) -> Result<()> {
    let json = serde_json::to_string_pretty(output)?;
    let content = format!(
        "// Generated by quartermaster \u{2014} manual edits will be overwritten\n{json}\n"
    );

    let tmp_path = config_path.with_extension("jsonc.tmp");
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, config_path)?;

    tracing::debug!(path = %config_path.display(), "wrote ModSync config");
    Ok(())
}
```

- [ ] **Step 6: Implement regenerate_if_enabled**

Replace the `todo!()` in `regenerate_if_enabled`:

```rust
pub fn regenerate_if_enabled(spt_dir: &Path, config: &Config, db: &Database) -> Result<bool> {
    let ms_config = match &config.modsync {
        Some(c) => c,
        None => return Ok(false),
    };

    if !is_modsync_installed(spt_dir) {
        tracing::debug!("ModSync not installed, skipping config generation");
        return Ok(false);
    }

    let output = generate_config(ms_config, db)?;
    let config_path = modsync_config_path(spt_dir);
    write_config(&config_path, &output)?;

    Ok(true)
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p quartermaster modsync::tests -- --nocapture`

Expected: all tests pass.

- [ ] **Step 8: Run lint**

Run: `just lint`

Expected: no warnings or errors.

- [ ] **Step 9: Commit**

```bash
git add src/modsync.rs src/main.rs
git commit -m "feat(modsync): add config generation module"
```

---

### Task 3: Hook ops.rs and Queue Drain

Wire `modsync::regenerate_if_enabled()` into the mod operation functions and the queue drain handler so config.jsonc is regenerated after every mod change.

**Files:**
- Modify: `src/ops.rs` (add `config: &Config` parameter to install/update/remove, call regenerate)
- Modify: `src/web/handlers/queue.rs` (call regenerate after successful queue drain)
- Modify: All callers of `install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id` (add config param)

**Interfaces:**
- Consumes: `modsync::regenerate_if_enabled(spt_dir, config, db)`
- Produces: Updated function signatures for `install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id` that accept `config: &Config`

- [ ] **Step 1: Update ops.rs function signatures and add regeneration calls**

Add `config` parameter to all three functions in `src/ops.rs` and call `regenerate_if_enabled` at the end of each:

Update `install_mod_from_archive`:
```rust
pub fn install_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    forge_mod_id: i64,
    version_id: i64,
    name: &str,
    slug: Option<&str>,
    version: &str,
    archive_path: &Path,
) -> Result<i64> {
    tracing::info!(name, forge_mod_id, version, "installing mod from archive");
    let extracted = crate::spt::mods::extract_mod(archive_path, spt_dir)?;
    let db_id = db.insert_mod(forge_mod_id, version_id, name, slug, version)?;
    record_extracted_files(db, db_id, &extracted)?;
    tracing::debug!(
        db_id,
        file_count = extracted.len(),
        "mod installed, files recorded"
    );
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(error = %e, "failed to regenerate ModSync config");
    }
    Ok(db_id)
}
```

Update `update_mod_from_archive`:
```rust
pub fn update_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
    version_id: i64,
    version_str: &str,
    archive_path: &Path,
) -> Result<()> {
    // ... existing body unchanged until the end ...
    record_extracted_files(db, mod_db_id, &extracted)?;
    db.update_mod(mod_db_id, version_id, version_str)?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(error = %e, "failed to regenerate ModSync config");
    }
    Ok(())
}
```

Update `remove_mod_by_id`:
```rust
pub fn remove_mod_by_id(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
) -> Result<()> {
    tracing::info!(mod_db_id, "removing mod");
    let files = db.get_files_for_mod(mod_db_id)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(file_count = paths.len(), "deleting mod files");
    crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
    db.delete_mod(mod_db_id)?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(error = %e, "failed to regenerate ModSync config");
    }
    Ok(())
}
```

- [ ] **Step 2: Fix all callers**

Search for all call sites with `rg "install_mod_from_archive\|update_mod_from_archive\|remove_mod_by_id" src/ --no-filename` and update each one to pass `config` (or `&state.config`).

Key locations:
- `src/cli/install.rs` — CLI install command, has `ctx.config` available
- `src/cli/update.rs` — CLI update command
- `src/cli/remove.rs` — CLI remove command
- `src/cli/apply.rs` — CLI queue drain
- `src/web/handlers/mods.rs` — web install/update/remove handlers (use `&state.config` or clone into the spawned task)
- `src/web/handlers/queue.rs` — web queue drain (apply_install, apply_update, apply_remove)
- `src/ops.rs` tests — pass `&Config::default()`

For each call site, add the `config` parameter. The web handlers that run ops in a `web::block` closure will need to clone the config or pass a reference. Since `Config` is `Clone`, clone it into the closure.

- [ ] **Step 3: Add regeneration after queue drain in web handler**

In `src/web/handlers/queue.rs`, after the successful drain loop in `apply_queue`, add a ModSync regeneration call:

```rust
    // After the for loop over ops, before the success flash:
    // Regenerate ModSync config after all queue operations
    {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let config = state.config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db)
        })
        .await;
    }
```

- [ ] **Step 4: Fix ops.rs tests**

Update all tests in `src/ops.rs` to pass `&Config::default()`:

```rust
// Example — update all three test functions similarly:
let db_id = install_mod_from_archive(
    &db,
    spt_dir.path(),
    &Config::default(),
    100,
    200,
    "TestMod",
    Some("test-mod"),
    "1.0.0",
    zip.path(),
)
.unwrap();
```

Add `use crate::config::Config;` to the test module.

- [ ] **Step 5: Build and test**

Run: `cargo test -p quartermaster && just lint`

Expected: all tests pass, no lint errors.

- [ ] **Step 6: Commit**

```bash
git add src/ops.rs src/web/handlers/queue.rs src/cli/ src/web/handlers/mods.rs
git commit -m "feat(modsync): hook regeneration into mod operations and queue drain"
```

---

### Task 4: AppState and Server Startup Integration

Add `modsync_installed` to `AppState`, set it at startup, pass it through templates, and regenerate config on startup.

**Files:**
- Modify: `src/web/state.rs` (add `modsync_installed: bool`)
- Modify: `src/web/mod.rs` (`start_server` — accept and set `modsync_installed`, regenerate on startup)
- Modify: `src/cli/serve.rs` (detect ModSync, pass to `start_server`)
- Modify: `src/web/handlers/dashboard.rs` (add `modsync_installed` to template)
- Modify: `templates/dashboard.html` (add ModSync badge)
- Modify: All templates and handlers that pass `fika_installed` — also pass `modsync_installed` (for nav/badge consistency). This includes: `dashboard.rs`, `mods.rs`, `queue.rs`, `status.rs`, `logs.rs`, `admin.rs`, `clients.rs`, `requests.rs`
- Modify: `templates/partials/nav.html` (accept `modsync_installed` parameter)

**Interfaces:**
- Consumes: `is_modsync_installed()`, `modsync::regenerate_if_enabled()`
- Produces: `AppState.modsync_installed: bool`, ModSync badge on dashboard

- [ ] **Step 1: Add `modsync_installed` to AppState**

Since ModSync can be installed/removed at runtime via the web UI, use `AtomicBool` (like `converging`) so it can be updated without mutable access to `AppState`:

In `src/web/state.rs`:

```rust
pub struct AppState {
    // ... existing fields ...
    pub modsync_installed: AtomicBool,
    // ... rest ...
}
```

Add a helper method:
```rust
impl AppState {
    pub fn is_modsync_installed(&self) -> bool {
        self.modsync_installed.load(std::sync::atomic::Ordering::Relaxed)
    }
}
```

Update all reads of `state.modsync_installed` to `state.is_modsync_installed()` and the initialization to `AtomicBool::new(modsync_installed)`.

- [ ] **Step 2: Update `start_server` to accept and use `modsync_installed`**

In `src/web/mod.rs`, add `modsync_installed: bool` parameter to `start_server` and set it on `AppState`. Also regenerate config on startup:

```rust
pub async fn start_server(
    config: Config,
    config_path: std::path::PathBuf,
    db: Database,
    forge: ForgeClient,
    spt_dir: std::path::PathBuf,
    spt_info: SptInfo,
    log_broadcast: Arc<LogBroadcast>,
    container_mgr: Option<Arc<crate::container::ContainerManager>>,
    client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    converging: Arc<std::sync::atomic::AtomicBool>,
    fika_installed: bool,
    modsync_installed: bool,
) -> Result<()> {
```

Before creating `AppState`, regenerate ModSync config:

```rust
    // Regenerate ModSync config on startup to ensure consistency
    if modsync_installed && config.modsync.is_some() {
        if let Err(e) = crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db) {
            tracing::warn!(error = %e, "failed to regenerate ModSync config on startup");
        }
    }
```

Add `modsync_installed` to the `AppState` construction.

- [ ] **Step 3: Update `serve.rs` to detect and pass ModSync**

In `src/cli/serve.rs`, after the `fika_installed` line:

```rust
    let fika_installed = is_fika_installed(&spt_dir);
    let modsync_installed = crate::config::is_modsync_installed(&spt_dir);
```

Pass `modsync_installed` to `start_server`.

- [ ] **Step 4: Add ModSync badge to dashboard**

Update `DashboardTemplate` in `src/web/handlers/dashboard.rs`:

```rust
struct DashboardTemplate {
    // ... existing fields ...
    modsync_installed: bool,
    modsync_managed: bool,
}
```

Set in the handler:
```rust
    let tmpl = DashboardTemplate {
        // ... existing fields ...
        modsync_installed: state.modsync_installed,
        modsync_managed: state.modsync_installed && state.config.modsync.is_some(),
    };
```

Add to `templates/dashboard.html` inside the `stat-card-grid` div, after the Server card:

```html
    {% if modsync_installed %}
    <div class="stat-card" style="border-left-color: {% if modsync_managed %}var(--success){% else %}var(--warning){% endif %}">
        <div class="stat-label">ModSync</div>
        <div class="stat-value text-sm">{% if modsync_managed %}Active{% else %}Not Managed{% endif %}</div>
        <div class="stat-detail">{% if modsync_managed %}config auto-generated{% else %}enable in config{% endif %}</div>
    </div>
    {% endif %}
```

- [ ] **Step 5: Update nav and all templates to accept `modsync_installed`**

Update the nav macro signature in `templates/partials/nav.html` to accept `modsync_installed`:

```html
{% macro nav(active, user, csrf_token, fika_installed, modsync_installed) %}
```

Then update every template and handler that calls the nav macro to pass `modsync_installed`. This is a mechanical change across all template structs and handler functions — add `modsync_installed: bool` to every template struct that has `fika_installed`, and set it from `state.modsync_installed` in every handler.

- [ ] **Step 6: Build and test**

Run: `cargo build && just lint`

Expected: compiles with no errors. Askama will catch any template/struct mismatches at compile time.

- [ ] **Step 7: Commit**

```bash
git add src/web/state.rs src/web/mod.rs src/cli/serve.rs src/web/handlers/ templates/
git commit -m "feat(modsync): add AppState tracking, startup regeneration, and dashboard badge"
```

---

### Task 5: ModSync Settings Web Page

Add a web UI settings page for ModSync configuration: global defaults, extra sync paths, exclusions, and per-mod overrides.

**Files:**
- Create: `src/web/handlers/modsync.rs`
- Create: `templates/modsync.html`
- Modify: `src/web/handlers/mod.rs` (add `pub mod modsync;`)
- Modify: `src/web/mod.rs` (add routes)
- Modify: `templates/partials/nav.html` (add ModSync nav link when installed)

**Interfaces:**
- Consumes: `AppState.modsync_installed`, `Config.modsync`, `Database::list_mods()`, `Database::get_files_for_mod()`, `modsync::regenerate_if_enabled()`
- Produces: `GET /modsync` (settings page), `POST /modsync/settings` (save settings)

- [ ] **Step 1: Create the handler**

Create `src/web/handlers/modsync.rs`:

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Form, Html};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::mods::InstalledMod;
use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage};
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

struct ModSyncModEntry {
    mod_info: InstalledMod,
    has_client_files: bool,
    override_enforced: Option<bool>,
    override_silent: Option<bool>,
    override_restart_required: Option<bool>,
    override_enabled: Option<bool>,
}

#[derive(Template)]
#[template(path = "modsync.html")]
struct ModSyncTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    fika_installed: bool,
    modsync_installed: bool,
    modsync_managed: bool,
    enforced: bool,
    silent: bool,
    restart_required: bool,
    extra_sync_paths: String,
    exclusions: String,
    mods: Vec<ModSyncModEntry>,
}

pub async fn modsync_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let ms_config = state.config.modsync.clone();

    let db = state.db.clone();
    let mods = web::block(move || {
        let db = db.lock();
        let all_mods = db.list_mods()?;
        let mut entries = Vec::new();
        for m in all_mods {
            let files = db.get_files_for_mod(m.id)?;
            let has_client = files.iter().any(|f| f.file_path.starts_with("BepInEx/"));
            let forge_id_str = m.forge_mod_id.to_string();
            let overrides = ms_config
                .as_ref()
                .and_then(|c| c.overrides.get(&forge_id_str));
            entries.push(ModSyncModEntry {
                mod_info: m,
                has_client_files: has_client,
                override_enforced: overrides.and_then(|o| o.enforced),
                override_silent: overrides.and_then(|o| o.silent),
                override_restart_required: overrides.and_then(|o| o.restart_required),
                override_enabled: overrides.and_then(|o| o.enabled),
            });
        }
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (enforced, silent, restart_required, extra_sync_paths, exclusions) =
        if let Some(ref ms) = state.config.modsync {
            (
                ms.enforced,
                ms.silent,
                ms.restart_required,
                ms.extra_sync_paths.join("\n"),
                ms.exclusions.join("\n"),
            )
        } else {
            (true, false, true, String::new(), String::new())
        };

    let tmpl = ModSyncTemplate {
        user,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        modsync_installed: state.modsync_installed,
        modsync_managed: state.modsync_installed && state.config.modsync.is_some(),
        enforced,
        silent,
        restart_required,
        extra_sync_paths,
        exclusions,
        mods,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct ModSyncSettingsForm {
    csrf_token: String,
    enforced: Option<String>,
    silent: Option<String>,
    restart_required: Option<String>,
    extra_sync_paths: String,
    exclusions: String,
}

pub async fn save_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ModSyncSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let extra_paths: Vec<String> = form
        .extra_sync_paths
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let exclusion_list: Vec<String> = form
        .exclusions
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    // Preserve existing overrides
    let existing_overrides = state
        .config
        .modsync
        .as_ref()
        .map(|ms| ms.overrides.clone())
        .unwrap_or_default();

    let ms_config = crate::config::ModSyncConfig {
        enforced: form.enforced.is_some(),
        silent: form.silent.is_some(),
        restart_required: form.restart_required.is_some(),
        extra_sync_paths: extra_paths,
        exclusions: exclusion_list,
        overrides: existing_overrides,
    };

    // Update config and save
    let mut new_config = state.config.clone();
    new_config.modsync = Some(ms_config);
    new_config
        .save(&state.config_path)
        .map_err(WebError::from)?;

    // Regenerate ModSync config.jsonc
    if state.modsync_installed {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let config = new_config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db)
        })
        .await;
    }

    set_flash(&session, "ModSync settings saved", "success");
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/modsync"))
        .finish())
}
```

- [ ] **Step 2: Register handler module**

Add to `src/web/handlers/mod.rs`:

```rust
pub mod modsync;
```

- [ ] **Step 3: Add routes**

In `src/web/mod.rs`, add routes inside the authenticated scope (the `web::scope("")` block):

```rust
    .route("/modsync", web::get().to(handlers::modsync::modsync_page))
    .route("/modsync/settings", web::post().to(handlers::modsync::save_settings))
```

- [ ] **Step 4: Create the template**

Create `templates/modsync.html`:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% block title %}ModSync — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("modsync", user, csrf_token, fika_installed, modsync_installed) %}{% endcall %}{% endblock %}
{% block content %}
<h1>ModSync Settings</h1>

{% if !modsync_installed %}
<div class="card" style="border-left: 3px solid var(--warning)">
    <p><strong>ModSync is not installed.</strong> Install it from Forge to enable mod synchronization.</p>
</div>
{% else %}

{% if !modsync_managed %}
<div class="card" style="border-left: 3px solid var(--warning)">
    <p><strong>ModSync is installed but not managed by Quartermaster.</strong> Save settings below to enable automatic config generation.</p>
    <p class="text-muted text-sm mt-1">Note: enabling management will overwrite the existing <code>config.jsonc</code>. The default <code>enforced = true</code> is stricter than ModSync's default — all synced mods will require clients to match the server exactly.</p>
</div>
{% endif %}

<form method="post" action="/modsync/settings">
    <input type="hidden" name="csrf_token" value="{{ csrf_token }}">

    <div class="card">
        <h2>Global Defaults</h2>
        <p class="text-muted text-sm mb-1">Applied to all synced mods unless overridden per-mod.</p>
        <div class="form-group">
            <label><input type="checkbox" name="enforced" {% if enforced %}checked{% endif %}> <strong>Enforced</strong> — force clients to match server files exactly</label>
        </div>
        <div class="form-group">
            <label><input type="checkbox" name="silent" {% if silent %}checked{% endif %}> <strong>Silent</strong> — auto-apply updates without prompting</label>
        </div>
        <div class="form-group">
            <label><input type="checkbox" name="restart_required" {% if restart_required %}checked{% endif %}> <strong>Restart Required</strong> — require game restart after sync</label>
        </div>
    </div>

    <div class="card">
        <h2>Extra Sync Paths</h2>
        <p class="text-muted text-sm mb-1">Additional paths to sync beyond auto-detected mod files. One per line.</p>
        <textarea name="extra_sync_paths" rows="4" class="form-control" placeholder="BepInEx/config">{{ extra_sync_paths }}</textarea>
    </div>

    <div class="card">
        <h2>Exclusions</h2>
        <p class="text-muted text-sm mb-1">Glob patterns for files to exclude from sync. One per line.</p>
        <textarea name="exclusions" rows="4" class="form-control" placeholder="**/*.nosync">{{ exclusions }}</textarea>
    </div>

    <button type="submit" class="btn">Save Settings</button>
</form>

{% if modsync_managed %}
<div class="card mt-2">
    <h2>Per-Mod Sync Status</h2>
    <p class="text-muted text-sm mb-1">Mods with client-side files that will be synced to players.</p>
    {% let client_mods: Vec<&ModSyncModEntry> = mods.iter().filter(|m| m.has_client_files).collect() %}
    {% if client_mods.is_empty() %}
    <p class="text-muted">No client-side mods installed.</p>
    {% else %}
    <table>
        <thead>
            <tr>
                <th>Mod</th>
                <th>Enforced</th>
                <th>Silent</th>
                <th>Enabled</th>
            </tr>
        </thead>
        <tbody>
            {% for entry in mods.iter().filter(|m| m.has_client_files) %}
            <tr>
                <td><a href="/mods/{{ entry.mod_info.id }}">{{ entry.mod_info.name }}</a></td>
                <td>{% match entry.override_enforced %}{% when Some with (v) %}{{ v }}{% when None %}default{% endmatch %}</td>
                <td>{% match entry.override_silent %}{% when Some with (v) %}{{ v }}{% when None %}default{% endmatch %}</td>
                <td>{% match entry.override_enabled %}{% when Some with (false) %}<span style="color: var(--danger)">disabled</span>{% when _ %}yes{% endmatch %}</td>
            </tr>
            {% endfor %}
        </tbody>
    </table>
    {% endif %}
</div>
{% endif %}

{% endif %}
{% endblock %}
```

- [ ] **Step 5: Add nav link**

In `templates/partials/nav.html`, add after the Status link:

```html
    {% if modsync_installed %}
    <a href="/modsync"{% if active == "modsync" %} class="active"{% endif %}>{% call icons::refresh() %}{% endcall %} ModSync</a>
    {% endif %}
```

- [ ] **Step 6: Build and test**

Run: `cargo build && just lint`

Expected: compiles with no errors. Askama validates templates at compile time.

- [ ] **Step 7: Commit**

```bash
git add src/web/handlers/modsync.rs src/web/handlers/mod.rs src/web/mod.rs templates/modsync.html templates/partials/nav.html
git commit -m "feat(modsync): add web UI settings page"
```

---

### Task 6: Mod Detail Sync Info and AppState Updates on Install/Remove

Show ModSync sync settings on individual mod detail pages, and update the `modsync_installed` AtomicBool on `AppState` after mod install/remove in the web handlers (since installing or removing ModSync itself changes detection state).

**Files:**
- Modify: `src/web/handlers/mods.rs` (add sync info to detail template, update `modsync_installed` after install/remove tasks complete)
- Modify: `templates/mods/detail.html` (show sync settings section)

**Interfaces:**
- Consumes: `AppState.is_modsync_installed()`, `Config.modsync`, `is_modsync_installed()`

- [ ] **Step 1: Add sync info to mod detail template struct**

In `src/web/handlers/mods.rs`, update `ModDetailTemplate`:

```rust
#[derive(Template)]
#[template(path = "mods/detail.html")]
struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    archive_files: Vec<InstalledFile>,
    runtime_files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
    flash: Option<FlashMessage>,
    csrf_token: String,
    fika_installed: bool,
    modsync_installed: bool,
    has_client_files: bool,
    sync_enforced: Option<bool>,
    sync_silent: Option<bool>,
    sync_restart_required: Option<bool>,
    sync_enabled: Option<bool>,
    modsync_managed: bool,
}
```

In the `mod_detail` handler, compute the sync fields:

```rust
    let has_client_files = archive_files.iter().any(|f| f.file_path.starts_with("BepInEx/"));
    let forge_id_str = mod_info.forge_mod_id.to_string();
    let overrides = state
        .config
        .modsync
        .as_ref()
        .and_then(|ms| ms.overrides.get(&forge_id_str));

    let tmpl = ModDetailTemplate {
        // ... existing fields ...
        modsync_installed: state.modsync_installed,
        has_client_files,
        sync_enforced: overrides.and_then(|o| o.enforced),
        sync_silent: overrides.and_then(|o| o.silent),
        sync_restart_required: overrides.and_then(|o| o.restart_required),
        sync_enabled: overrides.and_then(|o| o.enabled),
        modsync_managed: state.modsync_installed && state.config.modsync.is_some(),
    };
```

- [ ] **Step 2: Add sync section to detail template**

In `templates/mods/detail.html`, after the dependencies section and before the installed files section:

```html
{% if modsync_managed && has_client_files %}
<div class="card" style="border-left: 3px solid var(--success)">
    <h2>ModSync</h2>
    <p class="text-muted text-sm mb-1">This mod has client-side files that will be synced to players.</p>
    <table>
        <tr>
            <th style="width:140px">Enforced</th>
            <td>{% match sync_enforced %}{% when Some with (v) %}{{ v }} (override){% when None %}default{% endmatch %}</td>
        </tr>
        <tr>
            <th>Silent</th>
            <td>{% match sync_silent %}{% when Some with (v) %}{{ v }} (override){% when None %}default{% endmatch %}</td>
        </tr>
        <tr>
            <th>Restart Required</th>
            <td>{% match sync_restart_required %}{% when Some with (v) %}{{ v }} (override){% when None %}default{% endmatch %}</td>
        </tr>
        <tr>
            <th>Enabled</th>
            <td>{% match sync_enabled %}{% when Some with (false) %}<span style="color: var(--danger)">disabled</span>{% when _ %}yes{% endmatch %}</td>
        </tr>
    </table>
    <p class="text-muted text-sm mt-1"><a href="/modsync">Edit sync settings</a></p>
</div>
{% endif %}
```

- [ ] **Step 3: Update modsync_installed after install/remove tasks complete**

In `src/web/handlers/mods.rs`, in the `install_mod` handler's `tokio::spawn` block, after the successful branch (`Ok(()) =>`), add:

```rust
                // Re-check ModSync detection (installing/removing ModSync itself changes this)
                state.modsync_installed.store(
                    crate::config::is_modsync_installed(&spt_dir),
                    std::sync::atomic::Ordering::Relaxed,
                );
```

This requires cloning `state` (as `Data<AppState>`) into the spawned task. The `Data<AppState>` is already `Arc`-based, so cloning is cheap.

Apply the same pattern in the `remove_mod` handler's spawned task, and in the `update_all_mods` handler.

- [ ] **Step 4: Build and test**

Run: `cargo build && just lint`

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add src/web/handlers/mods.rs templates/mods/detail.html
git commit -m "feat(modsync): show sync settings on mod detail page, update detection on install/remove"
```

---

### Task 7: Integration Tests

Add integration tests that verify the full flow: install a mod, verify config.jsonc is generated correctly, update the mod, verify config changes, remove the mod, verify config updates.

**Files:**
- Modify: `src/modsync.rs` (add integration tests)

**Interfaces:**
- Consumes: all public APIs from previous tasks

- [ ] **Step 1: Write integration tests**

Add to the `#[cfg(test)] mod tests` block in `src/modsync.rs`:

```rust
    #[test]
    fn full_lifecycle_install_update_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        std::fs::create_dir_all(spt_dir.join("user/mods/Corter-ModSync")).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig::default());

        // Install a client mod
        let mod_id = db
            .insert_mod(100, 200, "TestClientMod", Some("test-client"), "1.0.0")
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/TestClientMod/test.dll",
            Some("hash1"),
            Some(1024),
        )
        .unwrap();

        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);

        let content = std::fs::read_to_string(modsync_config_path(spt_dir)).unwrap();
        assert!(content.contains("TestClientMod"));
        assert!(content.contains("BepInEx/plugins/TestClientMod"));

        // Update — add a second file (simulating version update)
        db.insert_file(
            mod_id,
            "BepInEx/plugins/TestClientMod/extra.dll",
            Some("hash2"),
            Some(512),
        )
        .unwrap();

        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);

        // Remove the mod
        db.delete_files_for_mod(mod_id).unwrap();
        db.delete_mod(mod_id).unwrap();

        let result = regenerate_if_enabled(spt_dir, &config, &db).unwrap();
        assert!(result);

        let content = std::fs::read_to_string(modsync_config_path(spt_dir)).unwrap();
        assert!(!content.contains("TestClientMod"));
    }

    #[test]
    fn full_lifecycle_mixed_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        std::fs::create_dir_all(spt_dir.join("user/mods/Corter-ModSync")).unwrap();

        let db = Database::open_in_memory().unwrap();
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig {
            extra_sync_paths: vec!["BepInEx/config".to_string()],
            exclusions: vec!["**/*.log".to_string()],
            ..ModSyncConfig::default()
        });

        // Server-only mod — should not appear in syncPaths
        let _server_mod = db
            .insert_mod(200, 300, "ServerOnly", None, "1.0.0")
            .unwrap();
        db.insert_file(
            _server_mod,
            "SPT/user/mods/ServerOnly/package.json",
            Some("s1"),
            Some(100),
        )
        .unwrap();

        // Client mod — should appear
        let client_mod = db
            .insert_mod(300, 400, "ClientMod", None, "1.0.0")
            .unwrap();
        db.insert_file(
            client_mod,
            "BepInEx/plugins/ClientMod/client.dll",
            Some("c1"),
            Some(200),
        )
        .unwrap();

        regenerate_if_enabled(spt_dir, &config, &db).unwrap();

        let content = std::fs::read_to_string(modsync_config_path(spt_dir)).unwrap();
        let json_part: String = content.lines().skip(1).collect::<Vec<_>>().join("\n");
        let parsed: serde_json::Value = serde_json::from_str(&json_part).unwrap();

        let paths: Vec<&str> = parsed["syncPaths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["path"].as_str().unwrap())
            .collect();

        assert!(paths.contains(&"BepInEx/plugins/ClientMod"));
        assert!(paths.contains(&"BepInEx/config"));
        assert!(!paths.iter().any(|p| p.contains("ServerOnly")));

        let exclusions: Vec<&str> = parsed["exclusions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap())
            .collect();
        assert_eq!(exclusions, vec!["**/*.log"]);
    }
```

- [ ] **Step 2: Run all tests**

Run: `cargo test -p quartermaster && just lint`

Expected: all tests pass, no lint errors.

- [ ] **Step 3: Commit**

```bash
git add src/modsync.rs
git commit -m "test(modsync): add full lifecycle integration tests"
```
