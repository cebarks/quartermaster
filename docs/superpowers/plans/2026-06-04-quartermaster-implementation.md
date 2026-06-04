# Quartermaster (`quma`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `quma`, a single Rust binary providing CLI + web UI for managing server-side mods on an SPT/Fika dedicated server. Installs, updates, and removes mods from the SPT Forge, with a web dashboard for both the server host (admin) and connected players.

**Architecture:** Single binary with clap CLI dispatch and an embedded actix-web server (the `serve` subcommand). SQLite (WAL mode) for state. Forge API (`https://forge.sp-tarkov.com/api/v0`) for mod discovery/downloads. Podman for SPT server lifecycle. HTMX + askama for the web UI (no JS build step).

**Tech Stack:** Rust, clap (derive), actix-web, askama, HTMX, rust-embed, rusqlite, reqwest, serde, argon2, tokio, indicatif

**Spec:** `SPEC.md` in the project root is the authoritative reference. Read it before starting any task.

---

## Roadmap

| Phase | Focus | Tasks | Produces |
|-------|-------|-------|----------|
| 1 | Foundation | 1–6 | Compilable project, config system, SPT detection, DB layer, Forge client, archive handling |
| 2 | Core CLI | 7–12 | `init`, `install`, `remove`, `update`, `list`, `check`, `track` commands |
| 3 | Server & Queue | 13–16 | Podman integration, change queue, `apply`, server lifecycle, health checks (`status`) |
| 4 | Web UI | 17–20 | Auth, dashboard, mod management pages, queue/status/server-control pages |
| 5 | Setup & Utilities | 21–23 | `setup`, `generate systemd`, `config`, `invite`, `serve` commands |

Phase 1 is fully detailed below. Phases 2–5 have task outlines with file lists, key interfaces, and goals — they will be expanded to full detail before execution.

---

## Phase 1: Foundation

### Task 1: Project Scaffold & CLI Skeleton

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/cli/mod.rs`
- Create: `src/config.rs` (empty placeholder)
- Create: `src/error.rs`
- Create: `src/forge/mod.rs` (empty placeholder)
- Create: `src/db/mod.rs` (empty placeholder)
- Create: `src/spt/mod.rs` (empty placeholder)
- Create: `src/web/mod.rs` (empty placeholder)
- Create: `Justfile`
- Create: `.gitignore`

- [ ] **Step 1: Initialize the Rust project**

```bash
cd ~/code/quartermaster
cargo init --name quartermaster
```

This creates a basic `Cargo.toml` and `src/main.rs`. We'll overwrite both.

- [ ] **Step 2: Write `Cargo.toml` with Phase 1 dependencies**

```toml
[package]
name = "quartermaster"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "quma"
path = "src/main.rs"

[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }
indicatif = "0.17"

# Serialization & config
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Database
rusqlite = { version = "0.32", features = ["bundled"] }

# HTTP client (for Forge API)
reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"], default-features = false }

# Async runtime
tokio = { version = "1", features = ["full"] }

# Error handling
anyhow = "1"
thiserror = "2"

# Crypto & hashing
sha2 = "0.10"
rand = "0.9"

# Archive handling
zip = "2"
tempfile = "3"

# Time
chrono = { version = "0.4", features = ["serde"] }

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 3: Write `src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QumaError {
    #[error("SPT directory not found — run `quma init` or pass --spt-dir")]
    SptDirNotFound,

    #[error("not a valid SPT 4.0+ install: {0}")]
    InvalidSptDir(String),

    #[error("config file not found: {0}")]
    ConfigNotFound(std::path::PathBuf),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("forge API error: {0}")]
    ForgeApi(String),

    #[error("forge API request failed: {0}")]
    ForgeHttp(#[from] reqwest::Error),

    #[error("archive error: {0}")]
    Archive(String),

    #[error("mod not found: {0}")]
    ModNotFound(String),

    #[error("mod conflict: file {path} already belongs to mod {owner}")]
    FileConflict { path: String, owner: String },

    #[error("server is running — queue the operation or use --force")]
    ServerRunning,
}
```

- [ ] **Step 4: Write `src/cli/mod.rs` with the full clap structure**

```rust
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "quma", version, about = "Quartermaster — SPT/Fika mod manager")]
pub struct Cli {
    /// Explicit SPT server directory
    #[arg(long, global = true)]
    pub spt_dir: Option<PathBuf>,

    /// Config file path override
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Interactive guided setup for Fika multiplayer
    Setup {
        /// Accept all defaults, skip prompts
        #[arg(long)]
        non_interactive: bool,
        /// Skip Fika installation (server management only)
        #[arg(long)]
        skip_fika: bool,
    },

    /// Initialize Quartermaster for an SPT server
    Init {
        /// SPT directory path (auto-detects if omitted)
        path: Option<PathBuf>,
    },

    /// Install a mod and its dependencies
    Install {
        /// Mod name, Forge ID, or slug
        mod_ref: String,
        /// Specific version (latest compatible if omitted)
        version: Option<String>,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
    },

    /// Update installed mods
    Update {
        /// Specific mod to update (all if omitted)
        mod_ref: Option<String>,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
    },

    /// Remove an installed mod
    Remove {
        /// Mod name, Forge ID, or slug
        mod_ref: String,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
    },

    /// List installed mods
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Associate an unmanaged mod with a Forge entry
    Track {
        /// Relative path from SPT root (e.g. user/mods/SAIN)
        path: String,
        /// Forge mod ID or slug
        forge_mod_id: String,
    },

    /// Check all installed mods for updates
    Check,

    /// Apply pending queued operations
    Apply {
        /// Apply even if SPT server is running
        #[arg(long)]
        force: bool,
    },

    /// Run health checks against SPT server and mod integrity
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage the SPT server container
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },

    /// Start the Quartermaster web UI
    Serve {
        /// Bind address
        #[arg(long)]
        bind: Option<String>,
        /// Port number
        #[arg(long)]
        port: Option<u16>,
    },

    /// Generate configuration files
    Generate {
        #[command(subcommand)]
        target: GenerateTarget,
    },

    /// Generate an invite code for a player
    Invite {
        /// Expiry duration (e.g. 24h, 7d)
        #[arg(long)]
        expires: Option<String>,
    },

    /// View and modify configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Start the SPT server container
    Start {
        /// Ping timeout in seconds
        #[arg(long, default_value = "60")]
        timeout: u64,
    },
    /// Stop the SPT server container
    Stop,
    /// Restart the SPT server container
    Restart {
        /// Force drain queue regardless of config
        #[arg(long)]
        drain: bool,
        /// Skip queue drain regardless of config
        #[arg(long)]
        skip_queue: bool,
    },
    /// Tail container logs
    Logs {
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
    /// Alias for `quma status`
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum GenerateTarget {
    /// Emit a systemd service file for `quma serve`
    Systemd {
        /// Write directly to /etc/systemd/system/ and enable
        #[arg(long)]
        install: bool,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a config value
    Set {
        key: String,
        value: String,
    },
    /// Get a config value
    Get {
        key: String,
    },
}
```

- [ ] **Step 5: Write `src/main.rs` with CLI dispatch skeleton**

```rust
mod cli;
mod config;
mod db;
mod error;
mod forge;
mod spt;
mod web;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Setup { .. } => todo!("setup"),
        Command::Init { .. } => todo!("init"),
        Command::Install { .. } => todo!("install"),
        Command::Update { .. } => todo!("update"),
        Command::Remove { .. } => todo!("remove"),
        Command::List { .. } => todo!("list"),
        Command::Track { .. } => todo!("track"),
        Command::Check => todo!("check"),
        Command::Apply { .. } => todo!("apply"),
        Command::Status { .. } => todo!("status"),
        Command::Server { .. } => todo!("server"),
        Command::Serve { .. } => todo!("serve"),
        Command::Generate { .. } => todo!("generate"),
        Command::Invite { .. } => todo!("invite"),
        Command::Config { .. } => todo!("config"),
    }
}
```

- [ ] **Step 6: Create empty module placeholders**

Each of these files just re-exports or is empty for now:

`src/config.rs`:
```rust
// Populated in Task 2
```

`src/db/mod.rs`:
```rust
// Populated in Task 4
```

`src/forge/mod.rs`:
```rust
// Populated in Task 5
```

`src/spt/mod.rs`:
```rust
// Populated in Task 3
```

`src/web/mod.rs`:
```rust
// Populated in Phase 4
```

- [ ] **Step 7: Write `.gitignore`**

```
/target
*.db
*.db-journal
*.db-wal
quartermaster.toml
```

- [ ] **Step 8: Write `Justfile`**

```just
default:
    @just --list

build:
    cargo build

check:
    cargo check

test:
    cargo test

clippy:
    cargo clippy -- -D warnings

fmt:
    cargo fmt

lint: fmt clippy

run *ARGS:
    cargo run -- {{ARGS}}
```

- [ ] **Step 9: Verify the project compiles**

```bash
cd ~/code/quartermaster && cargo check
```

Expected: compiles with no errors (may have `unused` warnings from empty modules — that's fine).

- [ ] **Step 10: Verify CLI parses `--help`**

```bash
cargo run -- --help
```

Expected: prints help text with all subcommands listed.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml Cargo.lock src/ Justfile .gitignore
git commit -m "feat: project scaffold with clap CLI skeleton"
```

---

### Task 2: Config System

**Files:**
- Create: `src/config.rs`
- Test: `src/config.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write failing test — deserialize a full TOML config**

In `src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_full_config() {
        let toml_str = r#"
            spt_dir = "/opt/spt"
            forge_token = "abc123"
            queue_changes = false
            auto_drain_on_lifecycle = true
            session_secret = "secret"
            server_container = "spt-server"
            server_host = "192.168.1.10"
            server_port = 6969
            web_bind = "127.0.0.1"
            web_port = 8080
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.spt_dir, Some(std::path::PathBuf::from("/opt/spt")));
        assert_eq!(config.forge_token.as_deref(), Some("abc123"));
        assert!(!config.queue_changes);
        assert!(config.auto_drain_on_lifecycle);
        assert_eq!(config.server_container.as_deref(), Some("spt-server"));
        assert_eq!(config.server_port, Some(6969));
        assert_eq!(config.web_bind, "127.0.0.1");
        assert_eq!(config.web_port, 8080);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test config::tests::deserialize_full_config
```

Expected: FAIL — `Config` type doesn't exist yet.

- [ ] **Step 3: Implement `Config` struct with serde**

```rust
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_web_bind() -> String {
    "0.0.0.0".to_string()
}

fn default_web_port() -> u16 {
    9190
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub spt_dir: Option<PathBuf>,

    #[serde(default)]
    pub forge_token: Option<String>,

    #[serde(default = "default_true")]
    pub queue_changes: bool,

    #[serde(default)]
    pub auto_drain_on_lifecycle: bool,

    #[serde(default)]
    pub session_secret: String,

    #[serde(default)]
    pub server_container: Option<String>,

    #[serde(default)]
    pub server_host: Option<String>,

    #[serde(default)]
    pub server_port: Option<u16>,

    #[serde(default = "default_web_bind")]
    pub web_bind: String,

    #[serde(default = "default_web_port")]
    pub web_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spt_dir: None,
            forge_token: None,
            queue_changes: true,
            auto_drain_on_lifecycle: false,
            session_secret: String::new(),
            server_container: None,
            server_host: None,
            server_port: None,
            web_bind: default_web_bind(),
            web_port: default_web_port(),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test config::tests::deserialize_full_config
```

Expected: PASS

- [ ] **Step 5: Write failing test — defaults when TOML is minimal**

```rust
    #[test]
    fn deserialize_minimal_config() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.spt_dir.is_none());
        assert!(config.forge_token.is_none());
        assert!(config.queue_changes); // default true
        assert!(!config.auto_drain_on_lifecycle); // default false
        assert_eq!(config.web_bind, "0.0.0.0");
        assert_eq!(config.web_port, 9190);
    }
```

Run: `cargo test config::tests::deserialize_minimal_config` — Expected: PASS (defaults are already wired via serde).

- [ ] **Step 6: Write failing test — load from file**

```rust
    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quartermaster.toml");
        std::fs::write(&path, "web_port = 3000\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.web_port, 3000);
        assert!(config.queue_changes); // default
    }
```

- [ ] **Step 7: Implement `Config::load` and `Config::save`**

```rust
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config: {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self)
            .context("failed to serialize config")?;
        std::fs::write(path, contents)
            .with_context(|| format!("failed to write config: {}", path.display()))?;
        Ok(())
    }
}
```

Run: `cargo test config::tests::load_from_file` — Expected: PASS

- [ ] **Step 8: Write failing test — env var overlay**

```rust
    #[test]
    fn env_var_overlay() {
        // NOTE: std::env::set_var is not thread-safe. Run env var tests with
        // `cargo test -- --test-threads=1` or use the `serial_test` crate.
        // SAFETY: test-only, single-threaded execution assumed.
        unsafe {
            std::env::set_var("QUMA_WEB_PORT", "4000");
            std::env::set_var("QUMA_FORGE_TOKEN", "envtoken");
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quartermaster.toml");
        std::fs::write(&path, "web_port = 3000\n").unwrap();

        let config = Config::load_with_env(&path).unwrap();
        assert_eq!(config.web_port, 4000);
        assert_eq!(config.forge_token.as_deref(), Some("envtoken"));

        unsafe {
            std::env::remove_var("QUMA_WEB_PORT");
            std::env::remove_var("QUMA_FORGE_TOKEN");
        }
    }
```

- [ ] **Step 9: Implement `Config::load_with_env`**

```rust
impl Config {
    pub fn load_with_env(path: &Path) -> Result<Self> {
        let mut config = Self::load(path)?;
        config.apply_env_overrides();
        Ok(config)
    }

    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("QUMA_SPT_DIR") {
            self.spt_dir = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("QUMA_FORGE_TOKEN") {
            self.forge_token = Some(v);
        }
        if let Ok(v) = std::env::var("QUMA_WEB_PORT") {
            if let Ok(port) = v.parse() {
                self.web_port = port;
            }
        }
        if let Ok(v) = std::env::var("QUMA_WEB_BIND") {
            self.web_bind = v;
        }
        if let Ok(v) = std::env::var("QUMA_SERVER_CONTAINER") {
            self.server_container = Some(v);
        }
        if let Ok(v) = std::env::var("QUMA_SERVER_HOST") {
            self.server_host = Some(v);
        }
        if let Ok(v) = std::env::var("QUMA_SERVER_PORT") {
            if let Ok(port) = v.parse() {
                self.server_port = Some(port);
            }
        }
    }
}
```

Run: `cargo test config::tests::env_var_overlay` — Expected: PASS

- [ ] **Step 10: Write failing test — generate session secret**

```rust
    #[test]
    fn generate_session_secret_if_empty() {
        let mut config = Config::default();
        assert!(config.session_secret.is_empty());
        config.ensure_session_secret();
        assert!(!config.session_secret.is_empty());
        assert!(config.session_secret.len() >= 32);
    }
```

- [ ] **Step 11: Implement `ensure_session_secret`**

```rust
use rand::Rng;

const SECRET_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

impl Config {
    pub fn ensure_session_secret(&mut self) {
        if self.session_secret.is_empty() {
            let mut rng = rand::rng();
            self.session_secret = (0..48)
                .map(|_| SECRET_ALPHABET[rng.random_range(0..SECRET_ALPHABET.len())] as char)
                .collect();
        }
    }
}
```

Run: `cargo test config::tests::generate_session_secret_if_empty` — Expected: PASS

- [ ] **Step 12: Write failing test — resolve config path**

```rust
    #[test]
    fn resolve_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let spt_dir = dir.path().join("spt");
        std::fs::create_dir_all(&spt_dir).unwrap();

        // Default: config lives at <spt_root>/quartermaster.toml
        let path = Config::resolve_path(None, Some(&spt_dir));
        assert_eq!(path, spt_dir.join("quartermaster.toml"));
    }

    #[test]
    fn resolve_config_path_explicit() {
        let explicit = std::path::PathBuf::from("/tmp/custom.toml");
        let path = Config::resolve_path(Some(&explicit), None);
        assert_eq!(path, explicit);
    }
```

- [ ] **Step 13: Implement `Config::resolve_path`**

```rust
impl Config {
    pub fn resolve_path(
        cli_config: Option<&Path>,
        spt_dir: Option<&Path>,
    ) -> PathBuf {
        // Priority: CLI flag > QUMA_CONFIG env > <spt_root>/quartermaster.toml
        if let Some(p) = cli_config {
            return p.to_path_buf();
        }
        if let Ok(env_path) = std::env::var("QUMA_CONFIG") {
            return PathBuf::from(env_path);
        }
        if let Some(spt) = spt_dir {
            return spt.join("quartermaster.toml");
        }
        PathBuf::from("quartermaster.toml")
    }
}
```

Run: `cargo test config::tests` — Expected: all PASS

- [ ] **Step 14: Commit**

```bash
git add src/config.rs
git commit -m "feat: config system with TOML parsing, env overlay, and secret generation"
```

---

### Task 3: SPT Directory Detection

**Files:**
- Create: `src/spt/mod.rs`
- Create: `src/spt/detect.rs`
- Test: `src/spt/detect.rs` (inline tests)

- [ ] **Step 1: Write `src/spt/mod.rs`**

```rust
pub mod detect;
```

- [ ] **Step 2: Write failing test — validate a valid SPT directory**

In `src/spt/detect.rs`:

```rust
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::error::QumaError;

#[derive(Debug, Clone)]
pub struct SptInfo {
    pub root: PathBuf,
    pub spt_version: String,
    pub tarkov_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_fake_spt_dir(base: &Path) -> PathBuf {
        let spt = base.join("spt");
        std::fs::create_dir_all(spt.join("SPT_Data/Server/configs")).unwrap();
        std::fs::create_dir_all(spt.join("user/mods")).unwrap();
        std::fs::create_dir_all(spt.join("user/profiles")).unwrap();
        std::fs::create_dir_all(spt.join("BepInEx/plugins")).unwrap();
        std::fs::write(spt.join("SPT.Server.exe"), b"").unwrap();
        std::fs::write(
            spt.join("SPT_Data/Server/configs/core.json"),
            r#"{"sptVersion": "4.0.13", "compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();
        spt
    }

    #[test]
    fn validate_valid_spt_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        assert!(validate_spt_dir(&spt).is_ok());
    }
}
```

Run: `cargo test spt::detect::tests::validate_valid_spt_dir` — Expected: FAIL (function doesn't exist).

- [ ] **Step 3: Implement `validate_spt_dir`**

```rust
pub fn validate_spt_dir(path: &Path) -> Result<()> {
    if !path.join("SPT.Server.exe").exists() {
        return Err(QumaError::InvalidSptDir("SPT.Server.exe not found".into()).into());
    }
    if !path.join("SPT_Data/Server/configs/core.json").exists() {
        return Err(
            QumaError::InvalidSptDir("SPT_Data/Server/configs/core.json not found".into()).into(),
        );
    }
    if !path.join("user/mods").is_dir() {
        return Err(QumaError::InvalidSptDir("user/mods/ directory not found".into()).into());
    }
    if !path.join("BepInEx/plugins").is_dir() {
        return Err(
            QumaError::InvalidSptDir("BepInEx/plugins/ directory not found".into()).into(),
        );
    }
    Ok(())
}
```

Run: `cargo test spt::detect::tests::validate_valid_spt_dir` — Expected: PASS

- [ ] **Step 4: Write failing test — reject invalid directory**

```rust
    #[test]
    fn validate_rejects_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(validate_spt_dir(tmp.path()).is_err());
    }

    #[test]
    fn validate_rejects_partial_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // Has SPT.Server.exe but nothing else
        std::fs::write(tmp.path().join("SPT.Server.exe"), b"").unwrap();
        assert!(validate_spt_dir(tmp.path()).is_err());
    }
```

Run: `cargo test spt::detect::tests` — Expected: PASS (validation correctly rejects).

- [ ] **Step 5: Write failing test — read SPT version**

```rust
    #[test]
    fn read_spt_version_from_core_json() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        let info = read_spt_version(&spt).unwrap();
        assert_eq!(info.spt_version, "4.0.13");
        assert_eq!(info.tarkov_version, "0.16.9-40087");
    }
```

- [ ] **Step 6: Implement `read_spt_version`**

```rust
pub fn read_spt_version(spt_dir: &Path) -> Result<SptInfo> {
    let core_path = spt_dir.join("SPT_Data/Server/configs/core.json");
    let contents = std::fs::read_to_string(&core_path)
        .with_context(|| format!("failed to read {}", core_path.display()))?;

    #[derive(serde::Deserialize)]
    struct CoreJson {
        #[serde(alias = "sptVersion")]
        spt_version: String,
        #[serde(alias = "compatibleTarkovVersion")]
        compatible_tarkov_version: String,
    }

    let core: CoreJson = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", core_path.display()))?;

    Ok(SptInfo {
        root: spt_dir.to_path_buf(),
        spt_version: core.spt_version,
        tarkov_version: core.compatible_tarkov_version,
    })
}
```

Run: `cargo test spt::detect::tests::read_spt_version_from_core_json` — Expected: PASS

- [ ] **Step 6b: Write test — malformed core.json is handled gracefully**

```rust
    #[test]
    fn read_spt_version_malformed_json() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        // Overwrite core.json with invalid JSON
        std::fs::write(
            spt.join("SPT_Data/Server/configs/core.json"),
            "not json at all",
        )
        .unwrap();
        let result = read_spt_version(&spt);
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("failed to parse"));
    }

    #[test]
    fn read_spt_version_missing_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        std::fs::write(
            spt.join("SPT_Data/Server/configs/core.json"),
            r#"{"otherField": "value"}"#,
        )
        .unwrap();
        let result = read_spt_version(&spt);
        assert!(result.is_err());
    }
```

Run: `cargo test spt::detect::tests` — Expected: all PASS

- [ ] **Step 7: Write failing test — detect SPT dir from explicit path**

```rust
    #[test]
    fn detect_from_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        let found = detect_spt_dir(Some(&spt), None).unwrap();
        assert_eq!(found, spt);
    }
```

- [ ] **Step 8: Implement `detect_spt_dir`**

```rust
/// Detect the SPT directory. Priority: explicit path > QUMA_SPT_DIR env > walk up from cwd.
pub fn detect_spt_dir(explicit: Option<&Path>, cwd: Option<&Path>) -> Result<PathBuf> {
    // 1. Explicit path
    if let Some(path) = explicit {
        validate_spt_dir(path)?;
        return Ok(path.to_path_buf());
    }

    // 2. Environment variable
    if let Ok(env_path) = std::env::var("QUMA_SPT_DIR") {
        let path = PathBuf::from(&env_path);
        validate_spt_dir(&path)
            .with_context(|| format!("QUMA_SPT_DIR={env_path} is not a valid SPT directory"))?;
        return Ok(path);
    }

    // 3. Walk up from cwd
    let start = match cwd {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().context("failed to get current directory")?,
    };

    let mut dir = start.as_path();
    loop {
        if validate_spt_dir(dir).is_ok() {
            return Ok(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    Err(QumaError::SptDirNotFound.into())
}
```

Run: `cargo test spt::detect::tests::detect_from_explicit_path` — Expected: PASS

- [ ] **Step 9: Write test — detect from cwd walkup**

```rust
    #[test]
    fn detect_from_cwd_walkup() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        // Simulate being inside user/mods/
        let deep = spt.join("user/mods");
        let found = detect_spt_dir(None, Some(&deep)).unwrap();
        assert_eq!(found, spt);
    }

    #[test]
    fn detect_fails_when_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(detect_spt_dir(None, Some(tmp.path())).is_err());
    }
```

Run: `cargo test spt::detect::tests` — Expected: all PASS

- [ ] **Step 10: Implement `read_http_config` for server host/port defaults**

```rust
/// Read server host/port from SPT's http.json config (fallback source for server_host/server_port).
pub fn read_http_config(spt_dir: &Path) -> Option<(String, u16)> {
    let path = spt_dir.join("SPT_Data/Server/configs/http.json");
    let contents = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let ip = json.get("ip")?.as_str()?.to_string();
    let port = json.get("port")?.as_u64()? as u16;
    Some((ip, port))
}
```

Add test:
```rust
    #[test]
    fn read_http_config_parses() {
        let tmp = tempfile::tempdir().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        std::fs::write(
            spt.join("SPT_Data/Server/configs/http.json"),
            r#"{"ip": "0.0.0.0", "port": 6969}"#,
        )
        .unwrap();
        let (ip, port) = read_http_config(&spt).unwrap();
        assert_eq!(ip, "0.0.0.0");
        assert_eq!(port, 6969);
    }

    #[test]
    fn read_http_config_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_http_config(tmp.path()).is_none());
    }
```

Run: `cargo test spt::detect::tests` — Expected: all PASS

- [ ] **Step 11: Commit**

```bash
git add src/spt/
git commit -m "feat: SPT directory detection, validation, and version reading"
```

---

### Task 4: Database Layer

**Files:**
- Create: `src/db/mod.rs`
- Create: `src/db/schema.rs`
- Create: `src/db/mods.rs`
- Create: `src/db/users.rs`
- Create: `migrations/001_initial.sql`
- Test: inline tests in each module

- [ ] **Step 1: Write `migrations/001_initial.sql`**

```sql
CREATE TABLE IF NOT EXISTS installed_mods (
    id INTEGER PRIMARY KEY,
    forge_mod_id INTEGER NOT NULL,
    forge_version_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    slug TEXT,
    version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    updated_at TEXT,
    UNIQUE(forge_mod_id)
);

CREATE TABLE IF NOT EXISTS installed_files (
    id INTEGER PRIMARY KEY,
    mod_id INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    file_hash TEXT,
    file_size INTEGER,
    UNIQUE(file_path)
);

CREATE TABLE IF NOT EXISTS mod_dependencies (
    id INTEGER PRIMARY KEY,
    mod_id INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    depends_on_mod_id INTEGER NOT NULL REFERENCES installed_mods(id),
    version_constraint TEXT,
    UNIQUE(mod_id, depends_on_mod_id)
);

CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    spt_profile_id TEXT NOT NULL,
    password_hash TEXT,
    role TEXT NOT NULL DEFAULT 'player',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS invite_codes (
    id INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    created_by INTEGER REFERENCES users(id),
    used_by INTEGER REFERENCES users(id),
    created_at TEXT NOT NULL,
    used_at TEXT,
    expires_at TEXT
);

CREATE TABLE IF NOT EXISTS pending_operations (
    id INTEGER PRIMARY KEY,
    action TEXT NOT NULL,
    forge_mod_id INTEGER NOT NULL,
    forge_version_id INTEGER,
    mod_name TEXT NOT NULL,
    metadata TEXT,
    queued_at TEXT NOT NULL,
    queued_by TEXT
);
```

- [ ] **Step 2: Write `src/db/schema.rs` — migration runner**

```rust
use anyhow::{Context, Result};
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[(
    "001_initial",
    include_str!("../../migrations/001_initial.sql"),
)];

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("failed to create migrations table")?;

    for (name, sql) in MIGRATIONS {
        let applied: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM _migrations WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .context("failed to check migration status")?;

        if !applied {
            conn.execute_batch(sql)
                .with_context(|| format!("failed to run migration: {name}"))?;
            conn.execute(
                "INSERT INTO _migrations (name) VALUES (?1)",
                [name],
            )
            .with_context(|| format!("failed to record migration: {name}"))?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Write `src/db/mod.rs` — Database struct with connection setup**

```rust
pub mod mods;
pub mod schema;
pub mod users;

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database: {}", path.display()))?;
        Self::configure(&conn)?;
        schema::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory database")?;
        Self::configure(&conn)?;
        schema::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    fn configure(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }
}
```

- [ ] **Step 4: Write failing test — create DB and run migrations**

In `src/db/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_in_memory_db() {
        let db = Database::open_in_memory().unwrap();
        // Verify tables exist by querying sqlite_master
        let tables: Vec<String> = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(tables.contains(&"installed_mods".to_string()));
        assert!(tables.contains(&"installed_files".to_string()));
        assert!(tables.contains(&"mod_dependencies".to_string()));
        assert!(tables.contains(&"users".to_string()));
        assert!(tables.contains(&"invite_codes".to_string()));
        assert!(tables.contains(&"pending_operations".to_string()));
    }
}
```

Run: `cargo test db::tests::create_in_memory_db` — Expected: PASS (schema + migrations already implemented above).

- [ ] **Step 5: Write mod data types and failing test — insert/query installed mod**

In `src/db/mods.rs`:

```rust
use anyhow::{Context, Result};
use rusqlite::params;

use super::Database;

#[derive(Debug, Clone)]
pub struct InstalledMod {
    pub id: Option<i64>,
    pub forge_mod_id: i64,
    pub forge_version_id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub version: String,
    pub installed_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledFile {
    pub id: Option<i64>,
    pub mod_id: i64,
    pub file_path: String,
    pub file_hash: Option<String>,
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ModDependency {
    pub id: Option<i64>,
    pub mod_id: i64,
    pub depends_on_mod_id: i64,
    pub version_constraint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_get_mod() {
        let db = test_db();
        let m = InstalledMod {
            id: None,
            forge_mod_id: 100,
            forge_version_id: 200,
            name: "TestMod".into(),
            slug: Some("test-mod".into()),
            version: "1.0.0".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: None,
        };
        let id = db.insert_mod(&m).unwrap();
        let fetched = db.get_mod(id).unwrap().unwrap();
        assert_eq!(fetched.name, "TestMod");
        assert_eq!(fetched.forge_mod_id, 100);
        assert_eq!(fetched.version, "1.0.0");
    }
}
```

Run: `cargo test db::mods::tests::insert_and_get_mod` — Expected: FAIL (methods don't exist).

- [ ] **Step 6: Implement mod CRUD**

```rust
impl Database {
    pub fn insert_mod(&self, m: &InstalledMod) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO installed_mods (forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    m.forge_mod_id,
                    m.forge_version_id,
                    m.name,
                    m.slug,
                    m.version,
                    m.installed_at,
                    m.updated_at,
                ],
            )
            .context("failed to insert mod")?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_mod(&self, id: i64) -> Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
                 FROM installed_mods WHERE id = ?1",
                [id],
                |row| {
                    Ok(InstalledMod {
                        id: Some(row.get(0)?),
                        forge_mod_id: row.get(1)?,
                        forge_version_id: row.get(2)?,
                        name: row.get(3)?,
                        slug: row.get(4)?,
                        version: row.get(5)?,
                        installed_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("failed to get mod")
    }

    pub fn get_mod_by_forge_id(&self, forge_mod_id: i64) -> Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
                 FROM installed_mods WHERE forge_mod_id = ?1",
                [forge_mod_id],
                |row| {
                    Ok(InstalledMod {
                        id: Some(row.get(0)?),
                        forge_mod_id: row.get(1)?,
                        forge_version_id: row.get(2)?,
                        name: row.get(3)?,
                        slug: row.get(4)?,
                        version: row.get(5)?,
                        installed_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("failed to get mod by forge id")
    }

    pub fn list_mods(&self) -> Result<Vec<InstalledMod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
             FROM installed_mods ORDER BY name",
        )?;
        let mods = stmt
            .query_map([], |row| {
                Ok(InstalledMod {
                    id: Some(row.get(0)?),
                    forge_mod_id: row.get(1)?,
                    forge_version_id: row.get(2)?,
                    name: row.get(3)?,
                    slug: row.get(4)?,
                    version: row.get(5)?,
                    installed_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to list mods")?;
        Ok(mods)
    }

    pub fn update_mod(&self, m: &InstalledMod) -> Result<()> {
        self.conn.execute(
            "UPDATE installed_mods SET forge_version_id = ?1, version = ?2, updated_at = ?3
             WHERE id = ?4",
            params![m.forge_version_id, m.version, m.updated_at, m.id],
        )?;
        Ok(())
    }

    pub fn delete_mod(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM installed_mods WHERE id = ?1", [id])?;
        Ok(())
    }
}
```

Add the `use rusqlite::OptionalExtension;` import at the top of `src/db/mods.rs` (needed for `.optional()`).

Run: `cargo test db::mods::tests::insert_and_get_mod` — Expected: PASS

- [ ] **Step 6b: Write additional constraint and cascade tests**

```rust
    #[test]
    fn duplicate_forge_mod_id_rejected() {
        let db = test_db();
        let m = InstalledMod {
            id: None,
            forge_mod_id: 100,
            forge_version_id: 200,
            name: "Mod1".into(),
            slug: None,
            version: "1.0.0".into(),
            installed_at: "2026-01-01T00:00:00Z".into(),
            updated_at: None,
        };
        db.insert_mod(&m).unwrap();
        let m2 = InstalledMod { name: "Mod2".into(), ..m };
        assert!(db.insert_mod(&m2).is_err());
    }

    #[test]
    fn delete_mod_cascades_to_files_and_deps() {
        let db = test_db();
        let (base_id, dep_id) = insert_two_mods(&db);
        db.insert_file(&InstalledFile {
            id: None,
            mod_id: base_id,
            file_path: "user/mods/BaseMod/base.dll".into(),
            file_hash: None,
            file_size: None,
        })
        .unwrap();
        db.insert_dependency(&ModDependency {
            id: None,
            mod_id: dep_id,
            depends_on_mod_id: base_id,
            version_constraint: None,
        })
        .unwrap();

        db.delete_mod(base_id).unwrap();

        assert!(db.get_files_for_mod(base_id).unwrap().is_empty());
        // dep_id's dependency on base_id should also be gone (CASCADE on depends_on_mod_id
        // is not set — only mod_id cascades). Verify this behavior.
        let rdeps = db.get_reverse_dependencies(base_id).unwrap();
        assert!(rdeps.is_empty());
    }
```

Run: `cargo test db::mods::tests` — Expected: all PASS. Note: the cascade test verifies `ON DELETE CASCADE` on `installed_files.mod_id`. The `mod_dependencies` table has CASCADE on `mod_id` but a plain FK on `depends_on_mod_id` — deleting the depended-on mod will fail with a foreign key constraint error unless the dependency is removed first. The implementing agent should verify this behavior and consider adding `ON DELETE CASCADE` to `depends_on_mod_id` if needed.

- [ ] **Step 7: Write tests and implement file tracking CRUD**

Tests (add to `src/db/mods.rs` tests):

```rust
    #[test]
    fn insert_and_get_files() {
        let db = test_db();
        let mod_id = db
            .insert_mod(&InstalledMod {
                id: None,
                forge_mod_id: 100,
                forge_version_id: 200,
                name: "TestMod".into(),
                slug: None,
                version: "1.0.0".into(),
                installed_at: "2026-01-01T00:00:00Z".into(),
                updated_at: None,
            })
            .unwrap();

        db.insert_file(&InstalledFile {
            id: None,
            mod_id,
            file_path: "user/mods/TestMod/test.dll".into(),
            file_hash: Some("abc123".into()),
            file_size: Some(1024),
        })
        .unwrap();

        let files = db.get_files_for_mod(mod_id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "user/mods/TestMod/test.dll");
        assert_eq!(files[0].file_hash.as_deref(), Some("abc123"));
    }

    #[test]
    fn file_path_unique_constraint() {
        let db = test_db();
        let mod_id = db
            .insert_mod(&InstalledMod {
                id: None,
                forge_mod_id: 100,
                forge_version_id: 200,
                name: "Mod1".into(),
                slug: None,
                version: "1.0.0".into(),
                installed_at: "2026-01-01T00:00:00Z".into(),
                updated_at: None,
            })
            .unwrap();
        db.insert_file(&InstalledFile {
            id: None,
            mod_id,
            file_path: "user/mods/shared.dll".into(),
            file_hash: None,
            file_size: None,
        })
        .unwrap();
        // Inserting same path again should fail
        assert!(db
            .insert_file(&InstalledFile {
                id: None,
                mod_id,
                file_path: "user/mods/shared.dll".into(),
                file_hash: None,
                file_size: None,
            })
            .is_err());
    }
```

Implementation (add to `impl Database` in `src/db/mods.rs`):

```rust
    pub fn insert_file(&self, f: &InstalledFile) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO installed_files (mod_id, file_path, file_hash, file_size)
                 VALUES (?1, ?2, ?3, ?4)",
                params![f.mod_id, f.file_path, f.file_hash, f.file_size],
            )
            .context("failed to insert file")?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_files_for_mod(&self, mod_id: i64) -> Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, file_path, file_hash, file_size
             FROM installed_files WHERE mod_id = ?1 ORDER BY file_path",
        )?;
        let files = stmt
            .query_map([mod_id], |row| {
                Ok(InstalledFile {
                    id: Some(row.get(0)?),
                    mod_id: row.get(1)?,
                    file_path: row.get(2)?,
                    file_hash: row.get(3)?,
                    file_size: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get files for mod")?;
        Ok(files)
    }

    pub fn delete_files_for_mod(&self, mod_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM installed_files WHERE mod_id = ?1", [mod_id])?;
        Ok(())
    }

    pub fn get_all_tracked_files(&self) -> Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, file_path, file_hash, file_size
             FROM installed_files ORDER BY file_path",
        )?;
        let files = stmt
            .query_map([], |row| {
                Ok(InstalledFile {
                    id: Some(row.get(0)?),
                    mod_id: row.get(1)?,
                    file_path: row.get(2)?,
                    file_hash: row.get(3)?,
                    file_size: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to get all tracked files")?;
        Ok(files)
    }
```

Run: `cargo test db::mods::tests` — Expected: all PASS

- [ ] **Step 8: Write tests and implement dependency CRUD**

Tests (add to `src/db/mods.rs` tests):

```rust
    fn insert_two_mods(db: &Database) -> (i64, i64) {
        let id1 = db
            .insert_mod(&InstalledMod {
                id: None,
                forge_mod_id: 100,
                forge_version_id: 200,
                name: "BaseMod".into(),
                slug: None,
                version: "1.0.0".into(),
                installed_at: "2026-01-01T00:00:00Z".into(),
                updated_at: None,
            })
            .unwrap();
        let id2 = db
            .insert_mod(&InstalledMod {
                id: None,
                forge_mod_id: 101,
                forge_version_id: 201,
                name: "DependentMod".into(),
                slug: None,
                version: "2.0.0".into(),
                installed_at: "2026-01-01T00:00:00Z".into(),
                updated_at: None,
            })
            .unwrap();
        (id1, id2)
    }

    #[test]
    fn insert_and_query_dependency() {
        let db = test_db();
        let (base_id, dep_id) = insert_two_mods(&db);
        db.insert_dependency(&ModDependency {
            id: None,
            mod_id: dep_id,
            depends_on_mod_id: base_id,
            version_constraint: Some(">=1.0.0".into()),
        })
        .unwrap();
        let deps = db.get_dependencies(dep_id).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].depends_on_mod_id, base_id);
    }

    #[test]
    fn reverse_dependencies() {
        let db = test_db();
        let (base_id, dep_id) = insert_two_mods(&db);
        db.insert_dependency(&ModDependency {
            id: None,
            mod_id: dep_id,
            depends_on_mod_id: base_id,
            version_constraint: None,
        })
        .unwrap();
        let rdeps = db.get_reverse_dependencies(base_id).unwrap();
        assert_eq!(rdeps.len(), 1);
        assert_eq!(rdeps[0].mod_id, dep_id);
    }
```

Implementation:

```rust
    pub fn insert_dependency(&self, dep: &ModDependency) -> Result<()> {
        self.conn.execute(
            "INSERT INTO mod_dependencies (mod_id, depends_on_mod_id, version_constraint)
             VALUES (?1, ?2, ?3)",
            params![dep.mod_id, dep.depends_on_mod_id, dep.version_constraint],
        )?;
        Ok(())
    }

    pub fn get_dependencies(&self, mod_id: i64) -> Result<Vec<ModDependency>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, depends_on_mod_id, version_constraint
             FROM mod_dependencies WHERE mod_id = ?1",
        )?;
        let deps = stmt
            .query_map([mod_id], |row| {
                Ok(ModDependency {
                    id: Some(row.get(0)?),
                    mod_id: row.get(1)?,
                    depends_on_mod_id: row.get(2)?,
                    version_constraint: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(deps)
    }

    pub fn get_reverse_dependencies(&self, mod_id: i64) -> Result<Vec<ModDependency>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, depends_on_mod_id, version_constraint
             FROM mod_dependencies WHERE depends_on_mod_id = ?1",
        )?;
        let deps = stmt
            .query_map([mod_id], |row| {
                Ok(ModDependency {
                    id: Some(row.get(0)?),
                    mod_id: row.get(1)?,
                    depends_on_mod_id: row.get(2)?,
                    version_constraint: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(deps)
    }

    pub fn delete_dependencies_for_mod(&self, mod_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM mod_dependencies WHERE mod_id = ?1", [mod_id])?;
        Ok(())
    }
```

Run: `cargo test db::mods::tests` — Expected: all PASS

- [ ] **Step 9: Write `src/db/users.rs` — user and invite data types**

```rust
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone)]
pub struct User {
    pub id: Option<i64>,
    pub username: String,
    pub spt_profile_id: String,
    pub password_hash: Option<String>,
    pub role: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct InviteCode {
    pub id: Option<i64>,
    pub code: String,
    pub created_by: Option<i64>,
    pub used_by: Option<i64>,
    pub created_at: String,
    pub used_at: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingOperation {
    pub id: Option<i64>,
    pub action: String,
    pub forge_mod_id: i64,
    pub forge_version_id: Option<i64>,
    pub mod_name: String,
    pub metadata: Option<String>,
    pub queued_at: String,
    pub queued_by: Option<String>,
}
```

- [ ] **Step 10: Write tests and implement user CRUD**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_get_user() {
        let db = test_db();
        let id = db
            .insert_user(&User {
                id: None,
                username: "player1".into(),
                spt_profile_id: "abc123".into(),
                password_hash: Some("hashed".into()),
                role: "admin".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            })
            .unwrap();
        let user = db.get_user_by_username("player1").unwrap().unwrap();
        assert_eq!(user.id, Some(id));
        assert_eq!(user.role, "admin");
    }

    #[test]
    fn admin_exists_check() {
        let db = test_db();
        assert!(!db.admin_exists().unwrap());
        db.insert_user(&User {
            id: None,
            username: "admin".into(),
            spt_profile_id: "abc".into(),
            password_hash: Some("hash".into()),
            role: "admin".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        })
        .unwrap();
        assert!(db.admin_exists().unwrap());
    }
}
```

Implementation:

```rust
impl Database {
    pub fn insert_user(&self, user: &User) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO users (username, spt_profile_id, password_hash, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                user.username,
                user.spt_profile_id,
                user.password_hash,
                user.role,
                user.created_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, created_at
                 FROM users WHERE username = ?1",
                [username],
                |row| {
                    Ok(User {
                        id: Some(row.get(0)?),
                        username: row.get(1)?,
                        spt_profile_id: row.get(2)?,
                        password_hash: row.get(3)?,
                        role: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .context("failed to get user")
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, username, spt_profile_id, password_hash, role, created_at
             FROM users ORDER BY username",
        )?;
        let users = stmt
            .query_map([], |row| {
                Ok(User {
                    id: Some(row.get(0)?),
                    username: row.get(1)?,
                    spt_profile_id: row.get(2)?,
                    password_hash: row.get(3)?,
                    role: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(users)
    }

    pub fn admin_exists(&self) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin'",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
```

Run: `cargo test db::users::tests` — Expected: all PASS

- [ ] **Step 11: Write tests and implement invite code + pending operations CRUD**

Add to `src/db/users.rs`:

Tests:
```rust
    #[test]
    fn create_and_use_invite() {
        let db = test_db();
        db.create_invite("quma-abc123", None, None).unwrap();
        let invite = db.get_invite("quma-abc123").unwrap().unwrap();
        assert!(invite.used_by.is_none());

        let used = db.use_invite("quma-abc123", 1).unwrap();
        assert!(used);

        let invite = db.get_invite("quma-abc123").unwrap().unwrap();
        assert_eq!(invite.used_by, Some(1));

        // Can't reuse
        let used = db.use_invite("quma-abc123", 2).unwrap();
        assert!(!used);
    }

    #[test]
    fn expired_invite_rejected() {
        let db = test_db();
        // Create invite that expired in the past
        db.create_invite("quma-expired", None, Some("2020-01-01T00:00:00Z")).unwrap();
        let used = db.use_invite("quma-expired", 1).unwrap();
        assert!(!used);
    }

    #[test]
    fn pending_operations_crud() {
        let db = test_db();
        let id = db
            .insert_pending_op(&PendingOperation {
                id: None,
                action: "install".into(),
                forge_mod_id: 100,
                forge_version_id: Some(200),
                mod_name: "TestMod".into(),
                metadata: Some(r#"{"deps":[]}"#.into()),
                queued_at: "2026-01-01T00:00:00Z".into(),
                queued_by: Some("admin".into()),
            })
            .unwrap();
        let ops = db.list_pending_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, "install");

        db.delete_pending_op(id).unwrap();
        assert!(db.list_pending_ops().unwrap().is_empty());
    }
```

Implementation:
```rust
impl Database {
    pub fn create_invite(
        &self,
        code: &str,
        created_by: Option<i64>,
        expires_at: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO invite_codes (code, created_by, created_at, expires_at)
             VALUES (?1, ?2, datetime('now'), ?3)",
            params![code, created_by, expires_at],
        )?;
        Ok(())
    }

    pub fn get_invite(&self, code: &str) -> Result<Option<InviteCode>> {
        self.conn
            .query_row(
                "SELECT id, code, created_by, used_by, created_at, used_at, expires_at
                 FROM invite_codes WHERE code = ?1",
                [code],
                |row| {
                    Ok(InviteCode {
                        id: Some(row.get(0)?),
                        code: row.get(1)?,
                        created_by: row.get(2)?,
                        used_by: row.get(3)?,
                        created_at: row.get(4)?,
                        used_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .context("failed to get invite code")
    }

    pub fn use_invite(&self, code: &str, user_id: i64) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE invite_codes SET used_by = ?1, used_at = datetime('now')
             WHERE code = ?2 AND used_by IS NULL
             AND (expires_at IS NULL OR expires_at > datetime('now'))",
            params![user_id, code],
        )?;
        Ok(rows > 0)
    }

    pub fn insert_pending_op(&self, op: &PendingOperation) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_operations (action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op.action,
                op.forge_mod_id,
                op.forge_version_id,
                op.mod_name,
                op.metadata,
                op.queued_at,
                op.queued_by,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_pending_ops(&self) -> Result<Vec<PendingOperation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by
             FROM pending_operations ORDER BY id",
        )?;
        let ops = stmt
            .query_map([], |row| {
                Ok(PendingOperation {
                    id: Some(row.get(0)?),
                    action: row.get(1)?,
                    forge_mod_id: row.get(2)?,
                    forge_version_id: row.get(3)?,
                    mod_name: row.get(4)?,
                    metadata: row.get(5)?,
                    queued_at: row.get(6)?,
                    queued_by: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ops)
    }

    pub fn delete_pending_op(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM pending_operations WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_pending_ops(&self) -> Result<()> {
        self.conn.execute("DELETE FROM pending_operations", [])?;
        Ok(())
    }
}
```

Run: `cargo test db::` — Expected: all PASS

- [ ] **Step 12: Commit**

```bash
git add src/db/ migrations/
git commit -m "feat: SQLite database layer with CRUD for mods, files, users, invites, and pending ops"
```

---

### Task 5: Forge API Client

**Files:**
- Create: `src/forge/mod.rs`
- Create: `src/forge/models.rs`
- Create: `src/forge/client.rs`
- Test: inline tests in `models.rs` and `client.rs`

- [ ] **Step 1: Write `src/forge/mod.rs`**

```rust
pub mod client;
pub mod models;
```

- [ ] **Step 2: Write failing test — deserialize mod JSON from Forge API**

In `src/forge/models.rs`:

```rust
use serde::Deserialize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_forge_mod() {
        let json = r#"{
            "id": 2326,
            "name": "Fika",
            "slug": "fika",
            "description": "A multiplayer mod for SPT",
            "fika_compatibility": true,
            "versions": []
        }"#;
        let m: ForgeMod = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, 2326);
        assert_eq!(m.name, "Fika");
        assert_eq!(m.slug.as_deref(), Some("fika"));
        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));
    }
}
```

- [ ] **Step 3: Implement Forge API models**

```rust
use serde::{de, Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq)]
pub enum FikaCompat {
    Compatible,
    Incompatible,
    Unknown,
}

impl<'de> Deserialize<'de> for FikaCompat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawFikaCompat {
            Bool(bool),
            Str(String),
        }

        match RawFikaCompat::deserialize(deserializer)? {
            RawFikaCompat::Bool(true) => Ok(FikaCompat::Compatible),
            RawFikaCompat::Bool(false) => Ok(FikaCompat::Incompatible),
            RawFikaCompat::Str(s) => match s.as_str() {
                "compatible" => Ok(FikaCompat::Compatible),
                "incompatible" => Ok(FikaCompat::Incompatible),
                _ => Ok(FikaCompat::Unknown),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeMod {
    pub id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub fika_compatibility: Option<FikaCompat>,
    #[serde(default)]
    pub versions: Option<Vec<ForgeVersion>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeVersion {
    pub id: i64,
    pub version: String,
    pub spt_version: Option<String>,
    pub link: Option<String>,
    pub content_length: Option<u64>,
    #[serde(default)]
    pub fika_compatibility: Option<FikaCompat>,
    #[serde(default)]
    pub dependencies: Option<Vec<ForgeDependency>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeDependency {
    pub mod_id: i64,
    pub version_id: Option<i64>,
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeSearchResponse {
    pub data: Vec<ForgeMod>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeModResponse {
    pub data: ForgeMod,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeVersionsResponse {
    pub data: Vec<ForgeVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DependencyNode {
    pub mod_id: i64,
    pub version_id: i64,
    pub name: String,
    pub version: String,
    pub resolved_dependencies: Option<Vec<DependencyNode>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCheckResult {
    pub mod_id: i64,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub latest_version_id: Option<i64>,
    pub status: String, // "updated", "blocked", "up_to_date", "incompatible"
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatesResponse {
    pub data: Vec<UpdateCheckResult>,
}
```

Run: `cargo test forge::models::tests::deserialize_forge_mod` — Expected: PASS

- [ ] **Step 4: Write tests for FikaCompat deserialization edge cases**

```rust
    #[test]
    fn fika_compat_from_bool() {
        let json_true = r#"{"id":1,"name":"A","fika_compatibility":true}"#;
        let json_false = r#"{"id":1,"name":"A","fika_compatibility":false}"#;
        let m_true: ForgeMod = serde_json::from_str(json_true).unwrap();
        let m_false: ForgeMod = serde_json::from_str(json_false).unwrap();
        assert_eq!(m_true.fika_compatibility, Some(FikaCompat::Compatible));
        assert_eq!(m_false.fika_compatibility, Some(FikaCompat::Incompatible));
    }

    #[test]
    fn fika_compat_from_string() {
        let json = r#"{"id":1,"version":"1.0","fika_compatibility":"compatible"}"#;
        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Compatible));
    }

    #[test]
    fn fika_compat_unknown_string() {
        let json = r#"{"id":1,"version":"1.0","fika_compatibility":"unknown"}"#;
        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Unknown));
    }

    #[test]
    fn fika_compat_missing() {
        let json = r#"{"id":1,"name":"A"}"#;
        let m: ForgeMod = serde_json::from_str(json).unwrap();
        assert_eq!(m.fika_compatibility, None);
    }
```

Run: `cargo test forge::models::tests` — Expected: all PASS

- [ ] **Step 5: Write `src/forge/client.rs` with ForgeClient struct**

```rust
use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use super::models::*;

const FORGE_BASE_URL: &str = "https://forge.sp-tarkov.com/api/v0";

pub struct ForgeClient {
    client: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl ForgeClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(ref t) = token {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {t}"))
                    .context("invalid forge token")?,
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(format!("quartermaster/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url: FORGE_BASE_URL.to_string(),
            token,
        })
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: String, token: Option<String>) -> Result<Self> {
        let mut c = Self::new(token)?;
        c.base_url = base_url;
        Ok(c)
    }

    pub async fn search_mods(&self, query: &str) -> Result<Vec<ForgeMod>> {
        let url = format!("{}/mods?query={}", self.base_url, query);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("forge search request failed")?;

        if !resp.status().is_success() {
            bail!("forge search failed: HTTP {}", resp.status());
        }

        let body: ForgeSearchResponse = resp.json().await.context("failed to parse search response")?;
        Ok(body.data)
    }

    pub async fn get_mod(&self, id: i64, include_versions: bool) -> Result<ForgeMod> {
        let mut url = format!("{}/mod/{}", self.base_url, id);
        if include_versions {
            url.push_str("?include=versions");
        }
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("forge get mod request failed")?;

        if !resp.status().is_success() {
            bail!("forge get mod failed: HTTP {}", resp.status());
        }

        let body: ForgeModResponse = resp.json().await.context("failed to parse mod response")?;
        Ok(body.data)
    }

    pub async fn get_versions(
        &self,
        mod_id: i64,
        spt_version: Option<&str>,
    ) -> Result<Vec<ForgeVersion>> {
        let mut url = format!("{}/mod/{}/versions", self.base_url, mod_id);
        if let Some(sv) = spt_version {
            url.push_str(&format!("?filter[spt_version]={}", sv));
        }
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("forge get versions request failed")?;

        if !resp.status().is_success() {
            bail!("forge get versions failed: HTTP {}", resp.status());
        }

        let body: ForgeVersionsResponse =
            resp.json().await.context("failed to parse versions response")?;
        Ok(body.data)
    }

    pub async fn get_dependencies(
        &self,
        mods: &[(i64, i64)],
    ) -> Result<Vec<DependencyNode>> {
        let url = format!("{}/mods/dependencies", self.base_url);
        let body: Vec<serde_json::Value> = mods
            .iter()
            .map(|(mod_id, version_id)| {
                serde_json::json!({"mod_id": mod_id, "version_id": version_id})
            })
            .collect();
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("forge dependencies request failed")?;

        if !resp.status().is_success() {
            bail!("forge dependencies failed: HTTP {}", resp.status());
        }

        let result: Vec<DependencyNode> =
            resp.json().await.context("failed to parse dependencies response")?;
        Ok(result)
    }

    pub async fn check_updates(
        &self,
        mods: &[(i64, String)],
        spt_version: &str,
    ) -> Result<Vec<UpdateCheckResult>> {
        let url = format!("{}/mods/updates", self.base_url);
        let body = serde_json::json!({
            "spt_version": spt_version,
            "mods": mods.iter().map(|(id, ver)| {
                serde_json::json!({"mod_id": id, "version": ver})
            }).collect::<Vec<_>>(),
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("forge updates request failed")?;

        if !resp.status().is_success() {
            bail!("forge update check failed: HTTP {}", resp.status());
        }

        let result: UpdatesResponse =
            resp.json().await.context("failed to parse updates response")?;
        Ok(result.data)
    }

    pub async fn download_file(&self, url: &str, dest: &std::path::Path) -> Result<()> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("download request failed")?;

        if !resp.status().is_success() {
            bail!("download failed: HTTP {}", resp.status());
        }

        let bytes = resp.bytes().await.context("failed to read download body")?;
        std::fs::write(dest, &bytes)
            .with_context(|| format!("failed to write to {}", dest.display()))?;
        Ok(())
    }
}
```

- [ ] **Step 6: Write tests — model deserialization from realistic JSON fixtures**

Add to `src/forge/models.rs` tests:

```rust
    #[test]
    fn deserialize_version_with_all_fields() {
        let json = r#"{
            "id": 500,
            "version": "2.3.1",
            "spt_version": "4.0.13",
            "link": "https://forge.sp-tarkov.com/files/mod.zip",
            "content_length": 1048576,
            "fika_compatibility": "compatible",
            "dependencies": [
                {"mod_id": 100, "version_id": 150, "name": "BaseMod", "version": "1.0.0"}
            ]
        }"#;
        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, 500);
        assert_eq!(v.version, "2.3.1");
        assert_eq!(v.spt_version.as_deref(), Some("4.0.13"));
        assert_eq!(v.link.as_deref(), Some("https://forge.sp-tarkov.com/files/mod.zip"));
        assert_eq!(v.content_length, Some(1048576));
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Compatible));
        let deps = v.dependencies.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].mod_id, 100);
    }

    #[test]
    fn deserialize_abbreviated_version() {
        let json = r#"{
            "id": 500,
            "version": "2.3.1",
            "spt_version": "4.0.13"
        }"#;
        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, 500);
        assert!(v.link.is_none());
        assert!(v.content_length.is_none());
        assert!(v.fika_compatibility.is_none());
    }

    #[test]
    fn deserialize_dependency_node() {
        let json = r#"{
            "mod_id": 100,
            "version_id": 150,
            "name": "BaseMod",
            "version": "1.0.0",
            "resolved_dependencies": [
                {"mod_id": 200, "version_id": 250, "name": "SubMod", "version": "0.5.0", "resolved_dependencies": []}
            ]
        }"#;
        let node: DependencyNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.mod_id, 100);
        let children = node.resolved_dependencies.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "SubMod");
    }
```

Run: `cargo test forge::` — Expected: all PASS

- [ ] **Step 6b: Verify Forge API HTTP methods against live API**

The spec says `GET /mods/updates` and `GET /mods/dependencies` but the implementation uses `POST` (since these endpoints require a request body with mod/version pairs). Before committing, verify the correct HTTP methods by testing against the live Forge API:

```bash
# Test dependencies endpoint
curl -X POST https://forge.sp-tarkov.com/api/v0/mods/dependencies \
  -H 'Content-Type: application/json' \
  -d '[{"mod_id": 2326, "version_id": 1}]'

# Test updates endpoint  
curl -X POST https://forge.sp-tarkov.com/api/v0/mods/updates \
  -H 'Content-Type: application/json' \
  -d '{"spt_version": "4.0.13", "mods": [{"mod_id": 2326, "version": "1.0.0"}]}'
```

If either uses GET, update the client accordingly. The spec table may be inaccurate.

- [ ] **Step 7: Commit**

```bash
git add src/forge/
git commit -m "feat: Forge API client with models, search, versions, dependencies, and update checks"
```

---

### Task 6: Archive Handling

**Files:**
- Create: `src/spt/mods.rs`
- Modify: `src/spt/mod.rs` (add `pub mod mods;`)
- Test: inline tests in `src/spt/mods.rs`

- [ ] **Step 1: Update `src/spt/mod.rs`**

```rust
pub mod detect;
pub mod mods;
```

- [ ] **Step 2: Write failing test — detect server mod from archive**

In `src/spt/mods.rs`:

```rust
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq)]
pub enum ModType {
    Server,
    Client,
    Hybrid,
    Ambiguous,
}

#[derive(Debug, Clone)]
pub struct ExtractedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn create_test_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let buf = std::io::Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(content).unwrap();
        }
        let buf = zip.finish().unwrap();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(buf.get_ref()).unwrap();
        tmp.flush().unwrap();
        tmp
    }

    #[test]
    fn detect_server_mod() {
        let zip_file = create_test_zip(&[
            ("user/mods/TestMod/TestMod.dll", b"fake dll"),
            ("user/mods/TestMod/config.json", b"{}"),
        ]);
        let mod_type = detect_mod_type(zip_file.path()).unwrap();
        assert_eq!(mod_type, ModType::Server);
    }
}
```

Run: `cargo test spt::mods::tests::detect_server_mod` — Expected: FAIL (function doesn't exist).

- [ ] **Step 3: Implement `detect_mod_type`**

```rust
pub fn detect_mod_type(archive_path: &Path) -> Result<ModType> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("failed to read ZIP archive")?;

    let mut has_user_mods = false;
    let mut has_bepinex = false;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).context("failed to read ZIP entry")?;
        let name = match entry.name() {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };

        if name.starts_with("user/mods/") || name.starts_with("user\\mods\\") {
            has_user_mods = true;
        }
        if name.starts_with("BepInEx/plugins/") || name.starts_with("BepInEx\\plugins\\") {
            has_bepinex = true;
        }
    }

    match (has_user_mods, has_bepinex) {
        (true, true) => Ok(ModType::Hybrid),
        (true, false) => Ok(ModType::Server),
        (false, true) => Ok(ModType::Client),
        (false, false) => Ok(ModType::Ambiguous),
    }
}
```

Run: `cargo test spt::mods::tests::detect_server_mod` — Expected: PASS

- [ ] **Step 4: Write tests for other mod type detections**

```rust
    #[test]
    fn detect_client_mod() {
        let zip_file = create_test_zip(&[
            ("BepInEx/plugins/ClientMod/ClientMod.dll", b"fake dll"),
        ]);
        let mod_type = detect_mod_type(zip_file.path()).unwrap();
        assert_eq!(mod_type, ModType::Client);
    }

    #[test]
    fn detect_hybrid_mod() {
        let zip_file = create_test_zip(&[
            ("user/mods/HybridMod/server.dll", b"server"),
            ("BepInEx/plugins/HybridMod/client.dll", b"client"),
        ]);
        let mod_type = detect_mod_type(zip_file.path()).unwrap();
        assert_eq!(mod_type, ModType::Hybrid);
    }

    #[test]
    fn detect_ambiguous_mod() {
        let zip_file = create_test_zip(&[
            ("SomeMod.dll", b"dll"),
            ("config.json", b"{}"),
        ]);
        let mod_type = detect_mod_type(zip_file.path()).unwrap();
        assert_eq!(mod_type, ModType::Ambiguous);
    }
```

Run: `cargo test spt::mods::tests` — Expected: all PASS

- [ ] **Step 4b: Write test and implement top-level directory stripping**

Many mod archives wrap everything in a top-level directory matching the mod name (e.g., `SAIN/user/mods/SAIN/...`). This needs to be stripped before extraction.

```rust
/// If all entries share a single top-level directory prefix that doesn't match
/// a known SPT path, strip it. Returns the prefix to strip (empty string if none).
pub fn detect_strip_prefix(archive_path: &Path) -> Result<String> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("failed to read ZIP archive")?;

    let known_prefixes = ["user/", "BepInEx/"];
    let mut top_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i).context("failed to read ZIP entry")?;
        let name = match entry.name() {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };

        // If any entry already starts with a known prefix, no stripping needed
        if known_prefixes.iter().any(|p| name.starts_with(p)) {
            return Ok(String::new());
        }

        // Track top-level directory names
        if let Some(slash_pos) = name.find('/') {
            top_dirs.insert(name[..=slash_pos].to_string());
        }
    }

    // If exactly one top-level directory, strip it
    if top_dirs.len() == 1 {
        return Ok(top_dirs.into_iter().next().unwrap_or_default());
    }

    Ok(String::new())
}
```

Test:
```rust
    #[test]
    fn strip_top_level_wrapper_dir() {
        let zip_file = create_test_zip(&[
            ("SAIN/user/mods/SAIN/SAIN.dll", b"dll"),
            ("SAIN/user/mods/SAIN/config.json", b"{}"),
        ]);
        let prefix = detect_strip_prefix(zip_file.path()).unwrap();
        assert_eq!(prefix, "SAIN/");
    }

    #[test]
    fn no_strip_when_known_prefix() {
        let zip_file = create_test_zip(&[
            ("user/mods/SAIN/SAIN.dll", b"dll"),
        ]);
        let prefix = detect_strip_prefix(zip_file.path()).unwrap();
        assert_eq!(prefix, "");
    }
```

Then update `extract_mod` to accept and apply the strip prefix:

Update the `extract_mod` signature to:
```rust
pub fn extract_mod(archive_path: &Path, spt_root: &Path) -> Result<Vec<ExtractedFile>> {
    let strip_prefix = detect_strip_prefix(archive_path)?;
    // ... in the extraction loop, strip the prefix from entry names:
    // let name = name.strip_prefix(&strip_prefix).unwrap_or(&name).to_string();
```

Run: `cargo test spt::mods::tests` — Expected: all PASS

- [ ] **Step 5: Write failing test — extract mod to SPT directory**

```rust
    #[test]
    fn extract_server_mod() {
        let zip_file = create_test_zip(&[
            ("user/mods/TestMod/TestMod.dll", b"fake dll content"),
            ("user/mods/TestMod/config.json", b"{\"key\":\"value\"}"),
        ]);
        let spt_root = tempfile::tempdir().unwrap();
        fs::create_dir_all(spt_root.path().join("user/mods")).unwrap();

        let files = extract_mod(zip_file.path(), spt_root.path()).unwrap();
        assert_eq!(files.len(), 2);

        // Verify files exist on disk
        assert!(spt_root
            .path()
            .join("user/mods/TestMod/TestMod.dll")
            .exists());
        assert!(spt_root
            .path()
            .join("user/mods/TestMod/config.json")
            .exists());

        // Verify hashes are populated
        assert!(!files[0].hash.is_empty());
        assert!(files[0].size > 0);
    }
```

- [ ] **Step 6: Implement `extract_mod`**

```rust
pub fn extract_mod(archive_path: &Path, spt_root: &Path) -> Result<Vec<ExtractedFile>> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open archive: {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("failed to read ZIP archive")?;
    let mut extracted = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("failed to read ZIP entry")?;
        let name = match entry.name() {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };

        // Skip directories
        if entry.is_dir() {
            let dir_path = spt_root.join(&name);
            fs::create_dir_all(&dir_path)
                .with_context(|| format!("failed to create directory: {}", dir_path.display()))?;
            continue;
        }

        let dest = spt_root.join(&name);

        // Ensure parent directory exists
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }

        // Extract file
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .with_context(|| format!("failed to read entry: {name}"))?;

        fs::write(&dest, &content)
            .with_context(|| format!("failed to write: {}", dest.display()))?;

        let hash = compute_hash(&content);
        let size = content.len() as u64;

        extracted.push(ExtractedFile {
            path: name,
            hash,
            size,
        });
    }

    Ok(extracted)
}

fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
```

Run: `cargo test spt::mods::tests::extract_server_mod` — Expected: PASS

- [ ] **Step 7: Write test and implement `compute_file_hash` (for existing files on disk)**

```rust
pub fn compute_file_hash(path: &Path) -> Result<String> {
    let data = fs::read(path)
        .with_context(|| format!("failed to read file: {}", path.display()))?;
    Ok(compute_hash(&data))
}
```

Test:
```rust
    #[test]
    fn compute_hash_of_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, b"hello world").unwrap();
        let hash = compute_file_hash(&path).unwrap();
        // SHA256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
```

Run: `cargo test spt::mods::tests::compute_hash_of_file` — Expected: PASS

- [ ] **Step 8: Write test and implement `delete_mod_files`**

```rust
pub fn delete_mod_files(spt_root: &Path, file_paths: &[String]) -> Result<()> {
    for rel_path in file_paths {
        let full_path = spt_root.join(rel_path);
        if full_path.exists() {
            fs::remove_file(&full_path)
                .with_context(|| format!("failed to delete: {}", full_path.display()))?;
        }
    }

    // Clean up empty parent directories (walk up from each deleted file)
    for rel_path in file_paths {
        let full_path = spt_root.join(rel_path);
        let mut dir = full_path.parent();
        while let Some(d) = dir {
            if d == spt_root {
                break;
            }
            if d.exists() && d.read_dir().map(|mut r| r.next().is_none()).unwrap_or(false) {
                let _ = fs::remove_dir(d);
                dir = d.parent();
            } else {
                break;
            }
        }
    }

    Ok(())
}
```

Test:
```rust
    #[test]
    fn delete_mod_files_and_empty_dirs() {
        let spt_root = tempfile::tempdir().unwrap();
        let mod_dir = spt_root.path().join("user/mods/TestMod");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("test.dll"), b"dll").unwrap();
        fs::write(mod_dir.join("config.json"), b"{}").unwrap();

        delete_mod_files(
            spt_root.path(),
            &[
                "user/mods/TestMod/test.dll".into(),
                "user/mods/TestMod/config.json".into(),
            ],
        )
        .unwrap();

        // Files should be gone
        assert!(!mod_dir.join("test.dll").exists());
        assert!(!mod_dir.join("config.json").exists());
        // Empty directory should be cleaned up
        assert!(!mod_dir.exists());
    }
```

Run: `cargo test spt::mods::tests::delete_mod_files_and_empty_dirs` — Expected: PASS

- [ ] **Step 9: Write test and implement `scan_unmanaged_files`**

```rust
pub fn scan_mod_directories(spt_root: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();

    for subdir in &["user/mods", "BepInEx/plugins"] {
        let dir = spt_root.join(subdir);
        if !dir.is_dir() {
            continue;
        }
        scan_dir_recursive(&dir, spt_root, &mut found)?;
    }

    Ok(found)
}

fn scan_dir_recursive(dir: &Path, spt_root: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read dir: {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let rel = path
                .strip_prefix(spt_root)
                .map_err(|e| anyhow::anyhow!(
                    "path {} is not under spt_root {}: {e}",
                    path.display(),
                    spt_root.display()
                ))?
                .to_string_lossy()
                .to_string();
            out.push(rel);
        } else if path.is_dir() {
            scan_dir_recursive(&path, spt_root, out)?;
        }
    }
    Ok(())
}
```

Test:
```rust
    #[test]
    fn scan_finds_all_mod_files() {
        let spt_root = tempfile::tempdir().unwrap();
        let mods_dir = spt_root.path().join("user/mods/SomeMod");
        fs::create_dir_all(&mods_dir).unwrap();
        fs::write(mods_dir.join("mod.dll"), b"dll").unwrap();
        fs::create_dir_all(spt_root.path().join("BepInEx/plugins")).unwrap();
        fs::write(
            spt_root.path().join("BepInEx/plugins/Plugin.dll"),
            b"dll",
        )
        .unwrap();

        let files = scan_mod_directories(spt_root.path()).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.contains("SomeMod/mod.dll")));
        assert!(files.iter().any(|f| f.contains("Plugin.dll")));
    }
```

Run: `cargo test spt::mods::tests::scan_finds_all_mod_files` — Expected: PASS

- [ ] **Step 10: Commit**

```bash
git add src/spt/
git commit -m "feat: archive handling with mod type detection, extraction, hashing, and directory scanning"
```

---

## Phase 2: Core CLI (Tasks 7–12) — Outline

> Full detail will be added before execution. Below are task boundaries, file lists, and key interfaces.

### Task 7: Init Command

**Files:**
- Create: `src/cli/init.rs`

**Goal:** `quma init [path]` — validate SPT dir, create config + DB, scan existing mods in `user/mods/` and `BepInEx/plugins/` (list as "unmanaged"), prompt for admin user creation. This is moved to Phase 2 (from Phase 5) because later commands and Phase 3–4 testing all depend on having a config + DB created.

### Task 8: Install Command

**Files:**
- Create: `src/cli/install.rs`
- Modify: `src/cli/mod.rs` (wire up)
- Modify: `src/main.rs` (dispatch)

**Goal:** Implement `quma install <mod> [version]`. Resolves mod via Forge API, fetches compatible version, resolves dependencies, downloads/extracts, records in DB. Supports `--force` to bypass server-running check.

**Key flow:**
1. Resolve `<mod>` to Forge mod ID (search API, disambiguation if multiple)
2. Pick version (latest compatible or explicit)
3. Check Fika compatibility on selected version, warn if incompatible
4. Resolve dependency tree via Forge API
5. Display plan, prompt for confirmation
6. Check server status → queue or apply (see note below)
7. Download ZIP, detect type (handle `Ambiguous` by prompting user for target dir), extract (with top-level dir stripping)
8. Record `installed_mods`, `installed_files`, `mod_dependencies`

**Phase ordering note:** Server-running detection (Podman inspect / ping fallback) is Phase 3. For Phase 2, install/update/remove should accept `--force` and always apply. The queue integration (steps 6) should be wired in during Phase 3 Task 13.

**Dependencies:** Tasks 2–6 (config, SPT detect, DB, Forge client, archive handling)

### Task 9: Remove Command

**Files:**
- Create: `src/cli/remove.rs`

**Goal:** Implement `quma remove <mod>`. Check reverse dependencies, offer removal options, delete tracked files, clean up DB records.

### Task 10: Update Command

**Files:**
- Create: `src/cli/update.rs`

**Goal:** Implement `quma update [mod]`. Check for updates via Forge API, download/extract new versions, replace old files, update DB.

### Task 11: List & Check Commands

**Files:**
- Create: `src/cli/list.rs`
- Create: `src/cli/check.rs`

**Goal:** `quma list` — table output of installed mods with update indicators and `--json` flag. Also scans `user/mods/` and `BepInEx/plugins/` for unmanaged files not tracked in the DB and displays them as "unmanaged" entries. `quma check` — categorized update report, exit code 0 if all up-to-date, 1 if updates available (use `std::process::exit()`).

### Task 12: Track Command

**Files:**
- Create: `src/cli/track.rs`

**Goal:** `quma track <path> <forge_mod_id>` — associate unmanaged mod directory with a Forge entry. Fetches mod info, scans files, creates DB records.

---

## Phase 3: Server & Queue (Tasks 13–16) — Outline

### Task 13: Podman Integration

**Files:**
- Create: `src/podman.rs`

**Goal:** Wrapper for Podman CLI commands. Container state inspection (`running`/`stopped`), `start`, `stop`, `logs`. Parses `podman inspect` output.

**Key interface:**
```rust
pub struct PodmanClient { container: String }
impl PodmanClient {
    pub fn new(container: &str) -> Self;
    pub fn is_running(&self) -> Result<bool>;
    pub fn start(&self) -> Result<()>;
    pub fn stop(&self) -> Result<()>;
    pub fn logs(&self, follow: bool, tail: usize) -> Result<()>;
    pub fn detect_spt_containers(spt_dir: &Path) -> Result<Vec<String>>;
}
```

### Task 14: Change Queue & Apply Command

**Files:**
- Create: `src/cli/apply.rs`
- Create: `src/queue.rs`

**Goal:** Server-running detection (Podman state → ping fallback). Queue operations when server running. `quma apply` drains the queue. Respects `queue_changes` and `--force` config/flags.

### Task 15: Server Lifecycle Commands

**Files:**
- Create: `src/cli/server.rs`

**Goal:** `quma server start|stop|restart|logs|status`. Integrates with Podman, change queue drain on lifecycle (configurable — `auto_drain_on_lifecycle` applies to start, stop, AND restart per spec), ping wait on start.

### Task 16: Health Checks (Status Command)

**Files:**
- Create: `src/cli/status.rs`
- Create: `src/health.rs`

**Goal:** `quma status` — liveness ping, version verification, mod load verification (compare loaded vs installed), DB/disk sync (file existence + hash checks), SPT version compatibility. CLI output and `--json` flag. Exit codes 0/1/2.

**SPT server communication:** Requires a separate `SptClient` (distinct from `ForgeClient`) configured with `reqwest::Client::builder().danger_accept_invalid_certs(true)` for the self-signed cert, and a default `responsecompressed: 0` header for raw JSON responses. Do NOT reuse the Forge client for this.

---

## Phase 4: Web UI (Tasks 17–20) — Outline

> Add these dependencies to `Cargo.toml` at the start of Phase 4:
> ```toml
> actix-web = "4"
> actix-session = { version = "0.10", features = ["cookie-session"] }
> actix-governor = "0.6"
> askama = "0.13"
> askama_web = { version = "0.4", features = ["actix-web-4"] }
> rust-embed = "8"
> actix-web-rust-embed-responder = "3"
> argon2 = "0.5"
> ```

### Task 17: Actix Server Foundation

**Files:**
- Create: `src/web/mod.rs`, `src/web/routes.rs`, `src/web/state.rs`
- Create: `src/cli/serve.rs`
- Create: `src/assets/style.css`, `src/assets/htmx.min.js`
- Create: `src/templates/base.html`

**Goal:** Working actix-web server with static asset serving via rust-embed, session middleware, base template, and AppState (DB handle, Forge client, config).

**Concurrency note:** `rusqlite::Connection` is `Send` but not `Sync`. For concurrent web requests, wrap `Database` in `Arc<Mutex<Database>>` and use `web::block()` to run synchronous SQLite operations off the async thread. Alternatively, use `r2d2` with `rusqlite` for a connection pool. Decide at implementation time based on expected load (single-server tool, so `Arc<Mutex>` is likely sufficient for v1).

Also create `src/web/handlers/mod.rs` to re-export handler modules.

### Task 18: Auth System

**Files:**
- Create: `src/web/middleware.rs`
- Create: `src/web/handlers/auth.rs`
- Create: `src/templates/login.html`, `src/templates/register.html`
- Create: `src/spt/profiles.rs`

**Goal:** Login/register/logout flows. Invite code validation. SPT profile dropdown on registration. Argon2 password hashing. Session cookies (signed, HttpOnly, SameSite=Strict). Rate limiting on `/login` and `/register` (5 req/min/IP). Auth middleware for role-based access.

### Task 19: Dashboard & Mod Pages

**Files:**
- Create: `src/web/handlers/dashboard.rs`, `src/web/handlers/mods.rs`
- Create: `src/templates/dashboard.html`, `src/templates/mods/list.html`, `src/templates/mods/detail.html`
- Create: `src/templates/mods/partials/dependency_tree.html`, `src/templates/mods/partials/update_badges.html`

**Goal:** Dashboard showing installed mods + server status. Mod list page (admin: install/update/remove actions). Mod detail page. HTMX update badge polling and dependency tree partial.

### Task 20: Queue, Status & Server Control Pages

**Files:**
- Create: `src/web/handlers/queue.rs`, `src/web/handlers/server.rs`
- Create: `src/templates/queue.html`, `src/templates/status.html`

**Goal:** Queue page (view pending ops, admin cancel/apply). Status page (auto-refreshing health checks via HTMX polling). Server control (admin start/stop/restart buttons). Install flow (admin enters Forge ID, sees dep tree, confirms).

---

## Phase 5: Setup & Utilities (Tasks 21–23) — Outline

### Task 21: Setup Command (Fika)

**Files:**
- Create: `src/cli/setup.rs`

**Goal:** `quma setup` — guided interactive flow: detect SPT dir, validate, configure Podman container (auto-detect or prompt), configure networking (edit `http.json`), install Fika (mod 2326), first boot, configure `fika.jsonc`, print network guidance, run `init`. Supports `--non-interactive` and `--skip-fika`.

### Task 22: Serve Command

**Files:**
- Create: `src/cli/serve.rs`

**Goal:** `quma serve` — start the web UI server. Wire the actix-web server (from Phase 4) into the CLI dispatch. If no admin user exists, prompt to create one first.

### Task 23: Generate, Config & Invite Commands

**Files:**
- Create: `src/cli/generate.rs`
- Create: `src/cli/config.rs` (CLI handler, not config.rs in root)
- Create: `src/cli/invite.rs`

**Goal:**
- `quma generate systemd` — emit systemd unit file to stdout or `--install` to write + enable.
- `quma config [set|get]` — read/write config values.
- `quma invite` — generate invite code, store in DB, print code + registration URL.
