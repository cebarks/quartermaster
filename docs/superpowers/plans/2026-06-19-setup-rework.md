# Setup Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `quma setup` and `quma init` with a single command that can bootstrap a brand new SPT/Fika server from nothing or wrap an existing SPT installation, asking at most three questions.

**Architecture:** The new `quma setup` detects whether the target directory is empty (bootstrap from scratch using the `ghcr.io/zhliau/fika-spt-server-docker` container image) or contains a valid SPT install (wrap it). Admin users are web-only accounts with nullable `spt_profile_id`. The container image handles Fika installation via `FIKA_MODE` env var.

**Tech Stack:** Rust, clap (CLI), bollard (container API), rusqlite (SQLite), rpassword (password input)

## Global Constraints

- Container mount path is `/opt/server` (not `/opt/tarkov` — the existing `server create` command has this wrong; fix it as part of this work)
- Container image: `ghcr.io/zhliau/fika-spt-server-docker:latest` (already defined as `SPT_SERVER_IMAGE` in `src/container.rs:18`)
- Default container name: `spt-server` (already defined as `DEFAULT_CONTAINER_NAME` in `src/container.rs:20`)
- Default data dir: `~/spt-server`
- Admin username: `admin`
- Run `just lint` and `just test` to verify after each task

---

### Task 1: Migration — Make `spt_profile_id` Nullable

**Files:**
- Create: `migrations/008_nullable_profile_id.sql`
- Modify: `src/db/users.rs:64-74` (User struct), `src/db/users.rs:118-130` (insert_user), `src/db/users.rs:379-398` (row_to_user)
- Modify: `src/db/tests.rs:212-233` (insert_and_get_user test)
- Modify: `src/web/handlers/admin.rs:22-48` (build_user_profiles), `src/web/handlers/admin.rs:504-519` (render_user_detail)
- Modify: `src/web/handlers/auth.rs:346` (register handler)
- Modify: `src/cli/setup.rs:500-508` (create_admin_user — will be rewritten in Task 4, but callers in tests reference this)

**Interfaces:**
- Produces: `User.spt_profile_id: Option<String>`, `insert_user(username, spt_profile_id: Option<&str>, password_hash, role)`

- [ ] **Step 1: Write the migration SQL**

Create `migrations/008_nullable_profile_id.sql`:

```sql
BEGIN;

CREATE TABLE users_new (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL UNIQUE,
    spt_profile_id  TEXT,
    password_hash   TEXT,
    role            TEXT NOT NULL DEFAULT 'player',
    disabled        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    password_changed_at TEXT
);

INSERT INTO users_new (id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at)
    SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
    FROM users;

DROP TABLE users;
ALTER TABLE users_new RENAME TO users;

COMMIT;
```

- [ ] **Step 2: Update `User` struct and `row_to_user`**

In `src/db/users.rs`, change the `User` struct field:

```rust
pub struct User {
    pub id: i64,
    pub username: String,
    pub spt_profile_id: Option<String>,  // was: String
    pub password_hash: Option<String>,
    pub role: Role,
    pub disabled: bool,
    pub created_at: String,
    pub password_changed_at: Option<String>,
}
```

`row_to_user` already uses `row.get(2)?` which will correctly produce `Option<String>` for a nullable column — no change needed there since `rusqlite::Row::get` returns `Option<String>` when the target type is `Option<String>`.

- [ ] **Step 3: Update `insert_user` signature**

In `src/db/users.rs`, change `insert_user`:

```rust
pub fn insert_user(
    &self,
    username: &str,
    spt_profile_id: Option<&str>,
    password_hash: Option<&str>,
    role: Role,
) -> rusqlite::Result<i64> {
    self.conn.execute(
        "INSERT INTO users (username, spt_profile_id, password_hash, role) VALUES (?1, ?2, ?3, ?4)",
        params![username, spt_profile_id, password_hash, role.as_str()],
    )?;
    Ok(self.conn.last_insert_rowid())
}
```

- [ ] **Step 4: Update all `insert_user` callers**

In `src/web/handlers/auth.rs:346`, change:
```rust
// Before:
let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), Role::Player)?;
// After:
let user_id = db.insert_user(&username, Some(&profile_id), Some(&password_hash), Role::Player)?;
```

In `src/cli/setup.rs:502-508`, change:
```rust
// Before:
db.insert_user(
    &profile.username,
    &profile.aid,
    Some(&password_hash),
    Role::Admin,
)
// After:
db.insert_user(
    &profile.username,
    Some(&profile.aid),
    Some(&password_hash),
    Role::Admin,
)
```

- [ ] **Step 5: Update `build_user_profiles` in admin handler**

In `src/web/handlers/admin.rs:22-48`, change to handle `Option<String>`:

```rust
fn build_user_profiles(
    users: &[User],
    spt_dir: &std::path::Path,
    profile_stats: &std::collections::HashMap<String, SptProfileStats>,
) -> Vec<ProfileStatus> {
    users
        .iter()
        .map(|u| {
            let Some(ref profile_id) = u.spt_profile_id else {
                return ProfileStatus::NotFound;
            };
            if profile_id.is_empty() {
                return ProfileStatus::NotFound;
            }
            match profile_stats.get(profile_id) {
                Some(stats) => ProfileStatus::Found(stats.clone()),
                None => {
                    let profile_path = spt_dir
                        .join("SPT/user/profiles")
                        .join(format!("{}.json", profile_id));
                    if profile_path.exists() {
                        ProfileStatus::ParseError
                    } else {
                        ProfileStatus::NotFound
                    }
                }
            }
        })
        .collect()
}
```

- [ ] **Step 6: Update `render_user_detail` in admin handler**

In `src/web/handlers/admin.rs` around line 505-519, change:

```rust
// Before:
let aid = user.spt_profile_id.clone();
// ...
let profile = if aid.is_empty() {
// After:
let aid = user.spt_profile_id.clone().unwrap_or_default();
// ...
let profile = if aid.is_empty() {
```

- [ ] **Step 7: Update tests**

In `src/db/tests.rs`, update the `insert_and_get_user` test:

```rust
#[test]
fn insert_and_get_user() {
    let db = test_db();
    let id = db
        .insert_user("alice", Some("profile-abc"), Some("hashed_pw"), Role::Admin)
        .unwrap();
    assert!(id > 0);

    let user = db
        .get_user_by_username("alice")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.username, "alice");
    assert_eq!(user.spt_profile_id.as_deref(), Some("profile-abc"));
    assert_eq!(user.password_hash.as_deref(), Some("hashed_pw"));
    assert_eq!(user.role, Role::Admin);

    let missing = db.get_user_by_username("bob").unwrap();
    assert!(missing.is_none());

    let users = db.list_users().unwrap();
    assert_eq!(users.len(), 1);
}
```

Update `insert_user_without_password` test (around line 236):

```rust
#[test]
fn insert_user_without_password() {
    let db = test_db();
    let id = db
        .insert_user("trusty", Some("profile-xyz"), None, Role::Player)
```

Add a new test for null profile:

```rust
#[test]
fn insert_user_without_profile() {
    let db = test_db();
    let id = db
        .insert_user("admin", None, Some("hashed_pw"), Role::Admin)
        .unwrap();
    assert!(id > 0);

    let user = db
        .get_user_by_username("admin")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.username, "admin");
    assert!(user.spt_profile_id.is_none());
    assert_eq!(user.role, Role::Admin);
}
```

Also search for and update any other `insert_user` calls in test files:

```bash
rg "insert_user" src/ --type rust
```

Update each call to wrap the profile_id arg in `Some(...)`.

- [ ] **Step 8: Run tests and lint**

```bash
just lint && just test
```

Expected: all pass. The migration runs automatically when tests open an in-memory DB.

- [ ] **Step 9: Commit**

```bash
git add migrations/008_nullable_profile_id.sql src/db/users.rs src/db/tests.rs src/web/handlers/admin.rs src/web/handlers/auth.rs src/cli/setup.rs
git commit -m "feat(db): make spt_profile_id nullable for web-only admin accounts"
```

---

### Task 2: Update Error Messages and Remove `quma init`

**Files:**
- Delete: `src/cli/init.rs`
- Modify: `src/cli/mod.rs:6,46-56,59-62` (remove Init, update Setup)
- Modify: `src/main.rs:54-75` (remove Init dispatch, update Setup dispatch)
- Modify: `src/error.rs:9` (update error message)
- Modify: `src/cli/serve.rs:47` (update error message)

**Interfaces:**
- Consumes: nothing from other tasks
- Produces: `Command::Setup { path: Option<PathBuf>, no_fika: bool }` enum variant

- [ ] **Step 1: Update `Command` enum in `src/cli/mod.rs`**

Replace the `Setup` and `Init` variants:

```rust
/// Bootstrap or initialize Quartermaster for an SPT server
Setup {
    /// Data directory path (default: ~/spt-server)
    path: Option<PathBuf>,
    /// Skip Fika installation
    #[arg(long)]
    no_fika: bool,
},
```

Remove the `Init` variant entirely:
```rust
// DELETE:
/// Initialize Quartermaster for an SPT server
Init {
    /// SPT directory path (auto-detects if omitted)
    path: Option<PathBuf>,
},
```

Remove `pub mod init;` from the module declarations at the top.

- [ ] **Step 2: Update `main.rs` dispatch**

In `src/main.rs`, replace the Setup and Init match arms:

```rust
Command::Setup {
    path,
    no_fika,
} => {
    let filter = logging::resolve_log_filter(
        &config::LoggingConfig::default(),
        cli.verbose,
        cli.log_level.as_deref(),
    );
    reload_handles.reconfigure(&config::LoggingConfig::default(), &filter, None);
    cli::setup::run(path.clone(), *no_fika, &cli).await
}
```

Remove the `Command::Init { path }` arm entirely.

- [ ] **Step 3: Update error messages**

In `src/error.rs:9`:
```rust
// Before:
#[error("SPT directory not found — run `quma init` or pass --spt-dir")]
// After:
#[error("SPT directory not found — run `quma setup` or pass --spt-dir")]
```

In `src/cli/serve.rs:47`:
```rust
// Before:
anyhow::bail!("No admin user exists. Run `quma init` first to create an admin account.");
// After:
anyhow::bail!("No admin user exists. Run `quma setup` first to create an admin account.");
```

- [ ] **Step 4: Delete `src/cli/init.rs`**

```bash
rm src/cli/init.rs
```

- [ ] **Step 5: Verify compilation**

```bash
just check
```

Expected: compiles with no errors. Tests will fail because `setup::run` signature changed — that's expected and fixed in Task 3.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(cli): remove quma init, update setup command signature"
```

---

### Task 3: Fix Container Mount Path

**Files:**
- Modify: `src/cli/server.rs:113` (container_path)

**Interfaces:**
- Consumes: nothing
- Produces: correct `/opt/server` mount path for `server create`

The existing `server create` command uses `/opt/tarkov` but the `fika-spt-server-docker` image expects `/opt/server`. Fix it now so both `server create` and the new `setup` use the same correct path.

- [ ] **Step 1: Update the container path in `server create`**

In `src/cli/server.rs:113`:
```rust
// Before:
container_path: "/opt/tarkov".to_string(),
// After:
container_path: "/opt/server".to_string(),
```

- [ ] **Step 2: Run tests**

```bash
just test
```

- [ ] **Step 3: Commit**

```bash
git add src/cli/server.rs
git commit -m "fix: correct container mount path from /opt/tarkov to /opt/server"
```

---

### Task 4: Rewrite `quma setup`

**Files:**
- Modify: `Cargo.toml` (add `dirs` dependency)
- Modify: `src/cli/setup.rs` (full rewrite)
- Modify: `src/cli/common.rs` (if `find_unmanaged_mod_dirs` needs to be made public — verify)

**Interfaces:**
- Consumes: `ContainerManager` (pull_image, create_container, start, stop, is_running, detect_spt_containers, inspect), `Database` (open, insert_user, admin_exists), `Config` (load, save, ensure_session_secret, resolve_path), `validate_spt_dir`, `read_spt_version`, `SptClient::ping`, `hash_password`, `find_unmanaged_mod_dirs`
- Produces: `pub async fn run(path: Option<PathBuf>, no_fika: bool, cli: &Cli) -> Result<()>`

- [ ] **Step 1: Add `dirs` dependency**

Add to `[dependencies]` in `Cargo.toml`:

```toml
dirs = "6"
```

Run `cargo check` to update `Cargo.lock`.

- [ ] **Step 2: Write the new `setup.rs`**

Replace the entire contents of `src/cli/setup.rs` with:

```rust
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::container::{
    ContainerManager, CreateContainerOpts, PortMapping, Protocol, SelinuxLabel, VolumeMount,
    DEFAULT_CONTAINER_NAME, DEFAULT_SPT_PORT, SPT_SERVER_IMAGE,
};
use crate::db::users::Role;
use crate::db::Database;
use crate::spt::detect::{read_spt_version, validate_spt_dir};
use crate::web::auth::hash_password;

use super::common::find_unmanaged_mod_dirs;
use super::Cli;

pub async fn run(path: Option<PathBuf>, no_fika: bool, cli: &Cli) -> Result<()> {
    println!("=== Quartermaster Setup ===\n");

    // --- Collect input ---
    let data_dir = resolve_data_dir(path)?;
    let install_fika = if no_fika {
        false
    } else {
        prompt_fika()?
    };
    let admin_password = prompt_admin_password()?;

    // --- Detect path ---
    let mgr = ContainerManager::new().context(
        "No container runtime found. Install Podman or Docker and ensure the socket is enabled.",
    )?;

    let dir_state = classify_directory(&data_dir)?;

    match dir_state {
        DirState::Empty => {
            bootstrap(&mgr, &data_dir, install_fika, &admin_password, cli).await
        }
        DirState::ExistingSpt => {
            wrap_existing(&mgr, &data_dir, install_fika, &admin_password, cli).await
        }
    }
}

enum DirState {
    Empty,
    ExistingSpt,
}

fn classify_directory(path: &Path) -> Result<DirState> {
    if !path.exists() {
        return Ok(DirState::Empty);
    }

    if path.is_file() {
        bail!("{} is a file, not a directory.", path.display());
    }

    // Check if empty
    let mut entries = std::fs::read_dir(path)
        .with_context(|| format!("failed to read directory {}", path.display()))?;

    if entries.next().is_none() {
        return Ok(DirState::Empty);
    }

    // Non-empty — check if it's a valid SPT install
    if validate_spt_dir(path).is_ok() {
        return Ok(DirState::ExistingSpt);
    }

    bail!(
        "Directory {} exists and contains files but is not a valid SPT installation.\n\
         Use an empty directory for a fresh setup, or point at an existing SPT install.",
        path.display()
    );
}

fn resolve_data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }

    let default = dirs::home_dir()
        .map(|h| h.join("spt-server"))
        .unwrap_or_else(|| PathBuf::from("./spt-server"));

    print!("Where should server data live? [{}]: ", default.display());
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        Ok(default)
    } else {
        Ok(PathBuf::from(trimmed))
    }
}

fn prompt_fika() -> Result<bool> {
    print!("Install Fika for multiplayer? [Y/n]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    Ok(trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("y")
        || trimmed.eq_ignore_ascii_case("yes"))
}

fn prompt_admin_password() -> Result<String> {
    loop {
        let password = rpassword::prompt_password("Admin password (min 8 chars): ")
            .context("failed to read password")?;

        if password.len() < 8 {
            println!("Password must be at least 8 characters. Try again.");
            continue;
        }

        let confirm = rpassword::prompt_password("Confirm password: ")
            .context("failed to read password confirmation")?;

        if password != confirm {
            println!("Passwords do not match. Try again.");
            continue;
        }

        return Ok(password);
    }
}

fn create_container_opts(data_dir: &Path, install_fika: bool) -> CreateContainerOpts {
    let fika_mode = if install_fika { "install" } else { "disabled" };

    CreateContainerOpts {
        name: DEFAULT_CONTAINER_NAME.to_string(),
        image: SPT_SERVER_IMAGE.to_string(),
        env: vec![
            ("LISTEN_ALL_NETWORKS".to_string(), "true".to_string()),
            ("FIKA_MODE".to_string(), fika_mode.to_string()),
        ],
        volumes: vec![VolumeMount {
            host_path: data_dir.to_path_buf(),
            container_path: "/opt/server".to_string(),
            read_only: false,
            selinux: SelinuxLabel::Private,
        }],
        ports: vec![PortMapping {
            host_port: DEFAULT_SPT_PORT,
            container_port: DEFAULT_SPT_PORT,
            protocol: Protocol::Tcp,
        }],
        labels: vec![],
        user: None,
    }
}

async fn check_container_name_available(mgr: &ContainerManager) -> Result<()> {
    match mgr.inspect(DEFAULT_CONTAINER_NAME).await {
        Ok(_) => bail!(
            "Container '{}' already exists. Remove it with \
             `podman rm {0}` or `docker rm {0}` and re-run setup.",
            DEFAULT_CONTAINER_NAME
        ),
        Err(_) => Ok(()),
    }
}

async fn wait_for_server(config: &Config, spt_dir: &Path) -> Result<()> {
    let (host, port) = crate::server_detect::resolve_server_addr(config, spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port)?;

    println!("Waiting for server to start (timeout: 180s)...");
    let start_time = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(180);

    loop {
        if start_time.elapsed() > timeout {
            bail!(
                "Server did not respond within 180s. Check container logs with \
                 `podman logs {}` or `docker logs {0}`.",
                DEFAULT_CONTAINER_NAME
            );
        }

        match spt_client.ping().await {
            Ok(ping) if ping.ok => {
                println!("Server is ready (responded in {}ms).", ping.latency_ms);
                return Ok(());
            }
            _ => {
                // Connection refused or not ready yet — keep waiting
                print!(".");
                std::io::stdout().flush()?;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}

fn create_config(data_dir: &Path, cli: &Cli) -> Result<(Config, PathBuf)> {
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(data_dir));
    let mut config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.spt_dir = Some(data_dir.to_path_buf());
    config.server_container = Some(DEFAULT_CONTAINER_NAME.to_string());
    config.server_host = Some("0.0.0.0".to_string());
    config.server_port = Some(DEFAULT_SPT_PORT);
    config.ensure_session_secret();
    config.save(&config_path)?;
    println!("Config saved to {}", config_path.display());
    Ok((config, config_path))
}

fn create_db_and_admin(data_dir: &Path, admin_password: &str) -> Result<Database> {
    let db_path = data_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    if db.admin_exists()? {
        println!("Admin user already exists.");
    } else {
        let password_hash = hash_password(admin_password)?;
        db.insert_user("admin", None, Some(&password_hash), Role::Admin)
            .map_err(|e| anyhow::anyhow!("failed to create admin user: {e}"))?;
        println!("Admin user 'admin' created.");
    }

    Ok(db)
}

fn print_summary(config: &Config, data_dir: &Path, install_fika: bool) {
    println!("\n=== Setup Complete ===\n");
    println!("SPT directory: {}", data_dir.display());
    if let Some(ref container) = config.server_container {
        println!("Container: {container}");
    }
    println!("Fika: {}", if install_fika { "installed" } else { "disabled" });
    println!("Web UI: http://{}:{}", config.web_bind, config.web_port);
    println!("Admin user: admin");
    println!("\nNext steps:");
    println!("  quma serve              Start the web UI");
    println!("  quma server start       Start the SPT server");
    println!("  quma invite             Generate invite codes for players");
    println!("\nNetwork requirements (for multiplayer):");
    println!("  TCP 6969 inbound        SPT server");
    println!("  UDP 25565 inbound       Fika P2P raids (whoever hosts)");
    println!("  Consider UPnP or VPN as alternatives to port forwarding.");
}

// --- Path A: Bootstrap ---

async fn bootstrap(
    mgr: &ContainerManager,
    data_dir: &Path,
    install_fika: bool,
    admin_password: &str,
    cli: &Cli,
) -> Result<()> {
    println!("\nNo existing SPT installation found. Bootstrapping from scratch...\n");

    // 1. Create data directory
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("failed to create directory {}", data_dir.display()))?;
    println!("Created {}", data_dir.display());

    // 2. Check container name available
    check_container_name_available(mgr).await?;

    // 3. Pull image
    println!("Pulling {}...", SPT_SERVER_IMAGE);
    mgr.pull_image(SPT_SERVER_IMAGE).await?;
    println!("Image pulled.");

    // 4. Create container
    let opts = create_container_opts(data_dir, install_fika);
    mgr.create_container(opts).await?;
    println!("Container '{}' created.", DEFAULT_CONTAINER_NAME);

    // 5. First boot
    println!("\nStarting first boot...");
    mgr.start(DEFAULT_CONTAINER_NAME).await?;

    // 6. Create config (needed for wait_for_server to resolve address)
    let (config, _config_path) = create_config(data_dir, cli)?;

    // 7. Wait for server
    wait_for_server(&config, data_dir).await?;

    // 8. Stop server
    println!("\nStopping server after first boot...");
    mgr.stop(DEFAULT_CONTAINER_NAME).await?;
    println!("Server stopped.");

    // 9. Create DB and admin
    create_db_and_admin(data_dir, admin_password)?;

    // 10. Summary
    print_summary(&config, data_dir, install_fika);

    Ok(())
}

// --- Path B: Wrap Existing ---

async fn wrap_existing(
    mgr: &ContainerManager,
    data_dir: &Path,
    install_fika: bool,
    admin_password: &str,
    cli: &Cli,
) -> Result<()> {
    let spt_info = read_spt_version(data_dir)?;
    println!(
        "\nExisting SPT {} (EFT {}) detected.\n",
        spt_info.spt_version, spt_info.tarkov_version
    );

    // 1. Detect or create container
    let container_name = detect_or_create_container(mgr, data_dir, install_fika).await?;

    // 2. Create config
    let (mut config, config_path) = create_config(data_dir, cli)?;
    config.server_container = Some(container_name);
    config.save(&config_path)?;

    // 3. Create DB and admin
    let db = create_db_and_admin(data_dir, admin_password)?;

    // 4. Scan unmanaged mods
    let (unmanaged_dirs, unmanaged_count) = find_unmanaged_mod_dirs(data_dir, &db)?;
    if unmanaged_dirs.is_empty() {
        println!("No unmanaged mod files found.");
    } else {
        println!(
            "Found {} unmanaged mod director{} ({} files).",
            unmanaged_dirs.len(),
            if unmanaged_dirs.len() == 1 { "y" } else { "ies" },
            unmanaged_count
        );
        for dir in unmanaged_dirs.keys() {
            println!("  {}", dir);
        }
        println!("Use `quma track <path> <forge_mod_id>` to associate them.");
    }

    // 5. Summary
    print_summary(&config, data_dir, install_fika);

    Ok(())
}

async fn detect_or_create_container(
    mgr: &ContainerManager,
    data_dir: &Path,
    install_fika: bool,
) -> Result<String> {
    let detected = mgr.detect_spt_containers(data_dir).await?;

    if detected.len() == 1 {
        println!("Detected existing container: {}", detected[0]);
        return Ok(detected[0].clone());
    }

    if detected.len() > 1 {
        // Prefer quma-managed container
        // detect_containers_by_label isn't async-friendly here, so just pick first
        println!(
            "Multiple containers detected, using first: {}",
            detected[0]
        );
        return Ok(detected[0].clone());
    }

    // No container found — create one
    println!("No existing container found. Creating one...");
    check_container_name_available(mgr).await?;

    println!("Pulling {}...", SPT_SERVER_IMAGE);
    mgr.pull_image(SPT_SERVER_IMAGE).await?;

    let opts = create_container_opts(data_dir, install_fika);
    mgr.create_container(opts).await?;
    println!("Container '{}' created.", DEFAULT_CONTAINER_NAME);

    Ok(DEFAULT_CONTAINER_NAME.to_string())
}
```

- [ ] **Step 2: Verify compilation**

```bash
just check
```

Expected: compiles. Some tests in `setup.rs` will fail because the old tests reference removed functions — that's addressed in the next step.

- [ ] **Step 3: Update setup tests**

Replace the `#[cfg(test)] mod tests` block at the bottom of `src/cli/setup.rs` with tests for the new helper functions:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let state = classify_directory(tmp.path()).unwrap();
        assert!(matches!(state, DirState::Empty));
    }

    #[test]
    fn classify_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        let state = classify_directory(&nonexistent).unwrap();
        assert!(matches!(state, DirState::Empty));
    }

    #[test]
    fn classify_valid_spt_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Create minimum SPT structure
        std::fs::create_dir_all(spt_dir.join("SPT")).unwrap();
        std::fs::write(spt_dir.join("SPT/SPT.Server.exe"), b"").unwrap();
        let configs_dir = spt_dir.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("core.json"),
            r#"{"compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let state = classify_directory(spt_dir).unwrap();
        assert!(matches!(state, DirState::ExistingSpt));
    }

    #[test]
    fn classify_non_spt_nonempty_dir_fails() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("random.txt"), b"hello").unwrap();
        let result = classify_directory(tmp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a valid SPT installation"));
    }

    #[test]
    fn classify_file_path_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("afile");
        std::fs::write(&file, b"data").unwrap();
        let result = classify_directory(&file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("is a file"));
    }

    #[test]
    fn create_container_opts_fika_enabled() {
        let dir = PathBuf::from("/data/spt");
        let opts = create_container_opts(&dir, true);
        assert!(opts.env.iter().any(|(k, v)| k == "FIKA_MODE" && v == "install"));
        assert_eq!(opts.name, "spt-server");
        assert_eq!(opts.volumes[0].container_path, "/opt/server");
    }

    #[test]
    fn create_container_opts_fika_disabled() {
        let dir = PathBuf::from("/data/spt");
        let opts = create_container_opts(&dir, false);
        assert!(opts.env.iter().any(|(k, v)| k == "FIKA_MODE" && v == "disabled"));
    }
}
```

- [ ] **Step 4: Run tests and lint**

```bash
just lint && just test
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/cli/setup.rs
git commit -m "feat: rewrite quma setup with bootstrap and wrap-existing paths"
```

---

### Task 5: Integration Verification

**Files:**
- No new files

**Interfaces:**
- Consumes: everything from Tasks 1-4

This task verifies all pieces work together.

- [ ] **Step 1: Full lint and test suite**

```bash
just lint && just test
```

Expected: all pass.

- [ ] **Step 2: Verify `--help` output**

```bash
cargo run -- setup --help
```

Expected output should show:
```
Bootstrap or initialize Quartermaster for an SPT server

Usage: quma setup [PATH] [--no-fika]

Arguments:
  [PATH]  Data directory path (default: ~/spt-server)

Options:
      --no-fika  Skip Fika installation
  -h, --help     Print help
```

- [ ] **Step 3: Verify `quma init` is gone**

```bash
cargo run -- init 2>&1
```

Expected: error about unrecognized subcommand.

- [ ] **Step 4: Verify no stale references**

```bash
rg "quma init" src/
rg "skip_fika\|non_interactive" src/cli/setup.rs
rg "cli::init" src/
```

Expected: no hits for any of these.

- [ ] **Step 5: Verify error messages**

```bash
rg "quma setup" src/error.rs src/cli/serve.rs
```

Expected: both files reference `quma setup`, not `quma init`.

- [ ] **Step 6: Commit (if any fixups needed)**

```bash
git add -A
git commit -m "fix: address integration issues from setup rework"
```

Only commit if there were changes to make. Skip if everything passed clean.
