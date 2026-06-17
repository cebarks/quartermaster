# Phase 2: Core CLI Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the six core CLI commands (`init`, `install`, `remove`, `update`, `list`/`check`, `track`) that make `quma` a functional mod manager.

**Architecture:** Each command lives in its own file under `src/cli/`. Commands share a common bootstrap pattern: resolve SPT dir → load config → open DB → build Forge client. A shared `resolve_context` helper in `src/cli/common.rs` handles this. All commands apply changes directly to disk in Phase 2 — server-running detection and the change queue are Phase 3 concerns.

**Tech Stack:** Rust, clap (derive), rusqlite, reqwest, tokio, anyhow, serde_json, indicatif, tempfile, sha2

## Global Constraints

- **SPT 4.0+ only** — pre-4.0 installs are not supported
- **Linux only for v1** — no Windows path handling
- **No server-running detection in Phase 2** — install/update/remove always apply directly. The `--force` flag is accepted but has no effect yet (Phase 3 wires it to queue bypass)
- **All errors use `anyhow::Result<T>`** with context chains
- **Tests use `tempfile::TempDir`** for filesystem fixtures and `Database::open_in_memory()` for DB tests
- **Binary name:** `quma`
- **No new dependencies** — all needed crates are already in Cargo.toml

---

## File Map

| File | Responsibility | Task |
|------|---------------|------|
| `src/cli/common.rs` | Shared CLI context bootstrap (SPT dir + config + DB + Forge client) | 7 |
| `src/cli/init.rs` | `quma init` — create config, DB, scan existing mods | 7 |
| `src/cli/install.rs` | `quma install` — resolve, download, extract, record mods | 8 |
| `src/cli/remove.rs` | `quma remove` — delete tracked files, clean DB | 9 |
| `src/cli/update.rs` | `quma update` — check + apply updates | 10 |
| `src/cli/list.rs` | `quma list` — table/JSON output of installed mods | 11 |
| `src/cli/check.rs` | `quma check` — update availability report | 11 |
| `src/cli/track.rs` | `quma track` — associate unmanaged mod with Forge entry | 12 |
| `src/cli/mod.rs` | Wire up new submodules, add `mod common;` etc. | 7–12 |
| `src/main.rs` | Replace `todo!()` arms with command handler calls | 7–12 |
| `src/db/mods.rs` | Add `get_mod_by_name_or_slug()` lookup helper | 8 |

---

### Task 7: Init Command & Shared CLI Context

**Files:**
- Create: `src/cli/common.rs`
- Create: `src/cli/init.rs`
- Modify: `src/cli/mod.rs` (add `mod common; mod init;`)
- Modify: `src/main.rs` (wire `Command::Init` dispatch)

**Interfaces:**
- Consumes: `Config::save()`, `Config::resolve_path()`, `Config::ensure_session_secret()`, `Database::open()`, `spt::detect::validate_spt_dir()`, `spt::detect::read_spt_version()`, `spt::detect::detect_spt_dir()`, `spt::mods::scan_mod_directories()`, `Database::get_all_tracked_files()`, `Database::insert_user()`, `Database::admin_exists()`
- Produces:
  - `CliContext { spt_dir: PathBuf, spt_info: SptInfo, config: Config, config_path: PathBuf, db: Database, forge: ForgeClient }` — used by all subsequent commands
  - `cli::common::resolve_context(cli: &Cli) -> Result<CliContext>` — loads existing config+DB (for commands that require init)
  - `cli::init::run(path: Option<PathBuf>, cli: &Cli) -> Result<()>` — the init command handler

- [ ] **Step 1: Create `src/cli/common.rs` with `CliContext`**

```rust
// src/cli/common.rs
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::{detect_spt_dir, read_spt_version, SptInfo};

use super::Cli;

pub struct CliContext {
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub config: Config,
    pub config_path: PathBuf,
    pub db: Database,
    pub forge: ForgeClient,
}

/// Bootstrap the full CLI context for commands that require an initialized project.
///
/// Resolution order:
/// 1. Detect SPT dir (explicit flag > env > cwd walkup)
/// 2. Resolve config path, load config (with env overrides)
/// 3. Open database
/// 4. Build Forge client
pub fn resolve_context(cli: &Cli) -> Result<CliContext> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let spt_info = read_spt_version(&spt_dir)?;

    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let config = Config::load_with_env(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    let forge = ForgeClient::new(config.forge_token.clone())?;

    Ok(CliContext {
        spt_dir,
        spt_info,
        config,
        config_path,
        db,
        forge,
    })
}
```

- [ ] **Step 2: Create `src/cli/init.rs`**

```rust
// src/cli/init.rs
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::db::Database;
use crate::spt::detect::{detect_spt_dir, read_spt_version};
use crate::spt::mods::scan_mod_directories;

use super::Cli;

pub fn run(path: Option<PathBuf>, cli: &Cli) -> Result<()> {
    // 1. Resolve SPT directory
    let spt_dir = match path {
        Some(ref p) => {
            crate::spt::detect::validate_spt_dir(p)?;
            p.clone()
        }
        None => detect_spt_dir(cli.spt_dir.as_deref(), None)?,
    };

    let spt_info = read_spt_version(&spt_dir)?;
    println!(
        "Detected SPT {} (EFT {}) at {}",
        spt_info.spt_version,
        spt_info.tarkov_version,
        spt_dir.display()
    );

    // 2. Create config file
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = if config_path.exists() {
        println!("Config already exists at {}", config_path.display());
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.spt_dir = Some(spt_dir.clone());
    config.ensure_session_secret();
    config.save(&config_path)?;
    println!("Config saved to {}", config_path.display());

    // 3. Create database
    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    // 4. Scan for existing mods
    let all_files_on_disk = scan_mod_directories(&spt_dir)?;
    let tracked_files = db.get_all_tracked_files()?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let unmanaged: Vec<&str> = all_files_on_disk
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    if unmanaged.is_empty() {
        println!("No existing mod files found.");
    } else {
        // Group unmanaged files by their top-level mod directory
        let mut mod_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for path in &unmanaged {
            // Extract "user/mods/<name>" or "BepInEx/plugins/<name>"
            let parts: Vec<&str> = path.split('/').collect();
            if parts.len() >= 3 {
                let dir = format!("{}/{}/{}", parts[0], parts[1], parts[2]);
                mod_dirs.insert(dir);
            }
        }

        println!(
            "\nFound {} unmanaged mod director{} ({} files):",
            mod_dirs.len(),
            if mod_dirs.len() == 1 { "y" } else { "ies" },
            unmanaged.len()
        );
        for dir in &mod_dirs {
            println!("  {}", dir);
        }
        println!("\nUse `quma track <path> <forge_mod_id>` to associate them with Forge entries.");
    }

    // 5. Check for admin user
    if !db.admin_exists()? {
        println!("\nNo admin user exists. Create one with the web UI (`quma serve`) or during `quma setup`.");
    }

    println!("\nQuartermaster initialized successfully.");
    Ok(())
}
```

- [ ] **Step 3: Wire up in `src/cli/mod.rs` and `src/main.rs`**

Add to `src/cli/mod.rs` at the top:

```rust
pub mod common;
pub mod init;
```

Update `src/main.rs` to replace the `Command::Init` arm:

```rust
Command::Init { path } => cli::init::run(path, &cli),
```

- [ ] **Step 4: Write tests for `init` command**

Create a test in `src/cli/init.rs` that exercises the init flow against a fake SPT directory:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_fake_spt_dir(base: &std::path::Path) -> PathBuf {
        let spt_root = base.to_path_buf();
        std::fs::write(spt_root.join("SPT.Server.exe"), b"").unwrap();
        let configs_dir = spt_root.join("SPT_Data/Server/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("core.json"),
            r#"{"sptVersion": "4.0.13", "compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(spt_root.join("user/mods")).unwrap();
        std::fs::create_dir_all(spt_root.join("BepInEx/plugins")).unwrap();
        spt_root
    }

    #[test]
    fn init_creates_config_and_db() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = create_fake_spt_dir(tmp.path());

        let cli = Cli {
            spt_dir: None,
            config: None,
            command: crate::cli::Command::Init { path: None },
        };

        run(Some(spt_dir.clone()), &cli).unwrap();

        // Config file should exist
        let config_path = spt_dir.join("quartermaster.toml");
        assert!(config_path.exists(), "config file should be created");

        // DB file should exist
        let db_path = spt_dir.join("quartermaster.db");
        assert!(db_path.exists(), "database should be created");

        // Config should have spt_dir set and a session secret
        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.spt_dir, Some(spt_dir));
        assert!(!config.session_secret.is_empty());
    }

    #[test]
    fn init_detects_unmanaged_mods() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = create_fake_spt_dir(tmp.path());

        // Create some existing mod files
        let mod_dir = spt_dir.join("user/mods/SomeMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();

        let cli = Cli {
            spt_dir: None,
            config: None,
            command: crate::cli::Command::Init { path: None },
        };

        // Should succeed and report unmanaged files (output goes to stdout)
        run(Some(spt_dir), &cli).unwrap();
    }

    #[test]
    fn init_idempotent_with_existing_config() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = create_fake_spt_dir(tmp.path());

        let cli = Cli {
            spt_dir: None,
            config: None,
            command: crate::cli::Command::Init { path: None },
        };

        // Run init twice
        run(Some(spt_dir.clone()), &cli).unwrap();
        run(Some(spt_dir), &cli).unwrap();
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -- cli::init
```

Expected: All 3 tests pass.

- [ ] **Step 6: Run clippy and verify the full test suite**

```bash
cargo clippy -- -D warnings && cargo test
```

Expected: No warnings, all tests pass (59 existing + 3 new = 62).

- [ ] **Step 7: Commit**

```bash
git add src/cli/common.rs src/cli/init.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add init command and shared CLI context bootstrap"
```

---

### Task 8: Install Command

**Files:**
- Create: `src/cli/install.rs`
- Modify: `src/cli/mod.rs` (add `mod install;`)
- Modify: `src/main.rs` (wire `Command::Install` dispatch)
- Modify: `src/db/mods.rs` (add `get_mod_by_name_or_slug()`)

**Interfaces:**
- Consumes: `CliContext` (from Task 7), `ForgeClient::search_mods()`, `ForgeClient::get_mod()`, `ForgeClient::get_versions()`, `ForgeClient::get_dependencies()`, `ForgeClient::download_file()`, `spt::mods::detect_mod_type()`, `spt::mods::extract_mod()`, `Database::insert_mod()`, `Database::insert_file()`, `Database::insert_dependency()`, `Database::get_mod_by_forge_id()`
- Produces:
  - `cli::install::run(mod_ref: &str, version: Option<&str>, force: bool, ctx: &CliContext) -> Result<()>`
  - `resolve_mod(forge: &ForgeClient, mod_ref: &str) -> Result<ForgeMod>` — shared mod resolution (name/ID/slug)
  - `install_single_mod(ctx: &CliContext, mod_id: i64, version_id: i64, download_url: &str, name: &str, slug: Option<&str>, version: &str) -> Result<i64>` — download, extract, record a single mod
  - `Database::get_mod_by_name_or_slug(query: &str) -> rusqlite::Result<Option<InstalledMod>>` — case-insensitive lookup

- [ ] **Step 1: Add `get_mod_by_name_or_slug()` to `src/db/mods.rs`**

```rust
// Add to impl Database in src/db/mods.rs

    pub fn get_mod_by_name_or_slug(&self, query: &str) -> rusqlite::Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
                 FROM installed_mods WHERE LOWER(name) = LOWER(?1) OR LOWER(slug) = LOWER(?1)",
                params![query],
                row_to_installed_mod,
            )
            .optional()
    }
```

- [ ] **Step 2: Write test for `get_mod_by_name_or_slug`**

Add to `src/db/tests.rs`:

```rust
#[test]
fn lookup_mod_by_name_or_slug() {
    let db = Database::open_in_memory().unwrap();
    db.insert_mod(100, 200, "SAIN", Some("sain"), "3.0.0").unwrap();

    // Lookup by name (case-insensitive)
    let by_name = db.get_mod_by_name_or_slug("sain").unwrap();
    assert!(by_name.is_some());
    assert_eq!(by_name.unwrap().name, "SAIN");

    // Lookup by slug
    let by_slug = db.get_mod_by_name_or_slug("sain").unwrap();
    assert!(by_slug.is_some());

    // Not found
    let missing = db.get_mod_by_name_or_slug("nonexistent").unwrap();
    assert!(missing.is_none());
}
```

- [ ] **Step 3: Run test to verify it passes**

```bash
cargo test -- db::tests::lookup_mod_by_name_or_slug
```

Expected: PASS

- [ ] **Step 4: Create `src/cli/install.rs`**

```rust
// src/cli/install.rs
use std::io::{self, Write};

use anyhow::{bail, Context, Result};

use crate::forge::client::ForgeClient;
use crate::forge::models::{FikaCompat, ForgeMod, ForgeVersion, DependencyNode};
use crate::spt::mods::{extract_mod, ModType, detect_mod_type};

use super::common::CliContext;

/// Resolve a user-provided mod reference (name, slug, or numeric ID) to a ForgeMod.
pub async fn resolve_mod(forge: &ForgeClient, mod_ref: &str) -> Result<ForgeMod> {
    // Try parsing as a numeric Forge mod ID first
    if let Ok(id) = mod_ref.parse::<i64>() {
        return forge
            .get_mod(id, false)
            .await
            .with_context(|| format!("mod with ID {id} not found on Forge"));
    }

    // Otherwise search by name/slug
    let results = forge.search_mods(mod_ref).await?;

    match results.len() {
        0 => bail!("no mods found matching '{mod_ref}' on Forge"),
        1 => Ok(results.into_iter().next().unwrap()),
        _ => {
            // Check for exact name or slug match first
            if let Some(exact) = results.iter().find(|m| {
                m.name.eq_ignore_ascii_case(mod_ref)
                    || m.slug.as_deref().map_or(false, |s| s.eq_ignore_ascii_case(mod_ref))
            }) {
                return Ok(exact.clone());
            }

            // Disambiguation: show options and let user pick
            println!("Multiple mods match '{mod_ref}':");
            for (i, m) in results.iter().enumerate() {
                println!(
                    "  [{}] {} (ID: {}){}",
                    i + 1,
                    m.name,
                    m.id,
                    m.description
                        .as_deref()
                        .map(|d| format!(" — {}", truncate(d, 60)))
                        .unwrap_or_default()
                );
            }

            print!("Select [1-{}]: ", results.len());
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let choice: usize = input
                .trim()
                .parse()
                .with_context(|| "invalid selection")?;

            if choice < 1 || choice > results.len() {
                bail!("selection out of range");
            }

            Ok(results.into_iter().nth(choice - 1).unwrap())
        }
    }
}

pub async fn run(mod_ref: &str, version: Option<&str>, _force: bool, ctx: &CliContext) -> Result<()> {
    // 1. Resolve mod
    let forge_mod = resolve_mod(&ctx.forge, mod_ref).await?;
    println!("Found: {} (ID: {})", forge_mod.name, forge_mod.id);

    // Check if already installed
    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        bail!(
            "{} is already installed (version {}). Use `quma update` to update it.",
            existing.name,
            existing.version
        );
    }

    // 2. Pick version
    let versions = ctx
        .forge
        .get_versions(forge_mod.id, Some(&ctx.spt_info.spt_version))
        .await?;

    let selected_version = match version {
        Some(v) => versions
            .iter()
            .find(|ver| ver.version == v)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "version {v} not found for {} (compatible with SPT {})",
                    forge_mod.name,
                    ctx.spt_info.spt_version
                )
            })?,
        None => versions
            .into_iter()
            .next()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no versions of {} are compatible with SPT {}",
                    forge_mod.name,
                    ctx.spt_info.spt_version
                )
            })?,
    };

    println!(
        "Selected version: {} (SPT {})",
        selected_version.version,
        selected_version
            .spt_version
            .as_deref()
            .unwrap_or("unknown")
    );

    // 3. Check Fika compatibility
    check_fika_compat(&selected_version)?;

    // 4. Resolve dependencies
    let dep_nodes = ctx
        .forge
        .get_dependencies(&[(forge_mod.id, selected_version.id)])
        .await?;

    // Flatten dependency tree to list of mods to install
    let mut to_install: Vec<(i64, i64, String, String)> = Vec::new(); // (mod_id, version_id, name, version)
    collect_deps_to_install(&dep_nodes, &ctx.db, &mut to_install)?;

    // 5. Display plan
    println!("\nInstall plan:");
    println!("  {} v{}", forge_mod.name, selected_version.version);
    for (_, _, name, ver) in &to_install {
        println!("  + {} v{} (dependency)", name, ver);
    }

    // 6. Install dependencies first
    for (dep_mod_id, dep_ver_id, dep_name, dep_ver) in &to_install {
        println!("\nInstalling dependency: {} v{}", dep_name, dep_ver);
        let dep_versions = ctx
            .forge
            .get_versions(*dep_mod_id, None)
            .await?;

        if let Some(dep_version) = dep_versions.iter().find(|v| v.id == *dep_ver_id) {
            let download_url = dep_version.link.as_deref().ok_or_else(|| {
                anyhow::anyhow!("no download link for {} v{}", dep_name, dep_ver)
            })?;
            let dep_mod = ctx.forge.get_mod(*dep_mod_id, false).await?;
            install_single_mod(
                ctx,
                *dep_mod_id,
                *dep_ver_id,
                download_url,
                dep_name,
                dep_mod.slug.as_deref(),
                dep_ver,
            )
            .await?;
        }
    }

    // 7. Install the main mod
    let download_url = selected_version.link.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "no download link for {} v{}",
            forge_mod.name,
            selected_version.version
        )
    })?;

    let db_id = install_single_mod(
        ctx,
        forge_mod.id,
        selected_version.id,
        download_url,
        &forge_mod.name,
        forge_mod.slug.as_deref(),
        &selected_version.version,
    )
    .await?;

    // 8. Record dependency relationships
    for (dep_mod_id, _, _, _) in &to_install {
        if let Some(dep_installed) = ctx.db.get_mod_by_forge_id(*dep_mod_id)? {
            ctx.db
                .insert_dependency(db_id, dep_installed.id, None)
                .ok(); // Ignore duplicate constraint errors
        }
    }

    println!(
        "\n{} v{} installed successfully.",
        forge_mod.name, selected_version.version
    );

    Ok(())
}

/// Download, extract, and record a single mod in the database.
pub async fn install_single_mod(
    ctx: &CliContext,
    forge_mod_id: i64,
    forge_version_id: i64,
    download_url: &str,
    name: &str,
    slug: Option<&str>,
    version: &str,
) -> Result<i64> {
    // Skip if already installed
    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod_id)? {
        println!("  {} already installed (v{}), skipping", name, existing.version);
        return Ok(existing.id);
    }

    // Download to temp file
    let tmp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let archive_path = tmp_dir.path().join("mod.zip");
    println!("  Downloading {}...", name);
    ctx.forge.download_file(download_url, &archive_path).await?;

    // Detect mod type
    let mod_type = detect_mod_type(&archive_path)?;
    if mod_type == ModType::Ambiguous {
        println!("  Warning: could not determine mod type for {}. Extracting as-is.", name);
    }

    // Extract
    println!("  Extracting...");
    let extracted_files = extract_mod(&archive_path, &ctx.spt_dir)?;
    println!("  Extracted {} files", extracted_files.len());

    // Record in database
    let db_id = ctx
        .db
        .insert_mod(forge_mod_id, forge_version_id, name, slug, version)?;

    for file in &extracted_files {
        ctx.db.insert_file(
            db_id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }

    Ok(db_id)
}

fn check_fika_compat(version: &ForgeVersion) -> Result<()> {
    match &version.fika_compatibility {
        Some(FikaCompat::Incompatible) => {
            println!(
                "Warning: {} v{} is marked as Fika INCOMPATIBLE.",
                version.version,
                version
                    .spt_version
                    .as_deref()
                    .unwrap_or("unknown")
            );
            print!("Continue anyway? [y/N]: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                bail!("installation cancelled due to Fika incompatibility");
            }
        }
        Some(FikaCompat::Unknown) => {
            println!(
                "Note: Fika compatibility for v{} is unknown.",
                version.version
            );
        }
        _ => {}
    }
    Ok(())
}

/// Walk the dependency tree and collect mods that need to be installed.
fn collect_deps_to_install(
    nodes: &[DependencyNode],
    db: &crate::db::Database,
    out: &mut Vec<(i64, i64, String, String)>,
) -> Result<()> {
    for node in nodes {
        // Skip if already installed or already in our install list
        if db.get_mod_by_forge_id(node.mod_id)?.is_some() {
            continue;
        }
        if out.iter().any(|(id, _, _, _)| *id == node.mod_id) {
            continue;
        }

        let name = node
            .name
            .clone()
            .unwrap_or_else(|| format!("mod-{}", node.mod_id));
        let version = node
            .version
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        out.push((node.mod_id, node.version_id, name, version));

        // Recurse into nested dependencies
        if let Some(ref children) = node.resolved_dependencies {
            collect_deps_to_install(children, db, out)?;
        }
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
```

- [ ] **Step 5: Wire up in `src/cli/mod.rs` and `src/main.rs`**

Add to `src/cli/mod.rs`:
```rust
pub mod install;
```

Update `src/main.rs`:
```rust
Command::Install { mod_ref, version, force } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::install::run(&mod_ref, version.as_deref(), force, &ctx).await
}
```

- [ ] **Step 6: Write unit tests for `resolve_mod` and `collect_deps_to_install`**

Add to `src/cli/install.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::forge::models::DependencyNode;

    #[test]
    fn collect_deps_skips_already_installed() {
        let db = Database::open_in_memory().unwrap();
        // Pre-install mod 10
        db.insert_mod(10, 20, "AlreadyInstalled", None, "1.0.0")
            .unwrap();

        let nodes = vec![DependencyNode {
            mod_id: 10,
            version_id: 20,
            name: Some("AlreadyInstalled".to_string()),
            version: Some("1.0.0".to_string()),
            resolved_dependencies: None,
        }];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        assert!(out.is_empty(), "should skip already-installed deps");
    }

    #[test]
    fn collect_deps_flattens_tree() {
        let db = Database::open_in_memory().unwrap();

        let nodes = vec![DependencyNode {
            mod_id: 10,
            version_id: 20,
            name: Some("Parent".to_string()),
            version: Some("1.0.0".to_string()),
            resolved_dependencies: Some(vec![DependencyNode {
                mod_id: 30,
                version_id: 40,
                name: Some("Child".to_string()),
                version: Some("0.5.0".to_string()),
                resolved_dependencies: None,
            }]),
        }];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, 10); // Parent first
        assert_eq!(out[1].0, 30); // Child second
    }

    #[test]
    fn collect_deps_deduplicates() {
        let db = Database::open_in_memory().unwrap();

        let shared_dep = DependencyNode {
            mod_id: 99,
            version_id: 100,
            name: Some("SharedLib".to_string()),
            version: Some("1.0.0".to_string()),
            resolved_dependencies: None,
        };

        let nodes = vec![
            DependencyNode {
                mod_id: 10,
                version_id: 20,
                name: Some("ModA".to_string()),
                version: Some("1.0.0".to_string()),
                resolved_dependencies: Some(vec![shared_dep.clone()]),
            },
            DependencyNode {
                mod_id: 30,
                version_id: 40,
                name: Some("ModB".to_string()),
                version: Some("2.0.0".to_string()),
                resolved_dependencies: Some(vec![DependencyNode {
                    mod_id: 99,
                    version_id: 100,
                    name: Some("SharedLib".to_string()),
                    version: Some("1.0.0".to_string()),
                    resolved_dependencies: None,
                }]),
            },
        ];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        let shared_count = out.iter().filter(|(id, _, _, _)| *id == 99).count();
        assert_eq!(shared_count, 1, "SharedLib should appear only once");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate("hello world", 5), "hello");
    }
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -- cli::install
```

Expected: All tests pass.

- [ ] **Step 8: Run full test suite and clippy**

```bash
cargo clippy -- -D warnings && cargo test
```

Expected: No warnings, all tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/cli/install.rs src/cli/mod.rs src/main.rs src/db/mods.rs src/db/tests.rs
git commit -m "feat: add install command with dependency resolution and Forge download"
```

---

### Task 9: Remove Command

**Files:**
- Create: `src/cli/remove.rs`
- Modify: `src/cli/mod.rs` (add `mod remove;`)
- Modify: `src/main.rs` (wire `Command::Remove` dispatch)

**Interfaces:**
- Consumes: `CliContext`, `Database::get_mod_by_forge_id()`, `Database::get_mod_by_name_or_slug()`, `Database::get_reverse_dependencies()`, `Database::get_mod()`, `Database::get_files_for_mod()`, `Database::delete_mod()`, `spt::mods::delete_mod_files()`
- Produces: `cli::remove::run(mod_ref: &str, force: bool, ctx: &CliContext) -> Result<()>`

- [ ] **Step 1: Create `src/cli/remove.rs`**

```rust
// src/cli/remove.rs
use std::io::{self, Write};

use anyhow::{bail, Result};

use crate::db::mods::InstalledMod;
use crate::spt::mods::delete_mod_files;

use super::common::CliContext;

pub fn run(mod_ref: &str, _force: bool, ctx: &CliContext) -> Result<()> {
    // 1. Resolve mod reference to installed mod
    let installed = resolve_installed_mod(mod_ref, ctx)?;

    // 2. Check reverse dependencies
    let rev_deps = ctx.db.get_reverse_dependencies(installed.id)?;
    if !rev_deps.is_empty() {
        println!(
            "Warning: the following installed mods depend on {}:",
            installed.name
        );
        for dep in &rev_deps {
            if let Some(dependent) = ctx.db.get_mod(dep.mod_id)? {
                println!("  - {} (v{})", dependent.name, dependent.version);
            }
        }

        println!("\nOptions:");
        println!("  [1] Remove {} only (may break dependents)", installed.name);
        println!("  [2] Remove {} and all dependents", installed.name);
        println!("  [3] Cancel");

        print!("Select [1-3]: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => {
                remove_single_mod(&installed, ctx)?;
            }
            "2" => {
                // Remove dependents first (in reverse order)
                for dep in &rev_deps {
                    if let Some(dependent) = ctx.db.get_mod(dep.mod_id)? {
                        remove_single_mod(&dependent, ctx)?;
                    }
                }
                remove_single_mod(&installed, ctx)?;
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else {
        remove_single_mod(&installed, ctx)?;
    }

    println!("{} removed successfully.", installed.name);
    Ok(())
}

/// Resolve a user-provided mod reference to an installed mod in the database.
///
/// Tries: numeric Forge ID, then name/slug lookup.
pub fn resolve_installed_mod(mod_ref: &str, ctx: &CliContext) -> Result<InstalledMod> {
    // Try as numeric Forge mod ID
    if let Ok(forge_id) = mod_ref.parse::<i64>() {
        if let Some(m) = ctx.db.get_mod_by_forge_id(forge_id)? {
            return Ok(m);
        }
    }

    // Try by name or slug
    if let Some(m) = ctx.db.get_mod_by_name_or_slug(mod_ref)? {
        return Ok(m);
    }

    bail!(
        "mod '{}' is not installed. Run `quma list` to see installed mods.",
        mod_ref
    );
}

fn remove_single_mod(installed: &InstalledMod, ctx: &CliContext) -> Result<()> {
    // Get tracked files
    let files = ctx.db.get_files_for_mod(installed.id)?;
    let file_paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();

    // Delete files from disk
    if !file_paths.is_empty() {
        delete_mod_files(&ctx.spt_dir, &file_paths)?;
        println!(
            "  Deleted {} files for {}",
            file_paths.len(),
            installed.name
        );
    }

    // Remove from database (cascades to files and dependencies)
    ctx.db.delete_mod(installed.id)?;

    Ok(())
}
```

- [ ] **Step 2: Wire up in `src/cli/mod.rs` and `src/main.rs`**

Add to `src/cli/mod.rs`:
```rust
pub mod remove;
```

Update `src/main.rs`:
```rust
Command::Remove { mod_ref, force } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::remove::run(&mod_ref, force, &ctx)
}
```

- [ ] **Step 3: Write tests**

Add to `src/cli/remove.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::config::Config;
    use crate::db::Database;
    use crate::forge::client::ForgeClient;
    use crate::spt::detect::SptInfo;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_test_ctx(tmp: &TempDir) -> CliContext {
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            config_path: tmp.path().join("quartermaster.toml"),
            db: Database::open_in_memory().unwrap(),
            forge: ForgeClient::new(None).unwrap(),
        }
    }

    #[test]
    fn resolve_installed_by_forge_id() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db.insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0").unwrap();

        let m = resolve_installed_mod("100", &ctx).unwrap();
        assert_eq!(m.name, "TestMod");
    }

    #[test]
    fn resolve_installed_by_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db.insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0").unwrap();

        let m = resolve_installed_mod("TestMod", &ctx).unwrap();
        assert_eq!(m.forge_mod_id, 100);
    }

    #[test]
    fn resolve_installed_by_slug() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db.insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0").unwrap();

        let m = resolve_installed_mod("test-mod", &ctx).unwrap();
        assert_eq!(m.forge_mod_id, 100);
    }

    #[test]
    fn resolve_installed_not_found() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        let result = resolve_installed_mod("nonexistent", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn remove_single_mod_deletes_files_and_db() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);

        // Create mod files on disk
        let mod_dir = ctx.spt_dir.join("user/mods/TestMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();

        // Insert into DB
        let db_id = ctx.db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        ctx.db.insert_file(db_id, "user/mods/TestMod/package.json", Some("abc"), Some(2)).unwrap();

        let installed = ctx.db.get_mod_by_forge_id(100).unwrap().unwrap();
        remove_single_mod(&installed, &ctx).unwrap();

        // File should be gone
        assert!(!mod_dir.join("package.json").exists());

        // DB record should be gone
        assert!(ctx.db.get_mod_by_forge_id(100).unwrap().is_none());
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -- cli::remove
```

Expected: All 5 tests pass.

- [ ] **Step 5: Run full test suite and clippy**

```bash
cargo clippy -- -D warnings && cargo test
```

Expected: No warnings, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/cli/remove.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add remove command with reverse dependency checking"
```

---

### Task 10: Update Command

**Files:**
- Create: `src/cli/update.rs`
- Modify: `src/cli/mod.rs` (add `mod update;`)
- Modify: `src/main.rs` (wire `Command::Update` dispatch)

**Interfaces:**
- Consumes: `CliContext`, `Database::list_mods()`, `Database::get_files_for_mod()`, `Database::delete_files_for_mod()`, `Database::update_mod()`, `ForgeClient::check_updates()`, `ForgeClient::get_versions()`, `ForgeClient::download_file()`, `spt::mods::delete_mod_files()`, `spt::mods::extract_mod()`, `Database::insert_file()`, `cli::remove::resolve_installed_mod()`
- Produces: `cli::update::run(mod_ref: Option<&str>, force: bool, ctx: &CliContext) -> Result<()>`

- [ ] **Step 1: Create `src/cli/update.rs`**

```rust
// src/cli/update.rs
use anyhow::{bail, Result};

use crate::db::mods::InstalledMod;
use crate::spt::mods::{delete_mod_files, extract_mod};

use super::common::CliContext;
use super::remove::resolve_installed_mod;

pub async fn run(mod_ref: Option<&str>, _force: bool, ctx: &CliContext) -> Result<()> {
    let mods_to_check: Vec<InstalledMod> = match mod_ref {
        Some(r) => vec![resolve_installed_mod(r, ctx)?],
        None => ctx.db.list_mods()?,
    };

    if mods_to_check.is_empty() {
        println!("No mods installed. Use `quma install` to install mods.");
        return Ok(());
    }

    // Build the list for the updates API
    let check_list: Vec<(i64, String)> = mods_to_check
        .iter()
        .map(|m| (m.forge_mod_id, m.version.clone()))
        .collect();

    let results = ctx
        .forge
        .check_updates(&check_list, &ctx.spt_info.spt_version)
        .await?;

    // Collect mods that have updates available
    let updatable: Vec<_> = results
        .iter()
        .filter(|r| r.status == "updated")
        .collect();

    if updatable.is_empty() {
        println!("All mods are up to date.");
        // Still report other statuses
        for r in &results {
            match r.status.as_str() {
                "blocked" => println!(
                    "  {} — blocked (dependency conflict)",
                    mod_name_for_id(&mods_to_check, r.mod_id)
                ),
                "incompatible" => println!(
                    "  {} — incompatible with SPT {}",
                    mod_name_for_id(&mods_to_check, r.mod_id),
                    ctx.spt_info.spt_version
                ),
                _ => {}
            }
        }
        return Ok(());
    }

    // Display update plan
    println!("Updates available:");
    for r in &updatable {
        let name = mod_name_for_id(&mods_to_check, r.mod_id);
        println!(
            "  {} — {} → {}",
            name,
            r.current_version,
            r.latest_version.as_deref().unwrap_or("?")
        );
    }

    // Apply updates
    let mut updated_count = 0;
    for update_result in &updatable {
        let installed = mods_to_check
            .iter()
            .find(|m| m.forge_mod_id == update_result.mod_id)
            .unwrap();

        let latest_version_id = match update_result.latest_version_id {
            Some(id) => id,
            None => {
                println!(
                    "  Skipping {} — no version ID in update response",
                    installed.name
                );
                continue;
            }
        };

        let latest_version_str = update_result
            .latest_version
            .as_deref()
            .unwrap_or("unknown");

        // Get the version details to find the download link
        let versions = ctx
            .forge
            .get_versions(installed.forge_mod_id, None)
            .await?;

        let version_info = match versions.iter().find(|v| v.id == latest_version_id) {
            Some(v) => v,
            None => {
                println!(
                    "  Skipping {} — version {} not found",
                    installed.name, latest_version_id
                );
                continue;
            }
        };

        let download_url = match &version_info.link {
            Some(url) => url.clone(),
            None => {
                println!(
                    "  Skipping {} — no download link for v{}",
                    installed.name, latest_version_str
                );
                continue;
            }
        };

        println!("\nUpdating {} to v{}...", installed.name, latest_version_str);

        // Download new version to temp
        let tmp_dir = tempfile::tempdir()?;
        let archive_path = tmp_dir.path().join("mod.zip");
        ctx.forge.download_file(&download_url, &archive_path).await?;

        // Delete old files from disk
        let old_files = ctx.db.get_files_for_mod(installed.id)?;
        let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
        delete_mod_files(&ctx.spt_dir, &old_paths)?;

        // Delete old file records from DB
        ctx.db.delete_files_for_mod(installed.id)?;

        // Extract new version
        let new_files = extract_mod(&archive_path, &ctx.spt_dir)?;

        // Insert new file records
        for file in &new_files {
            ctx.db.insert_file(
                installed.id,
                &file.path,
                Some(&file.hash),
                Some(file.size as i64),
            )?;
        }

        // Update mod record
        ctx.db
            .update_mod(installed.id, latest_version_id, latest_version_str)?;

        println!(
            "  Updated {} files for {}",
            new_files.len(),
            installed.name
        );
        updated_count += 1;
    }

    println!("\n{} mod(s) updated.", updated_count);
    Ok(())
}

fn mod_name_for_id(mods: &[InstalledMod], forge_mod_id: i64) -> &str {
    mods.iter()
        .find(|m| m.forge_mod_id == forge_mod_id)
        .map(|m| m.name.as_str())
        .unwrap_or("unknown")
}
```

- [ ] **Step 2: Wire up in `src/cli/mod.rs` and `src/main.rs`**

Add to `src/cli/mod.rs`:
```rust
pub mod update;
```

Update `src/main.rs`:
```rust
Command::Update { mod_ref, force } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::update::run(mod_ref.as_deref(), force, &ctx).await
}
```

- [ ] **Step 3: Write unit tests**

Add to `src/cli/update.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_name_for_id_finds_match() {
        let mods = vec![InstalledMod {
            id: 1,
            forge_mod_id: 100,
            forge_version_id: 200,
            name: "TestMod".to_string(),
            slug: None,
            version: "1.0.0".to_string(),
            installed_at: "2025-01-01".to_string(),
            updated_at: None,
        }];

        assert_eq!(mod_name_for_id(&mods, 100), "TestMod");
    }

    #[test]
    fn mod_name_for_id_returns_unknown_on_miss() {
        let mods: Vec<InstalledMod> = vec![];
        assert_eq!(mod_name_for_id(&mods, 999), "unknown");
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -- cli::update
```

Expected: PASS

- [ ] **Step 5: Run full test suite and clippy**

```bash
cargo clippy -- -D warnings && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/cli/update.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add update command with Forge API update checking"
```

---

### Task 11: List & Check Commands

**Files:**
- Create: `src/cli/list.rs`
- Create: `src/cli/check.rs`
- Modify: `src/cli/mod.rs` (add `mod list; mod check;`)
- Modify: `src/main.rs` (wire `Command::List` and `Command::Check` dispatch)

**Interfaces:**
- Consumes: `CliContext`, `Database::list_mods()`, `Database::get_files_for_mod()`, `Database::get_all_tracked_files()`, `ForgeClient::check_updates()`, `spt::mods::scan_mod_directories()`
- Produces:
  - `cli::list::run(json: bool, ctx: &CliContext) -> Result<()>`
  - `cli::check::run(ctx: &CliContext) -> Result<()>` — exits with code 1 if updates available

- [ ] **Step 1: Create `src/cli/list.rs`**

```rust
// src/cli/list.rs
use std::collections::HashSet;

use anyhow::Result;
use serde::Serialize;

use crate::spt::mods::scan_mod_directories;

use super::common::CliContext;

#[derive(Serialize)]
struct ModEntry {
    name: String,
    version: String,
    forge_mod_id: i64,
    slug: Option<String>,
    file_count: usize,
    installed_at: String,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct UnmanagedEntry {
    directory: String,
    file_count: usize,
}

#[derive(Serialize)]
struct ListOutput {
    mods: Vec<ModEntry>,
    unmanaged: Vec<UnmanagedEntry>,
}

pub fn run(json: bool, ctx: &CliContext) -> Result<()> {
    let installed_mods = ctx.db.list_mods()?;
    let all_tracked_files = ctx.db.get_all_tracked_files()?;
    let tracked_paths: HashSet<&str> = all_tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    // Build mod entries
    let mut mod_entries = Vec::new();
    for m in &installed_mods {
        let files = ctx.db.get_files_for_mod(m.id)?;
        mod_entries.push(ModEntry {
            name: m.name.clone(),
            version: m.version.clone(),
            forge_mod_id: m.forge_mod_id,
            slug: m.slug.clone(),
            file_count: files.len(),
            installed_at: m.installed_at.clone(),
            updated_at: m.updated_at.clone(),
        });
    }

    // Scan for unmanaged files
    let all_disk_files = scan_mod_directories(&ctx.spt_dir)?;
    let unmanaged_files: Vec<&str> = all_disk_files
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    // Group unmanaged by top-level mod directory
    let mut unmanaged_dirs: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for path in &unmanaged_files {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 3 {
            let dir = format!("{}/{}/{}", parts[0], parts[1], parts[2]);
            *unmanaged_dirs.entry(dir).or_default() += 1;
        }
    }

    let unmanaged_entries: Vec<UnmanagedEntry> = unmanaged_dirs
        .into_iter()
        .map(|(dir, count)| UnmanagedEntry {
            directory: dir,
            file_count: count,
        })
        .collect();

    if json {
        let output = ListOutput {
            mods: mod_entries,
            unmanaged: unmanaged_entries,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Table output
    if mod_entries.is_empty() && unmanaged_entries.is_empty() {
        println!("No mods installed and no unmanaged mods found.");
        return Ok(());
    }

    if !mod_entries.is_empty() {
        println!(
            "{:<30} {:<12} {:<8} {:<20}",
            "Name", "Version", "Files", "Installed"
        );
        println!("{}", "-".repeat(72));

        for entry in &mod_entries {
            println!(
                "{:<30} {:<12} {:<8} {:<20}",
                truncate_string(&entry.name, 29),
                entry.version,
                entry.file_count,
                &entry.installed_at[..10.min(entry.installed_at.len())],
            );
        }
    }

    if !unmanaged_entries.is_empty() {
        println!("\nUnmanaged mods:");
        for entry in &unmanaged_entries {
            println!(
                "  {} ({} files)",
                entry.directory, entry.file_count
            );
        }
        println!("\nUse `quma track <path> <forge_mod_id>` to manage them.");
    }

    Ok(())
}

fn truncate_string(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn truncate_long() {
        let result = truncate_string("very long mod name here", 10);
        assert_eq!(result.chars().count(), 10);
        assert!(result.ends_with('…'));
    }
}
```

- [ ] **Step 2: Create `src/cli/check.rs`**

```rust
// src/cli/check.rs
use anyhow::Result;

use super::common::CliContext;

pub async fn run(ctx: &CliContext) -> Result<()> {
    let installed = ctx.db.list_mods()?;

    if installed.is_empty() {
        println!("No mods installed.");
        return Ok(());
    }

    let check_list: Vec<(i64, String)> = installed
        .iter()
        .map(|m| (m.forge_mod_id, m.version.clone()))
        .collect();

    let results = ctx
        .forge
        .check_updates(&check_list, &ctx.spt_info.spt_version)
        .await?;

    let mut has_updates = false;

    // Categorize results
    let mut up_to_date = Vec::new();
    let mut updatable = Vec::new();
    let mut blocked = Vec::new();
    let mut incompatible = Vec::new();

    for r in &results {
        let name = installed
            .iter()
            .find(|m| m.forge_mod_id == r.mod_id)
            .map(|m| m.name.as_str())
            .unwrap_or("unknown");

        match r.status.as_str() {
            "up_to_date" => up_to_date.push(name),
            "updated" => {
                has_updates = true;
                updatable.push((
                    name,
                    r.current_version.as_str(),
                    r.latest_version.as_deref().unwrap_or("?"),
                ));
            }
            "blocked" => blocked.push(name),
            "incompatible" => incompatible.push(name),
            _ => {}
        }
    }

    if !updatable.is_empty() {
        println!("Updates available:");
        for (name, current, latest) in &updatable {
            println!("  {} — {} → {}", name, current, latest);
        }
    }

    if !blocked.is_empty() {
        println!("\nBlocked (dependency conflict):");
        for name in &blocked {
            println!("  {}", name);
        }
    }

    if !incompatible.is_empty() {
        println!(
            "\nIncompatible with SPT {}:",
            ctx.spt_info.spt_version
        );
        for name in &incompatible {
            println!("  {}", name);
        }
    }

    if !up_to_date.is_empty() {
        println!("\nUp to date ({}):", up_to_date.len());
        for name in &up_to_date {
            println!("  ✓ {}", name);
        }
    }

    if has_updates {
        println!("\nRun `quma update` to apply updates.");
        std::process::exit(1);
    }

    Ok(())
}
```

- [ ] **Step 3: Wire up in `src/cli/mod.rs` and `src/main.rs`**

Add to `src/cli/mod.rs`:
```rust
pub mod check;
pub mod list;
```

Update `src/main.rs`:
```rust
Command::List { json } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::list::run(json, &ctx)
}
Command::Check => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::check::run(&ctx).await
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -- cli::list
```

Expected: PASS

- [ ] **Step 5: Run full test suite and clippy**

```bash
cargo clippy -- -D warnings && cargo test
```

- [ ] **Step 6: Commit**

```bash
git add src/cli/list.rs src/cli/check.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add list and check commands for mod status reporting"
```

---

### Task 12: Track Command

**Files:**
- Create: `src/cli/track.rs`
- Modify: `src/cli/mod.rs` (add `mod track;`)
- Modify: `src/main.rs` (wire `Command::Track` dispatch)

**Interfaces:**
- Consumes: `CliContext`, `ForgeClient::get_mod()`, `ForgeClient::get_versions()`, `spt::mods::scan_mod_directories()`, `spt::mods::compute_file_hash()`, `Database::insert_mod()`, `Database::insert_file()`, `Database::get_mod_by_forge_id()`
- Produces: `cli::track::run(path: &str, forge_mod_ref: &str, ctx: &CliContext) -> Result<()>`

- [ ] **Step 1: Create `src/cli/track.rs`**

```rust
// src/cli/track.rs
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::spt::mods::compute_file_hash;

use super::common::CliContext;
use super::install::resolve_mod;

pub async fn run(path: &str, forge_mod_ref: &str, ctx: &CliContext) -> Result<()> {
    // 1. Validate the path exists under the SPT root
    let full_path = ctx.spt_dir.join(path);
    if !full_path.exists() {
        bail!("path does not exist: {}", full_path.display());
    }
    if !full_path.is_dir() {
        bail!("path is not a directory: {}", full_path.display());
    }

    // Ensure the path is under user/mods/ or BepInEx/plugins/
    if !path.starts_with("user/mods/") && !path.starts_with("BepInEx/plugins/") {
        bail!(
            "path must be under user/mods/ or BepInEx/plugins/, got: {}",
            path
        );
    }

    // 2. Resolve the Forge mod
    let forge_mod = resolve_mod(&ctx.forge, forge_mod_ref).await?;
    println!("Forge mod: {} (ID: {})", forge_mod.name, forge_mod.id);

    // Check if already tracked
    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        bail!(
            "{} is already tracked (version {})",
            existing.name,
            existing.version
        );
    }

    // 3. Determine version — get latest versions and try to match, or use "unknown"
    let versions = ctx
        .forge
        .get_versions(forge_mod.id, Some(&ctx.spt_info.spt_version))
        .await?;

    let (version_id, version_str) = if let Some(latest) = versions.first() {
        println!(
            "Assuming version: {} (latest compatible with SPT {})",
            latest.version, ctx.spt_info.spt_version
        );
        (latest.id, latest.version.clone())
    } else {
        // Fall back to any version
        let all_versions = ctx.forge.get_versions(forge_mod.id, None).await?;
        if let Some(latest) = all_versions.first() {
            println!(
                "Warning: no SPT {}-compatible version found. Using latest: {}",
                ctx.spt_info.spt_version, latest.version
            );
            (latest.id, latest.version.clone())
        } else {
            bail!("no versions found for {} on Forge", forge_mod.name);
        }
    };

    // 4. Scan directory for files
    let mut files = Vec::new();
    scan_dir_for_tracking(&full_path, &ctx.spt_dir, &mut files)?;

    if files.is_empty() {
        bail!("no files found in {}", path);
    }

    println!("Found {} files to track", files.len());

    // 5. Record in database
    let db_id = ctx.db.insert_mod(
        forge_mod.id,
        version_id,
        &forge_mod.name,
        forge_mod.slug.as_deref(),
        &version_str,
    )?;

    for (rel_path, hash, size) in &files {
        ctx.db
            .insert_file(db_id, rel_path, Some(hash.as_str()), Some(*size as i64))?;
    }

    println!(
        "\n{} v{} is now tracked ({} files).",
        forge_mod.name,
        version_str,
        files.len()
    );

    Ok(())
}

/// Recursively scan a directory, collecting (relative_path, sha256_hash, size) for each file.
fn scan_dir_for_tracking(
    dir: &Path,
    spt_root: &Path,
    out: &mut Vec<(String, String, u64)>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            scan_dir_for_tracking(&path, spt_root, out)?;
        } else {
            let rel = path
                .strip_prefix(spt_root)
                .with_context(|| "path not under SPT root")?
                .to_string_lossy()
                .to_string();

            let hash = compute_file_hash(&path)?;
            let size = std::fs::metadata(&path)?.len();

            out.push((rel, hash, size));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scan_dir_collects_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let mod_dir = root.join("user/mods/TestMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();
        std::fs::write(mod_dir.join("mod.ts"), b"// code").unwrap();

        let mut files = Vec::new();
        scan_dir_for_tracking(&mod_dir, root, &mut files).unwrap();

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|(p, _, _)| p == "user/mods/TestMod/package.json"));
        assert!(files.iter().any(|(p, _, _)| p == "user/mods/TestMod/mod.ts"));

        // Verify hashes are present
        for (_, hash, _) in &files {
            assert_eq!(hash.len(), 64);
        }
    }

    #[test]
    fn scan_dir_empty() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("empty");
        std::fs::create_dir_all(&dir).unwrap();

        let mut files = Vec::new();
        scan_dir_for_tracking(&dir, tmp.path(), &mut files).unwrap();

        assert!(files.is_empty());
    }
}
```

- [ ] **Step 2: Wire up in `src/cli/mod.rs` and `src/main.rs`**

Add to `src/cli/mod.rs`:
```rust
pub mod track;
```

Update `src/main.rs`:
```rust
Command::Track { path, forge_mod_id } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::track::run(&path, &forge_mod_id, &ctx).await
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -- cli::track
```

Expected: PASS

- [ ] **Step 4: Run full test suite and clippy**

```bash
cargo clippy -- -D warnings && cargo test
```

- [ ] **Step 5: Commit**

```bash
git add src/cli/track.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add track command to associate unmanaged mods with Forge entries"
```

---

## Final Verification

After all tasks are complete, run the full quality check:

```bash
cargo clippy -- -D warnings && cargo test && cargo build
```

Verify:
- All 6 commands (`init`, `install`, `remove`, `update`, `list`, `check`, `track`) are wired into `src/main.rs`
- No remaining `todo!()` calls for Phase 2 commands in `src/main.rs`
- All tests pass
- No clippy warnings
- Binary compiles cleanly

The remaining `todo!()` arms in `main.rs` (`Setup`, `Apply`, `Status`, `Server`, `Serve`, `Generate`, `Invite`, `Config`) are Phase 3+ and should be left as-is.
