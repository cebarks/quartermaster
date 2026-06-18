# Phase 5: Setup & Utilities Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the remaining CLI commands — `quma config`, `quma invite`, `quma generate systemd`, and `quma setup` — completing all `todo!()` stubs in `main.rs`.

**Architecture:** Each command is a standalone CLI module under `src/cli/`. They consume existing infrastructure (config system, DB layer, Podman integration, install pipeline) without introducing new data types or tables. `setup` is the most complex — an orchestration command that chains together detect → validate → configure → install → init into a guided flow.

**Tech Stack:** Rust, clap (derive), existing `Config`, `Database`, `PodmanClient`, `ForgeClient`, `install_single_mod`

## Global Constraints

- Linux only for v1 — Podman container lifecycle, no Windows paths
- SPT 4.0+ only — directory signature uses `SPT/SPT.Server.exe`, `SPT/SPT_Data/configs/`
- Config file at `<spt_root>/quartermaster.toml` with 0600 permissions on Unix
- Invite codes prefixed `quma-` followed by 10 random alphanumeric chars (spec says 6, increased to ~51 bits of entropy since rate limiting on /register is not yet implemented)
- Session secret is 48-char alphanumeric, auto-generated if empty
- All `todo!()` stubs in `main.rs` must be replaced with real dispatch
- One new dependency allowed: `rpassword` for secure password input (no terminal echo)
- Existing test suite must continue to pass (`cargo test`)
- Follow existing module patterns: `pub async fn run(...)` or `pub fn run(...)` entry point, use `CliContext` from `common.rs` where config/DB/forge are needed

---

### Task 21: Config & Invite Commands

**Files:**
- Create: `src/cli/config_cmd.rs`
- Create: `src/cli/invite.rs`
- Modify: `src/cli/mod.rs` (add `pub mod config_cmd;` and `pub mod invite;`)
- Modify: `src/main.rs` (replace `todo!("config")` and `todo!("invite")` stubs)

**Interfaces:**
- Consumes:
  - `Config::load(path: &Path) -> Result<Config>` from `src/config.rs`
  - `Config::save(&self, path: &Path) -> Result<()>` from `src/config.rs`
  - `Config::resolve_path(cli_config: Option<&Path>, spt_dir: Option<&Path>) -> PathBuf` from `src/config.rs`
  - `Database::create_invite(&self, code: &str, created_by: Option<i64>, expires_at: Option<&str>) -> rusqlite::Result<i64>` from `src/db/users.rs`
  - `CliContext` from `src/cli/common.rs`
  - `detect_spt_dir(explicit: Option<&Path>, cwd: Option<&Path>) -> Result<PathBuf>` from `src/spt/detect.rs`
- Produces:
  - `cli::config_cmd::run(action: &Option<ConfigAction>, cli: &Cli) -> Result<()>`
  - `cli::invite::run(expires: Option<&str>, ctx: &CliContext) -> Result<()>`
  - `generate_invite_code() -> String` (public, used by Task 23's `setup` command)

- [ ] **Step 1: Write failing test — generate invite code format**

In `src/cli/invite.rs`:

```rust
use anyhow::{bail, Result};
use rand::distr::Alphanumeric;
use rand::Rng;

use super::common::CliContext;

pub fn generate_invite_code() -> String {
    let suffix: String = rand::rng()
        .sample_iter(Alphanumeric)
        .take(10)
        .map(char::from)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("quma-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_code_format() {
        let code = generate_invite_code();
        assert!(code.starts_with("quma-"), "code should start with 'quma-'");
        assert_eq!(code.len(), 15, "code should be 15 chars: 'quma-' + 10");
        let suffix = &code[5..];
        assert!(
            suffix.chars().all(|c| c.is_ascii_alphanumeric()),
            "suffix should be alphanumeric"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test cli::invite::tests::invite_code_format`
Expected: PASS

- [ ] **Step 3: Write failing test — parse expiry duration**

Add to `src/cli/invite.rs`:

```rust
fn parse_expiry(input: &str) -> Result<String> {
    let input = input.trim();
    let (num_str, unit) = if input.ends_with('d') {
        (&input[..input.len() - 1], "days")
    } else if input.ends_with('h') {
        (&input[..input.len() - 1], "hours")
    } else if input.ends_with('m') {
        (&input[..input.len() - 1], "minutes")
    } else {
        bail!("invalid expiry format: use e.g. '24h', '7d', '30m'");
    };

    let num: i64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number in expiry: '{num_str}'"))?;

    if num <= 0 {
        bail!("expiry must be positive");
    }

    let duration = match unit {
        "days" => chrono::Duration::days(num),
        "hours" => chrono::Duration::hours(num),
        "minutes" => chrono::Duration::minutes(num),
        _ => unreachable!(),
    };

    let expires_at = chrono::Utc::now() + duration;
    Ok(expires_at.to_rfc3339())
}

#[cfg(test)]
mod tests {
    // ... (existing test above)

    #[test]
    fn parse_expiry_hours() {
        let result = parse_expiry("24h").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_hours() >= 23 && diff.num_hours() <= 24);
    }

    #[test]
    fn parse_expiry_days() {
        let result = parse_expiry("7d").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_days() >= 6 && diff.num_days() <= 7);
    }

    #[test]
    fn parse_expiry_minutes() {
        let result = parse_expiry("30m").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_minutes() >= 29 && diff.num_minutes() <= 30);
    }

    #[test]
    fn parse_expiry_invalid() {
        assert!(parse_expiry("abc").is_err());
        assert!(parse_expiry("0h").is_err());
        assert!(parse_expiry("-5d").is_err());
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test cli::invite::tests`
Expected: all PASS

- [ ] **Step 5: Implement `invite::run`**

Add to `src/cli/invite.rs`:

```rust
pub fn run(expires: Option<&str>, ctx: &CliContext) -> Result<()> {
    let code = generate_invite_code();

    let expires_at = match expires {
        Some(exp) => Some(parse_expiry(exp)?),
        None => None,
    };

    ctx.db
        .create_invite(&code, None, expires_at.as_deref())
        .map_err(|e| anyhow::anyhow!("failed to create invite: {e}"))?;

    println!("Invite code: {code}");
    println!(
        "Registration URL: http://{}:{}/register?code={code}",
        ctx.config.web_bind, ctx.config.web_port
    );

    if let Some(ref exp) = expires_at {
        println!("Expires: {exp}");
    } else {
        println!("Expires: never");
    }

    Ok(())
}
```

- [ ] **Step 6: Implement `config_cmd::run`**

Create `src/cli/config_cmd.rs`:

```rust
use anyhow::{bail, Result};

use crate::config::Config;
use crate::spt::detect::detect_spt_dir;

use super::{Cli, ConfigAction};

pub fn run(action: &Option<ConfigAction>, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));

    match action {
        None => show_config(&config_path),
        Some(ConfigAction::Get { key }) => get_config(&config_path, key),
        Some(ConfigAction::Set { key, value }) => set_config(&config_path, key, value),
    }
}

fn show_config(config_path: &std::path::Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let toml_str = toml::to_string_pretty(&config)?;
    println!("{toml_str}");
    Ok(())
}

fn get_config(config_path: &std::path::Path, key: &str) -> Result<()> {
    let config = Config::load(config_path)?;
    let value = match key {
        "spt_dir" => config
            .spt_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        "forge_token" => config.forge_token.unwrap_or_default(),
        "queue_changes" => config.queue_changes.to_string(),
        "auto_drain_on_lifecycle" => config.auto_drain_on_lifecycle.to_string(),
        "session_secret" => config.session_secret,
        "server_container" => config.server_container.unwrap_or_default(),
        "server_host" => config.server_host.unwrap_or_default(),
        "server_port" => config
            .server_port
            .map(|p| p.to_string())
            .unwrap_or_default(),
        "web_bind" => config.web_bind,
        "web_port" => config.web_port.to_string(),
        _ => bail!("unknown config key: '{key}'"),
    };
    println!("{value}");
    Ok(())
}

fn set_config(config_path: &std::path::Path, key: &str, value: &str) -> Result<()> {
    let mut config = Config::load(config_path)?;
    match key {
        "spt_dir" => config.spt_dir = Some(std::path::PathBuf::from(value)),
        "forge_token" => config.forge_token = Some(value.to_string()),
        "queue_changes" => {
            config.queue_changes = value
                .parse()
                .map_err(|_| anyhow::anyhow!("expected 'true' or 'false'"))?
        }
        "auto_drain_on_lifecycle" => {
            config.auto_drain_on_lifecycle = value
                .parse()
                .map_err(|_| anyhow::anyhow!("expected 'true' or 'false'"))?
        }
        "session_secret" => config.session_secret = value.to_string(),
        "server_container" => config.server_container = Some(value.to_string()),
        "server_host" => config.server_host = Some(value.to_string()),
        "server_port" => {
            config.server_port = Some(
                value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("expected a port number"))?,
            )
        }
        "web_bind" => config.web_bind = value.to_string(),
        "web_port" => {
            config.web_port = value
                .parse()
                .map_err(|_| anyhow::anyhow!("expected a port number"))?
        }
        _ => bail!("unknown config key: '{key}'"),
    }
    config.save(config_path)?;
    println!("Set {key} = {value}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        set_config(&config_path, "web_port", "3000").unwrap();
        let reloaded = Config::load(&config_path).unwrap();
        assert_eq!(reloaded.web_port, 3000);
    }

    #[test]
    fn set_boolean_values() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        set_config(&config_path, "queue_changes", "false").unwrap();
        let reloaded = Config::load(&config_path).unwrap();
        assert!(!reloaded.queue_changes);

        set_config(&config_path, "auto_drain_on_lifecycle", "true").unwrap();
        let reloaded = Config::load(&config_path).unwrap();
        assert!(reloaded.auto_drain_on_lifecycle);
    }

    #[test]
    fn set_unknown_key_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        assert!(set_config(&config_path, "nonexistent_key", "value").is_err());
    }

    #[test]
    fn set_invalid_port_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        assert!(set_config(&config_path, "web_port", "not_a_number").is_err());
    }

    #[test]
    fn set_invalid_boolean_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("quartermaster.toml");
        let config = Config::default();
        config.save(&config_path).unwrap();

        assert!(set_config(&config_path, "queue_changes", "maybe").is_err());
    }
}
```

- [ ] **Step 7: Wire up modules in `cli/mod.rs`**

Add these two lines to the module declarations in `src/cli/mod.rs`:

```rust
pub mod config_cmd;
pub mod invite;
```

- [ ] **Step 8: Wire up dispatch in `main.rs`**

Replace the `todo!()` stubs in `src/main.rs`:

```rust
Command::Invite { expires } => {
    let ctx = cli::common::resolve_context(&cli)?;
    cli::invite::run(expires.as_deref(), &ctx)
}
Command::Config { action } => cli::config_cmd::run(action, &cli),
```

- [ ] **Step 9: Run all tests**

Run: `cargo test`
Expected: all PASS (existing tests + new tests)

- [ ] **Step 10: Verify CLI behavior**

```bash
cargo run -- config --help
cargo run -- invite --help
```

Expected: both print help text.

- [ ] **Step 11: Commit**

```bash
git add src/cli/config_cmd.rs src/cli/invite.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add config and invite CLI commands"
```

---

### Task 22: Generate Systemd Command

**Files:**
- Create: `src/cli/generate.rs`
- Modify: `src/cli/mod.rs` (add `pub mod generate;`)
- Modify: `src/main.rs` (replace `todo!("generate")` stub)

**Interfaces:**
- Consumes:
  - `Cli` struct from `src/cli/mod.rs`
  - `GenerateTarget::Systemd { install: bool }` from `src/cli/mod.rs`
  - `Config::load(path: &Path) -> Result<Config>` from `src/config.rs`
  - `Config::resolve_path(...)` from `src/config.rs`
  - `detect_spt_dir(...)` from `src/spt/detect.rs`
- Produces:
  - `cli::generate::run(target: &GenerateTarget, cli: &Cli) -> Result<()>`

- [ ] **Step 1: Write failing test — systemd unit file generation**

Create `src/cli/generate.rs`:

```rust
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::spt::detect::detect_spt_dir;

use super::{Cli, GenerateTarget};

pub fn run(target: &GenerateTarget, cli: &Cli) -> Result<()> {
    match target {
        GenerateTarget::Systemd { install } => generate_systemd(*install, cli),
    }
}

fn generate_systemd_unit(
    spt_dir: &Path,
    config_path: &Path,
    config: &Config,
) -> String {
    let quma_path = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("/usr/local/bin/quma"));

    format!(
        r#"[Unit]
Description=Quartermaster (quma) Web UI
After=network.target

[Service]
Type=simple
WorkingDirectory={working_dir}
ExecStart={exec} serve --bind {bind} --port {port} --spt-dir {spt_dir} --config {config}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
        working_dir = spt_dir.display(),
        exec = quma_path.display(),
        bind = config.web_bind,
        port = config.web_port,
        spt_dir = spt_dir.display(),
        config = config_path.display(),
    )
}

fn generate_systemd(install: bool, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let config = Config::load(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    let unit = generate_systemd_unit(&spt_dir, &config_path, &config);

    if install {
        let service_path = Path::new("/etc/systemd/system/quartermaster.service");
        std::fs::write(service_path, &unit)
            .with_context(|| format!("failed to write {} — are you running as root?", service_path.display()))?;
        println!("Wrote {}", service_path.display());

        let status = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()
            .context("failed to run systemctl daemon-reload")?;
        if !status.success() {
            bail!("systemctl daemon-reload failed");
        }

        let status = std::process::Command::new("systemctl")
            .args(["enable", "quartermaster.service"])
            .status()
            .context("failed to enable quartermaster.service")?;
        if !status.success() {
            bail!("systemctl enable failed");
        }

        println!("Service enabled. Start with: systemctl start quartermaster");
    } else {
        print!("{unit}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_contains_required_fields() {
        let config = Config {
            web_bind: "0.0.0.0".to_string(),
            web_port: 9190,
            ..Config::default()
        };

        let unit = generate_systemd_unit(
            Path::new("/opt/spt"),
            Path::new("/opt/spt/quartermaster.toml"),
            &config,
        );

        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("WorkingDirectory=/opt/spt"));
        assert!(unit.contains("--bind 0.0.0.0"));
        assert!(unit.contains("--port 9190"));
        assert!(unit.contains("--spt-dir /opt/spt"));
        assert!(unit.contains("--config /opt/spt/quartermaster.toml"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("After=network.target"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn systemd_unit_uses_custom_bind_port() {
        let config = Config {
            web_bind: "127.0.0.1".to_string(),
            web_port: 8080,
            ..Config::default()
        };

        let unit = generate_systemd_unit(
            Path::new("/srv/spt"),
            Path::new("/srv/spt/quartermaster.toml"),
            &config,
        );

        assert!(unit.contains("--bind 127.0.0.1"));
        assert!(unit.contains("--port 8080"));
        assert!(unit.contains("WorkingDirectory=/srv/spt"));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test cli::generate::tests`
Expected: all PASS

- [ ] **Step 3: Wire up module in `cli/mod.rs`**

Add to `src/cli/mod.rs`:

```rust
pub mod generate;
```

- [ ] **Step 4: Wire up dispatch in `main.rs`**

Replace the `todo!("generate")` stub:

```rust
Command::Generate { target } => cli::generate::run(target, &cli),
```

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 6: Verify CLI behavior**

```bash
cargo run -- generate systemd --help
```

Expected: prints help text showing `--install` flag.

- [ ] **Step 7: Commit**

```bash
git add src/cli/generate.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add generate systemd command"
```

---

### Task 23: Setup Command

**Files:**
- Create: `src/cli/setup.rs`
- Modify: `src/cli/mod.rs` (add `pub mod setup;`)
- Modify: `src/main.rs` (replace `todo!("setup")` stub)

**Interfaces:**
- Consumes:
  - `detect_spt_dir(explicit: Option<&Path>, cwd: Option<&Path>) -> Result<PathBuf>` from `src/spt/detect.rs`
  - `validate_spt_dir(path: &Path) -> Result<()>` from `src/spt/detect.rs`
  - `read_spt_version(spt_dir: &Path) -> Result<SptInfo>` from `src/spt/detect.rs`
  - `Config::default()`, `Config::save(...)`, `Config::load(...)`, `Config::resolve_path(...)`, `Config::ensure_session_secret()` from `src/config.rs`
  - `Database::open(path: &Path) -> Result<Database>` from `src/db/mod.rs`
  - `Database::admin_exists() -> rusqlite::Result<bool>` from `src/db/users.rs`
  - `Database::insert_user(username, spt_profile_id, password_hash, role) -> rusqlite::Result<i64>` from `src/db/users.rs`
  - `PodmanClient::detect_spt_containers(spt_dir: &Path) -> Result<Vec<String>>` from `src/podman.rs`
  - `PodmanClient::new(container: &str) -> PodmanClient` from `src/podman.rs`
  - `PodmanClient::start() -> Result<()>` from `src/podman.rs`
  - `ForgeClient::new(token: Option<String>) -> Result<ForgeClient>` from `src/forge/client.rs`
  - `install_single_mod(ctx, forge_mod_id, forge_version_id, download_url, name, slug, version) -> Result<i64>` from `src/cli/install.rs`
  - `list_profiles(spt_dir: &Path) -> Result<Vec<SptProfile>>` from `src/spt/profiles.rs`
  - `hash_password(password: &str) -> Result<String>` from `src/web/auth.rs`
  - `generate_invite_code() -> String` from `src/cli/invite.rs` (Task 21)
  - `confirm(prompt: &str) -> Result<bool>` from `src/cli/common.rs`
  - `find_unmanaged_mod_dirs(spt_dir, db) -> Result<(BTreeMap<String, usize>, usize)>` from `src/cli/common.rs`
  - `server_detect::resolve_server_addr(config, spt_dir) -> (String, u16)` from `src/server_detect.rs`
  - `spt::server::SptClient::new(host, port) -> Result<SptClient>` from `src/spt/server.rs`
- Produces:
  - `cli::setup::run(non_interactive: bool, skip_fika: bool, cli: &Cli) -> Result<()>`

The setup command is the most complex command in the system — it orchestrates a multi-step guided flow. Steps are sequential and depend on each other, but each step should be a clear function.

- [ ] **Step 1: Add `rpassword` dependency to `Cargo.toml`**

Add to `[dependencies]` in `Cargo.toml`:

```toml
# Secure password input (no terminal echo)
rpassword = "5"
```

- [ ] **Step 2: Create `src/cli/setup.rs` with module structure and helpers**

```rust
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::podman::PodmanClient;
use crate::spt::detect::{detect_spt_dir, read_spt_version, validate_spt_dir};
use crate::spt::profiles::list_profiles;
use crate::web::auth::hash_password;

use super::common::{confirm, find_unmanaged_mod_dirs};
use super::Cli;

const FIKA_FORGE_MOD_ID: i64 = 2326;

pub async fn run(non_interactive: bool, skip_fika: bool, cli: &Cli) -> Result<()> {
    println!("=== Quartermaster Setup ===\n");

    // Step 1: Detect/confirm SPT directory
    let spt_dir = detect_spt_directory(cli, non_interactive)?;

    // Step 2: Validate SPT install
    let spt_info = read_spt_version(&spt_dir)?;
    println!(
        "SPT {} (EFT {}) detected at {}",
        spt_info.spt_version,
        spt_info.tarkov_version,
        spt_dir.display()
    );

    // Step 3: Create config
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.spt_dir = Some(spt_dir.clone());
    config.ensure_session_secret();

    // Step 4: Configure Podman container
    configure_container(&spt_dir, &mut config, non_interactive).await?;

    // Step 5: Configure networking
    configure_networking(&spt_dir, &mut config, non_interactive)?;

    // Save config so far (container + networking)
    config.save(&config_path)?;
    println!("\nConfig saved to {}", config_path.display());

    // Step 6: Create database and build CliContext for reuse
    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    let forge = ForgeClient::new(config.forge_token.clone())?;
    let ctx = super::common::CliContext {
        spt_dir: spt_dir.clone(),
        spt_info: spt_info.clone(),
        config: config.clone(),
        config_path: config_path.clone(),
        db,
        forge,
    };

    // Step 7: Install Fika (if not skipped)
    if !skip_fika {
        install_fika(&ctx).await?;
    } else {
        println!("\nSkipping Fika installation (--skip-fika).");
    }

    // Step 8: First boot (if container configured)
    if config.server_container.is_some() && !skip_fika {
        first_boot(&config, &spt_dir, non_interactive).await?;
    }

    // Step 9: Configure fika.jsonc (if Fika installed and first boot ran)
    if config.server_container.is_some() && !skip_fika {
        configure_fika(&spt_dir, non_interactive)?;
    }

    // Step 10: Scan for unmanaged mods (same as quma init)
    scan_unmanaged(&spt_dir, &ctx.db)?;

    // Step 11: Create admin user
    create_admin_user(&spt_dir, &ctx.db, non_interactive)?;

    // Step 12: Print summary
    print_summary(&config, &spt_dir, skip_fika);

    Ok(())
}

fn detect_spt_directory(cli: &Cli, non_interactive: bool) -> Result<PathBuf> {
    match detect_spt_dir(cli.spt_dir.as_deref(), None) {
        Ok(dir) => {
            println!("Found SPT directory: {}", dir.display());
            if !non_interactive && !confirm("Use this directory?")? {
                bail!("Setup cancelled. Use --spt-dir to specify the SPT directory.");
            }
            Ok(dir)
        }
        Err(_) => {
            if non_interactive {
                bail!("Could not auto-detect SPT directory. Use --spt-dir to specify it.");
            }
            print!("Enter SPT server directory path: ");
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let path = PathBuf::from(input.trim());
            validate_spt_dir(&path)?;
            Ok(path)
        }
    }
}

async fn configure_container(
    spt_dir: &Path,
    config: &mut Config,
    non_interactive: bool,
) -> Result<()> {
    println!("\n--- Container Configuration ---");

    if config.server_container.is_some() {
        println!(
            "Container already configured: {}",
            config.server_container.as_deref().unwrap()
        );
        return Ok(());
    }

    let detected = PodmanClient::detect_spt_containers(spt_dir).await?;

    if detected.len() == 1 {
        let name = &detected[0];
        println!("Detected Podman container: {name}");
        if non_interactive || confirm("Use this container?")? {
            config.server_container = Some(name.clone());
            return Ok(());
        }
    } else if detected.len() > 1 {
        println!("Multiple containers detected:");
        for (i, name) in detected.iter().enumerate() {
            println!("  [{}] {}", i + 1, name);
        }
        if !non_interactive {
            print!("Select [1-{}] or press Enter to skip: ", detected.len());
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if !input.is_empty() {
                if let Ok(choice) = input.parse::<usize>() {
                    if choice >= 1 && choice <= detected.len() {
                        config.server_container = Some(detected[choice - 1].clone());
                        return Ok(());
                    }
                }
            }
        }
    } else {
        println!("No Podman containers detected.");
    }

    if !non_interactive {
        print!("Enter container name (or press Enter to skip): ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let name = input.trim();
        if !name.is_empty() {
            config.server_container = Some(name.to_string());
        } else {
            println!("Skipping container setup. Set it later with: quma config set server_container <name>");
        }
    }

    Ok(())
}

/// Read and optionally update SPT's http.json networking config.
/// Returns (ip, port) of the final server binding.
fn configure_networking(
    spt_dir: &Path,
    config: &mut Config,
    non_interactive: bool,
) -> Result<()> {
    println!("\n--- Network Configuration ---");

    let http_json_path = spt_dir.join("SPT/SPT_Data/configs/http.json");

    if http_json_path.exists() {
        let contents = std::fs::read_to_string(&http_json_path)?;
        let mut json: serde_json::Value = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", http_json_path.display()))?;

        let current_ip = json.get("ip").and_then(|v| v.as_str()).unwrap_or("127.0.0.1");
        let current_port = json.get("port").and_then(|v| v.as_u64()).unwrap_or(6969) as u16;

        println!("Current SPT server bind: {current_ip}:{current_port}");

        if current_ip == "127.0.0.1" {
            println!("SPT is bound to localhost — remote players won't be able to connect.");
            let should_update = non_interactive || confirm("Set bind to 0.0.0.0 (all interfaces)?")?;
            if should_update {
                json["ip"] = serde_json::Value::String("0.0.0.0".to_string());
                let updated = serde_json::to_string_pretty(&json)?;
                std::fs::write(&http_json_path, updated)?;
                println!("Updated http.json: ip = 0.0.0.0");
                println!(
                    "WARNING: SPT server will now listen on all network interfaces.\n\
                     Ensure your firewall allows TCP port {} only from trusted networks.\n\
                     Without firewall rules, the server is accessible from the public internet.",
                    current_port
                );
            }
        }

        config.server_host = Some(
            json.get("ip")
                .and_then(|v| v.as_str())
                .unwrap_or("0.0.0.0")
                .to_string(),
        );
        config.server_port = Some(current_port);
    } else {
        println!("http.json not found — using defaults (127.0.0.1:6969)");
        config.server_host = Some("127.0.0.1".to_string());
        config.server_port = Some(6969);
    }

    Ok(())
}

async fn install_fika(ctx: &super::common::CliContext) -> Result<()> {
    println!("\n--- Fika Installation ---");

    if ctx.db.get_mod_by_forge_id(FIKA_FORGE_MOD_ID)?.is_some() {
        println!("Fika is already installed.");
        return Ok(());
    }

    println!("Looking up Fika on Forge...");
    let versions = ctx
        .forge
        .get_versions(FIKA_FORGE_MOD_ID, Some(&ctx.spt_info.spt_version))
        .await?;

    let version = match versions.into_iter().next() {
        Some(v) => v,
        None => {
            println!(
                "Warning: no Fika version compatible with SPT {} found. Skipping Fika install.",
                ctx.spt_info.spt_version
            );
            return Ok(());
        }
    };

    println!("Installing Fika v{}...", version.version);
    crate::cli::install::install_with_deps(ctx, FIKA_FORGE_MOD_ID, version.id).await?;

    println!("Fika installed successfully.");
    Ok(())
}

async fn first_boot(config: &Config, spt_dir: &Path, non_interactive: bool) -> Result<()> {
    println!("\n--- First Boot ---");

    let container = match config.server_container.as_deref() {
        Some(c) => c,
        None => bail!("no server container configured"),
    };
    let podman = PodmanClient::new(container);

    let running = podman.is_running().await.unwrap_or(false);
    if running {
        println!("Server is already running.");
        return Ok(());
    }

    if !non_interactive && !confirm("Start SPT server for first boot (generates fika.jsonc)?")? {
        println!("Skipping first boot. Start the server manually to generate config files.");
        return Ok(());
    }

    println!("Starting SPT server...");
    podman.start().await?;

    let (host, port) = crate::server_detect::resolve_server_addr(config, spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port)?;

    println!("Waiting for server to start (timeout: 90s)...");
    let start_time = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(90);

    loop {
        if start_time.elapsed() > timeout {
            println!("Server did not respond within 90s. Check `quma server logs` for errors.");
            println!("You may need to start and configure it manually.");
            return Ok(());
        }

        let ping = spt_client.ping().await?;
        if ping.ok {
            println!("Server is ready (responded in {}ms).", ping.latency_ms);
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    // Stop the server after first boot
    println!("Stopping server after first boot...");
    podman.stop().await?;
    println!("Server stopped.");

    Ok(())
}

/// Configure key fika.jsonc settings after first boot generates the file.
fn configure_fika(spt_dir: &Path, non_interactive: bool) -> Result<()> {
    println!("\n--- Fika Configuration ---");

    let fika_config_path = spt_dir.join("SPT/user/mods/fika-server/assets/configs/fika.jsonc");
    if !fika_config_path.exists() {
        println!("fika.jsonc not found — Fika may not have generated its config yet.");
        println!("Start the server manually, then edit fika.jsonc.");
        return Ok(());
    }

    // Read fika.jsonc — strip // comments to parse as JSON
    let raw = std::fs::read_to_string(&fika_config_path)
        .with_context(|| format!("failed to read {}", fika_config_path.display()))?;
    let stripped: String = raw
        .lines()
        .map(|line| {
            // Naive comment stripping: remove everything after // that's not inside a string
            // This works for fika.jsonc's simple structure (no URLs in values)
            if let Some(pos) = line.find("//") {
                &line[..pos]
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut json: serde_json::Value = serde_json::from_str(&stripped)
        .with_context(|| "failed to parse fika.jsonc")?;

    if non_interactive {
        println!("Using Fika defaults (non-interactive mode).");
        return Ok(());
    }

    println!("Configure Fika settings (press Enter to keep default):\n");

    // friendlyFire (default: true)
    let ff_current = json.get("friendlyFire").and_then(|v| v.as_bool()).unwrap_or(true);
    print!("  Friendly fire [{}]: ", if ff_current { "Y/n" } else { "y/N" });
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if !input.is_empty() {
        json["friendlyFire"] = serde_json::Value::Bool(
            input.eq_ignore_ascii_case("y") || input.eq_ignore_ascii_case("yes")
        );
    }

    // forceSaveOnDeath (default: true)
    let fsd_current = json.get("forceSaveOnDeath").and_then(|v| v.as_bool()).unwrap_or(true);
    print!("  Force save on death [{}]: ", if fsd_current { "Y/n" } else { "y/N" });
    std::io::stdout().flush()?;
    input.clear();
    std::io::stdin().read_line(&mut input)?;
    let input_trimmed = input.trim();
    if !input_trimmed.is_empty() {
        json["forceSaveOnDeath"] = serde_json::Value::Bool(
            input_trimmed.eq_ignore_ascii_case("y") || input_trimmed.eq_ignore_ascii_case("yes")
        );
    }

    // sharedQuestProgression (default: false)
    let sqp_current = json.get("sharedQuestProgression").and_then(|v| v.as_bool()).unwrap_or(false);
    print!("  Shared quest progression [{}]: ", if sqp_current { "Y/n" } else { "y/N" });
    std::io::stdout().flush()?;
    input.clear();
    std::io::stdin().read_line(&mut input)?;
    let input_trimmed = input.trim();
    if !input_trimmed.is_empty() {
        json["sharedQuestProgression"] = serde_json::Value::Bool(
            input_trimmed.eq_ignore_ascii_case("y") || input_trimmed.eq_ignore_ascii_case("yes")
        );
    }

    let updated = serde_json::to_string_pretty(&json)?;
    std::fs::write(&fika_config_path, updated)?;
    println!("Fika config updated.");

    Ok(())
}

/// Scan for unmanaged mods (same as quma init step 4).
fn scan_unmanaged(spt_dir: &Path, db: &Database) -> Result<()> {
    let (unmanaged_dirs, unmanaged_count) = find_unmanaged_mod_dirs(spt_dir, db)?;

    if unmanaged_dirs.is_empty() {
        println!("\nNo unmanaged mod files found.");
    } else {
        println!(
            "\nFound {} unmanaged mod director{} ({} files):",
            unmanaged_dirs.len(),
            if unmanaged_dirs.len() == 1 { "y" } else { "ies" },
            unmanaged_count
        );
        for dir in unmanaged_dirs.keys() {
            println!("  {}", dir);
        }
        println!("\nUse `quma track <path> <forge_mod_id>` to associate them with Forge entries.");
    }

    Ok(())
}

fn create_admin_user(spt_dir: &Path, db: &Database, non_interactive: bool) -> Result<()> {
    println!("\n--- Admin User ---");

    if db.admin_exists()? {
        println!("Admin user already exists.");
        return Ok(());
    }

    if non_interactive {
        println!("No admin user created (non-interactive mode).");
        println!("Create one later with the web UI (`quma serve`) or `quma invite`.");
        return Ok(());
    }

    let profiles = list_profiles(spt_dir)?;
    if profiles.is_empty() {
        println!("No SPT profiles found. Start the server and create a profile first.");
        println!("Then run `quma setup` again to create an admin user.");
        return Ok(());
    }

    println!("Select an SPT profile for the admin user:");
    for (i, p) in profiles.iter().enumerate() {
        println!("  [{}] {} (AID: {})", i + 1, p.username, p.aid);
    }

    print!("Select [1-{}]: ", profiles.len());
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid selection"))?;

    if choice == 0 || choice > profiles.len() {
        bail!("selection out of range");
    }

    let profile = &profiles[choice - 1];

    // Prompt for password without echoing to terminal
    let password = rpassword::prompt_password("Password (min 8 chars): ")
        .context("failed to read password")?;

    if password.len() < 8 {
        bail!("password must be at least 8 characters");
    }

    let password_hash = hash_password(&password)?;

    db.insert_user(&profile.username, &profile.aid, Some(&password_hash), "admin")
        .map_err(|e| anyhow::anyhow!("failed to create admin user: {e}"))?;

    println!("Admin user '{}' created.", profile.username);
    Ok(())
}

fn print_summary(config: &Config, spt_dir: &Path, skip_fika: bool) {
    println!("\n=== Setup Complete ===\n");
    println!("SPT directory: {}", spt_dir.display());
    if let Some(ref container) = config.server_container {
        println!("Container: {container}");
    }
    if !skip_fika {
        println!("Fika: installed");
    }
    println!("Web UI: http://{}:{}", config.web_bind, config.web_port);
    println!("\nNext steps:");
    println!("  quma serve              Start the web UI");
    println!("  quma server start       Start the SPT server");
    println!("  quma invite             Generate invite codes for players");
    println!("\nNetwork requirements (for multiplayer):");
    println!("  TCP 6969 inbound        SPT server");
    println!("  UDP 25565 inbound       Fika P2P raids (whoever hosts)");
    println!("  Consider UPnP or VPN as alternatives to port forwarding.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_http_json(spt_dir: &Path, ip: &str, port: u16) -> PathBuf {
        let configs_dir = spt_dir.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        let path = configs_dir.join("http.json");
        std::fs::write(
            &path,
            format!(r#"{{"ip": "{ip}", "port": {port}}}"#),
        )
        .unwrap();
        path
    }

    #[test]
    fn configure_networking_updates_localhost_to_all_interfaces() {
        let tmp = tempfile::tempdir().unwrap();
        create_http_json(tmp.path(), "127.0.0.1", 6969);

        let mut config = Config::default();
        configure_networking(tmp.path(), &mut config, true).unwrap();

        assert_eq!(config.server_host, Some("0.0.0.0".to_string()));
        assert_eq!(config.server_port, Some(6969));

        // Verify the file was actually updated
        let updated = std::fs::read_to_string(
            tmp.path().join("SPT/SPT_Data/configs/http.json"),
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(json["ip"].as_str().unwrap(), "0.0.0.0");
    }

    #[test]
    fn configure_networking_preserves_non_localhost() {
        let tmp = tempfile::tempdir().unwrap();
        create_http_json(tmp.path(), "0.0.0.0", 7000);

        let mut config = Config::default();
        configure_networking(tmp.path(), &mut config, true).unwrap();

        assert_eq!(config.server_host, Some("0.0.0.0".to_string()));
        assert_eq!(config.server_port, Some(7000));
    }

    #[test]
    fn configure_networking_handles_missing_file() {
        let tmp = tempfile::tempdir().unwrap();

        let mut config = Config::default();
        configure_networking(tmp.path(), &mut config, true).unwrap();

        assert_eq!(config.server_host, Some("127.0.0.1".to_string()));
        assert_eq!(config.server_port, Some(6969));
    }

    #[test]
    fn scan_unmanaged_with_empty_db() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        // Create mod directories
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/SomeMod")).unwrap();
        std::fs::write(spt_dir.join("SPT/user/mods/SomeMod/mod.dll"), b"test").unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();
        // Should not panic, should report unmanaged dirs
        scan_unmanaged(spt_dir, &db).unwrap();
    }

    #[test]
    fn create_admin_user_skips_in_non_interactive() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();

        // Non-interactive should skip without error
        create_admin_user(tmp.path(), &db, true).unwrap();
        assert!(!db.admin_exists().unwrap());
    }

    #[test]
    fn create_admin_user_skips_when_admin_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        db.insert_user("admin", "aid123", Some("hash"), "admin").unwrap();

        // Should skip with message, not prompt
        create_admin_user(tmp.path(), &db, false).unwrap();
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test cli::setup::tests`
Expected: all PASS

- [ ] **Step 4: Wire up module in `cli/mod.rs`**

Add to `src/cli/mod.rs`:

```rust
pub mod setup;
```

- [ ] **Step 5: Wire up dispatch in `main.rs`**

Replace the `todo!("setup")` stub:

```rust
Command::Setup {
    non_interactive,
    skip_fika,
} => cli::setup::run(*non_interactive, *skip_fika, &cli).await,
```

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: all PASS. No `todo!()` stubs should remain in `main.rs`.

- [ ] **Step 7: Verify no `todo!()` remains**

```bash
grep -n 'todo!(' src/main.rs
```

Expected: no matches.

- [ ] **Step 8: Verify CLI behavior**

```bash
cargo run -- setup --help
```

Expected: prints help text showing `--non-interactive` and `--skip-fika` flags.

- [ ] **Step 9: Run clippy**

```bash
cargo clippy -- -D warnings
```

Expected: no warnings or errors.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml src/cli/setup.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add setup command with guided Fika setup flow"
```

---

## Self-Review

**Spec coverage:**
- `quma config` (show/get/set) — Task 21 ✅
- `quma invite` (generate code, expiry, store in DB, print registration URL) — Task 21 ✅
- `quma generate systemd` (emit unit file, `--install` flag with daemon-reload + enable) — Task 22 ✅
- `quma setup` — all 9 spec steps covered in Task 23:
  1. Detect/confirm SPT directory ✅
  2. Validate SPT install ✅
  3. Configure Podman container ✅
  4. Configure networking (with firewall warning) ✅
  5. Install Fika ✅
  6. First boot ✅
  7. Configure fika.jsonc settings ✅
  8. Network guidance (in print_summary) ✅
  9. Run init equivalent (unmanaged mod scan + admin user) ✅
- `quma serve` — already implemented, not in scope ✅
- All `todo!()` stubs removed from `main.rs` — Tasks 21-23 ✅

**Placeholder scan:** No TBD/TODO/implement-later placeholders found.

**Type consistency:** All function signatures reference existing types (`Config`, `Database`, `CliContext`, `ForgeClient`, `PodmanClient`, `SptInfo`, `ConfigAction`, `GenerateTarget`). Method names match existing code (`Config::load`, `Config::save`, `db.create_invite`, `db.admin_exists`, `PodmanClient::detect_spt_containers`, `install_with_deps`, `find_unmanaged_mod_dirs`).

**Notable design decisions:**
- Named the config CLI module `config_cmd.rs` (not `config.rs`) to avoid name collision with the existing `src/config.rs` module.
- `setup` reuses `install_with_deps` from the install pipeline rather than reimplementing mod installation.
- `setup` builds one `CliContext` early and reuses it for Fika install, avoiding the double-Database-open pattern.
- `first_boot` uses a soft 90s timeout and doesn't fail hard — first boot can be slow and the user can retry manually.
- Admin user creation is interactive-only; `--non-interactive` skips it with a message about creating one later.
- Invite codes use 10-char suffix (~51 bits entropy) instead of spec's 6-char, since rate limiting on `/register` is deferred.
- Password input uses `rpassword` crate to avoid echoing to terminal (one new dependency).
- Firewall warning printed immediately after changing `http.json` to `0.0.0.0`, per spec requirement.
- Unmanaged mod scan reuses `find_unmanaged_mod_dirs` from `common.rs`, matching `init` behavior.
