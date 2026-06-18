# High-Priority Spec Gaps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close three high-priority gaps between the SPEC.md and the implementation: (1) wire `loadedServerMods` into health checks, (2) pass the explicit `version` CLI arg through to `quma install`, and (3) add rate limiting on `/login` and `/register`.

**Architecture:** Each task modifies 2–4 existing files. No new modules or tables. Task 1 extends the health check pipeline with a server-mod-load comparison. Task 2 threads a `version: Option<&str>` through the install dispatch and version-selection logic. Task 3 adds `actix-governor` rate limiting scoped to the login and register routes.

**Tech Stack:** Rust, actix-web 4, actix-governor, reqwest, rusqlite, clap (derive), askama

## Global Constraints

- Linux only for v1
- SPT 4.0+ only
- Existing test suite must continue to pass (`cargo test`)
- Follow existing patterns (existing modules, `CliContext`, `web::block()` for DB access)
- `cargo clippy -- -D warnings` must pass

---

### Task 1: Wire `loadedServerMods` into Health Checks

**Files:**
- Modify: `src/health.rs` (add `loaded_count`, `load_failures`, `untracked_loaded` to `ModsHealth`; call `loaded_server_mods()` from `run_checks`)
- Modify: `src/cli/status.rs` (display the new fields in CLI output)
- Modify: `templates/partials/status_detail.html` (display load status in web UI)

**Interfaces:**
- Consumes:
  - `SptClient::loaded_server_mods(&self) -> Result<HashMap<String, serde_json::Value>>` from `src/spt/server.rs:79`
  - `Database::list_mods(&self) -> rusqlite::Result<Vec<InstalledMod>>` from `src/db/mods.rs:99`
  - `InstalledMod.name: String` from `src/db/mods.rs:7`
- Produces:
  - `ModsHealth.loaded_count: Option<usize>` — number of mods the server reports loaded (None if server unreachable)
  - `ModsHealth.load_failures: Vec<String>` — mod names installed in DB but not loaded by server
  - `ModsHealth.untracked_loaded: Vec<String>` — mod names loaded by server but not in DB

- [ ] **Step 1: Write the failing test for mod load comparison logic**

Add a new unit-testable function `check_mod_loads` to `src/health.rs` and a test. This function takes the DB mod list and the server-loaded map and returns the two comparison vectors, so it can be tested without network access.

In `src/health.rs`, add above the existing `#[cfg(test)]` block:

```rust
/// Compare installed mods (from DB) against loaded mods (from SPT server).
/// Uses case-insensitive name matching because the SPT server may report
/// mod names using the package.json `name` field which can differ in casing
/// from the Forge display name stored in the DB.
pub fn check_mod_loads(
    installed_mods: &[crate::db::mods::InstalledMod],
    loaded_mods: &std::collections::HashMap<String, serde_json::Value>,
) -> (Vec<String>, Vec<String>) {
    let installed_lower: std::collections::HashSet<String> = installed_mods
        .iter()
        .map(|m| m.name.to_lowercase())
        .collect();

    let loaded_lower: std::collections::HashSet<String> = loaded_mods
        .keys()
        .map(|k| k.to_lowercase())
        .collect();

    let load_failures: Vec<String> = installed_mods
        .iter()
        .filter(|m| !loaded_lower.contains(&m.name.to_lowercase()))
        .map(|m| m.name.clone())
        .collect();

    let untracked_loaded: Vec<String> = loaded_mods
        .keys()
        .filter(|name| !installed_lower.contains(&name.to_lowercase()))
        .cloned()
        .collect();

    (load_failures, untracked_loaded)
}
```

Then add tests inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn check_mod_loads_all_matching() {
    let installed = vec![
        InstalledMod { id: 1, forge_mod_id: 100, forge_version_id: 200, name: "ModA".to_string(), slug: None, version: "1.0.0".to_string(), installed_at: "2026-01-01T00:00:00Z".to_string(), updated_at: None },
        InstalledMod { id: 2, forge_mod_id: 101, forge_version_id: 201, name: "ModB".to_string(), slug: None, version: "2.0.0".to_string(), installed_at: "2026-01-01T00:00:00Z".to_string(), updated_at: None },
    ];
    let mut loaded = std::collections::HashMap::new();
    loaded.insert("ModA".to_string(), serde_json::json!({}));
    loaded.insert("ModB".to_string(), serde_json::json!({}));

    let (failures, untracked) = check_mod_loads(&installed, &loaded);
    assert!(failures.is_empty());
    assert!(untracked.is_empty());
}

#[test]
fn check_mod_loads_detects_load_failure() {
    let installed = vec![
        InstalledMod { id: 1, forge_mod_id: 100, forge_version_id: 200, name: "WorkingMod".to_string(), slug: None, version: "1.0.0".to_string(), installed_at: "2026-01-01T00:00:00Z".to_string(), updated_at: None },
        InstalledMod { id: 2, forge_mod_id: 101, forge_version_id: 201, name: "BrokenMod".to_string(), slug: None, version: "2.0.0".to_string(), installed_at: "2026-01-01T00:00:00Z".to_string(), updated_at: None },
    ];
    let mut loaded = std::collections::HashMap::new();
    loaded.insert("WorkingMod".to_string(), serde_json::json!({}));

    let (failures, untracked) = check_mod_loads(&installed, &loaded);
    assert_eq!(failures, vec!["BrokenMod"]);
    assert!(untracked.is_empty());
}

#[test]
fn check_mod_loads_detects_untracked() {
    let installed = vec![
        InstalledMod { id: 1, forge_mod_id: 100, forge_version_id: 200, name: "TrackedMod".to_string(), slug: None, version: "1.0.0".to_string(), installed_at: "2026-01-01T00:00:00Z".to_string(), updated_at: None },
    ];
    let mut loaded = std::collections::HashMap::new();
    loaded.insert("TrackedMod".to_string(), serde_json::json!({}));
    loaded.insert("ManualMod".to_string(), serde_json::json!({}));

    let (failures, untracked) = check_mod_loads(&installed, &loaded);
    assert!(failures.is_empty());
    assert_eq!(untracked, vec!["ManualMod"]);
}

#[test]
fn check_mod_loads_case_insensitive() {
    let installed = vec![
        InstalledMod { id: 1, forge_mod_id: 100, forge_version_id: 200, name: "SAIN".to_string(), slug: None, version: "1.0.0".to_string(), installed_at: "2026-01-01T00:00:00Z".to_string(), updated_at: None },
    ];
    let mut loaded = std::collections::HashMap::new();
    loaded.insert("sain".to_string(), serde_json::json!({}));

    let (failures, untracked) = check_mod_loads(&installed, &loaded);
    assert!(failures.is_empty(), "case-insensitive match should not report failure");
    assert!(untracked.is_empty(), "case-insensitive match should not report untracked");
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test health::tests::check_mod_loads`
Expected: 3 tests PASS

- [ ] **Step 3: Add new fields to `ModsHealth` struct**

In `src/health.rs`, update the `ModsHealth` struct (around line 27):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ModsHealth {
    pub installed_count: usize,
    pub loaded_count: Option<usize>,
    pub load_failures: Vec<String>,
    pub untracked_loaded: Vec<String>,
    pub updates_available: usize,
    pub incompatible_mods: Vec<String>,
}
```

- [ ] **Step 4: Update `check_mods_health` to accept and use loaded mods**

Change the `check_mods_health` function signature to accept the loaded mods map and populate the new fields. The map is `Option` because the server may be unreachable.

Replace the existing `check_mods_health` function:

```rust
pub async fn check_mods_health(
    installed_mods: &[crate::db::mods::InstalledMod],
    loaded_mods: Option<&std::collections::HashMap<String, serde_json::Value>>,
    forge: &crate::forge::client::ForgeClient,
    spt_version: &str,
) -> ModsHealth {
    let mut updates_available = 0;
    let mut incompatible_mods = Vec::new();

    if !installed_mods.is_empty() {
        let check_list: Vec<(i64, String)> = installed_mods
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();

        if let Ok(results) = forge.check_updates(&check_list, spt_version).await {
            updates_available = results.updates.len();

            for m in &results.incompatible_with_spt {
                incompatible_mods.push(m.name.clone());
            }
        }
    }

    let (loaded_count, load_failures, untracked_loaded) = match loaded_mods {
        Some(loaded) => {
            let (failures, untracked) = check_mod_loads(installed_mods, loaded);
            (Some(loaded.len()), failures, untracked)
        }
        None => (None, vec![], vec![]),
    };

    ModsHealth {
        installed_count: installed_mods.len(),
        loaded_count,
        load_failures,
        untracked_loaded,
        updates_available,
        incompatible_mods,
    }
}
```

- [ ] **Step 5: Update `run_checks` to fetch loaded mods and pass them through**

In `src/health.rs`, update `run_checks` (around line 66). After the server health check, attempt to fetch loaded mods if the server is reachable:

```rust
pub async fn run_checks(ctx: &CliContext) -> Result<HealthReport> {
    let (host, port) = resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    let server = check_server(&spt_client, &ctx.spt_info.spt_version, &address).await;

    let loaded_mods = if server.reachable {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let installed_mods = ctx.db.list_mods()?;
    let mods = check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &ctx.forge,
        &ctx.spt_info.spt_version,
    )
    .await;

    let tracked_files = ctx.db.get_all_tracked_files()?;
    let integrity = check_integrity_from(&tracked_files, &ctx.spt_dir)?;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}
```

- [ ] **Step 6: Update `exit_code` to include load failures**

In `src/health.rs`, update `exit_code` (around line 52) to also trigger exit code 2 on load failures:

```rust
pub fn exit_code(&self) -> i32 {
    if !self.server.reachable {
        return 1;
    }
    if !self.mods.incompatible_mods.is_empty()
        || !self.mods.load_failures.is_empty()
        || !self.integrity.missing_files.is_empty()
        || !self.integrity.modified_files.is_empty()
    {
        return 2;
    }
    0
}
```

- [ ] **Step 7: Fix the existing `good_mods()` test helper**

Update the `good_mods()` helper function in the tests module to include the new fields:

```rust
fn good_mods() -> ModsHealth {
    ModsHealth {
        installed_count: 5,
        loaded_count: Some(5),
        load_failures: vec![],
        untracked_loaded: vec![],
        updates_available: 0,
        incompatible_mods: vec![],
    }
}
```

- [ ] **Step 8: Update all inline `ModsHealth` constructions in existing tests**

The `exit_code_incompatible_mods` and `exit_code_server_down_trumps_mod_issues` tests construct `ModsHealth` inline. Add the three new fields to each.

In `exit_code_incompatible_mods` (around line 269), update the `ModsHealth`:
```rust
            mods: ModsHealth {
                installed_count: 5,
                loaded_count: None,
                load_failures: vec![],
                untracked_loaded: vec![],
                updates_available: 0,
                incompatible_mods: vec!["OldMod".to_string()],
            },
```

In `exit_code_server_down_trumps_mod_issues` (around line 320), update the `ModsHealth`:
```rust
            mods: ModsHealth {
                installed_count: 5,
                loaded_count: None,
                load_failures: vec![],
                untracked_loaded: vec![],
                updates_available: 0,
                incompatible_mods: vec!["X".to_string()],
            },
```

- [ ] **Step 9: Add exit code test for load failures**

Add inside the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn exit_code_load_failures() {
    let report = HealthReport {
        server: good_server(),
        mods: ModsHealth {
            installed_count: 5,
            loaded_count: Some(4),
            load_failures: vec!["BrokenMod".to_string()],
            untracked_loaded: vec![],
            updates_available: 0,
            incompatible_mods: vec![],
        },
        integrity: good_integrity(),
    };
    assert_eq!(report.exit_code(), 2);
}
```

- [ ] **Step 10: Update web status handler to fetch loaded mods**

In `src/web/handlers/status.rs`, update `build_health_report` to also call `loaded_server_mods()`:

```rust
async fn build_health_report(state: &AppState) -> anyhow::Result<HealthReport> {
    use crate::server_detect::resolve_server_addr;
    use crate::spt::server::SptClient;

    let (host, port) = resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    let server = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;

    let loaded_mods = if server.reachable {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let db = state.db.clone();
    let (installed_mods, tracked_files) = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let files = db.get_all_tracked_files()?;
        Ok::<_, anyhow::Error>((mods, files))
    })
    .await??;

    let mods = health::check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &state.forge,
        &state.spt_info.spt_version,
    )
    .await;

    let spt_dir = state.spt_dir.clone();
    let integrity =
        web::block(move || health::check_integrity_from(&tracked_files, &spt_dir)).await??;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}
```

- [ ] **Step 11: Update CLI status output for load info**

In `src/cli/status.rs`, update the `print_report` function. Replace the `Mods` section (starting at the `println!("Mods ({} installed)"` line):

```rust
    println!();
    match report.mods.loaded_count {
        Some(loaded) => println!(
            "Mods ({} installed, {} loaded)",
            report.mods.installed_count, loaded
        ),
        None => println!("Mods ({} installed)", report.mods.installed_count),
    }

    if !report.mods.load_failures.is_empty() {
        for name in &report.mods.load_failures {
            println!("  FAILED TO LOAD: {}", name);
        }
    }

    if !report.mods.untracked_loaded.is_empty() {
        for name in &report.mods.untracked_loaded {
            println!("  UNTRACKED (loaded but not managed): {}", name);
        }
    }
```

Keep the rest of the mods section (incompatible_mods warning, updates_available, etc.) unchanged.

- [ ] **Step 12: Update web status template for load info**

In `templates/partials/status_detail.html`, update the Mods card. Replace the `<tr><th style="width:140px">Installed</th>` line and add rows after it:

```html
<tr><th style="width:140px">Installed</th><td>{{ report.mods.installed_count }}</td></tr>
{% if let Some(loaded) = report.mods.loaded_count %}
<tr><th>Loaded</th><td>{{ loaded }}</td></tr>
{% endif %}
{% if !report.mods.load_failures.is_empty() %}
<tr><th>Load Failures</th><td>
    {% for name in &report.mods.load_failures %}
    <span class="badge badge-danger">{{ name }}</span>
    {% endfor %}
</td></tr>
{% endif %}
{% if !report.mods.untracked_loaded.is_empty() %}
<tr><th>Untracked</th><td>
    {% for name in &report.mods.untracked_loaded %}
    <span class="badge badge-warning">{{ name }}</span>
    {% endfor %}
</td></tr>
{% endif %}
```

- [ ] **Step 13: Remove `#![allow(dead_code)]` from `src/spt/server.rs`**

The `loaded_server_mods()` method is now used. Remove the module-level dead-code suppression at `src/spt/server.rs:2` and the comment at line 1. Note: the parent module `src/spt/mod.rs:3` also has `#![allow(dead_code)]` which will still suppress warnings on other unused items in the `spt` module — that's fine, this step is about cleaning up the server.rs-specific suppression.

Delete lines 1-2:
```rust
// SPT server client is used by health checks and server lifecycle (tasks 15-16).
#![allow(dead_code)]
```

- [ ] **Step 14: Run all tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 15: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings or errors

- [ ] **Step 16: Commit**

```bash
git add src/health.rs src/cli/status.rs src/web/handlers/status.rs src/spt/server.rs templates/partials/status_detail.html
git commit -m "feat: wire loadedServerMods into health checks

Compare installed mods against what the SPT server reports as loaded.
Detect mods that failed to load and mods loaded but not tracked in DB."
```

---

### Task 2: Wire Explicit Version Arg for `quma install`

**Files:**
- Modify: `src/main.rs:30-37` (pass `version` to `install::run`)
- Modify: `src/cli/install.rs:55,113-137` (accept `version` param, use it in `pick_version`)

**Interfaces:**
- Consumes:
  - `ForgeClient::get_versions(mod_id, spt_version_filter) -> Result<Vec<ForgeVersion>>` from `src/forge/client.rs:98`
  - `ForgeVersion.version: String` from `src/forge/models.rs:55`
- Produces:
  - `install::run(mod_ref, version, force, ctx)` — updated signature accepting `Option<&str>` version

- [ ] **Step 1: Update `install::run` signature to accept version**

In `src/cli/install.rs`, update the `run` function signature (line 55):

```rust
pub async fn run(mod_ref: &str, version: Option<&str>, force: bool, ctx: &CliContext) -> Result<()> {
```

And update the call to `pick_version` (line 67) to pass the version through:

```rust
    let selected_version = pick_version(ctx, &forge_mod, version).await?;
```

- [ ] **Step 2: Update `pick_version` to use explicit version when provided**

Replace the `pick_version` function (lines 113-137):

```rust
async fn pick_version(
    ctx: &CliContext,
    forge_mod: &crate::forge::models::ForgeMod,
    explicit_version: Option<&str>,
) -> Result<ForgeVersion> {
    let versions = ctx
        .forge
        .get_versions(forge_mod.id, Some(&ctx.spt_info.spt_version))
        .await?;

    let selected = match explicit_version {
        Some(ver) => {
            // If explicit version doesn't match any SPT-compatible version,
            // try fetching all versions unfiltered
            let found = versions.iter().find(|v| v.version == ver);
            match found {
                Some(v) => v.clone(),
                None => {
                    let all_versions = ctx.forge.get_versions(forge_mod.id, None).await?;
                    all_versions
                        .into_iter()
                        .find(|v| v.version == ver)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "version '{}' not found for {} on Forge",
                                ver,
                                forge_mod.name
                            )
                        })?
                }
            }
        }
        None => versions.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!(
                "no versions of {} are compatible with SPT {}",
                forge_mod.name,
                ctx.spt_info.spt_version
            )
        })?,
    };

    println!(
        "Selected version: {} (SPT {})",
        selected.version,
        selected.spt_version.as_deref().unwrap_or("unknown")
    );
    Ok(selected)
}
```

- [ ] **Step 3: Update dispatch in `main.rs`**

In `src/main.rs`, update the `Install` match arm (lines 29-38). Change `version: _` to `version` and pass it through:

```rust
        Command::Install {
            mod_ref,
            version,
            force,
        } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::install::run(mod_ref, version.as_deref(), *force, &ctx).await
        }
```

- [ ] **Step 4: Remove the TODO comment**

In `src/cli/install.rs`, delete the comment at line 122:
```
    // TODO: accept explicit version arg when we refactor CLI dispatch
```

- [ ] **Step 5: Remove the debt comment from `main.rs`**

In `src/main.rs`, delete the comment (lines 34-35):
```
            // TODO(debt): version selection is handled inside run() for now;
            // wire explicit version arg when CLI dispatch is refactored
```

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 7: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings or errors

- [ ] **Step 8: Verify CLI help text**

Run: `cargo run -- install --help`
Expected: shows `[VERSION]` positional argument in usage line

- [ ] **Step 9: Commit**

```bash
git add src/main.rs src/cli/install.rs
git commit -m "feat: wire explicit version argument for quma install

quma install <mod> [version] now uses the explicit version when
provided instead of always picking the latest compatible version."
```

---

### Task 3: Add Rate Limiting on Auth Endpoints

**Files:**
- Modify: `Cargo.toml` (add `actix-governor` dependency)
- Modify: `src/web/mod.rs` (configure rate limiter, wrap auth routes)

**Interfaces:**
- Consumes:
  - `actix_governor::Governor` middleware
  - `actix_governor::GovernorConfigBuilder` for configuration
- Produces:
  - Rate limiting on `POST /login`, `GET /register`, `POST /register` at 5 requests per minute per IP

- [ ] **Step 1: Add `actix-governor` to `Cargo.toml`**

Add to the `[dependencies]` section in `Cargo.toml`, after the `actix-session` line:

```toml
actix-governor = "0.10"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles without errors

- [ ] **Step 3: Configure rate limiter and wrap auth routes**

In `src/web/mod.rs`, add the import at the top (after the existing `use` block):

```rust
use actix_governor::{Governor, GovernorConfigBuilder};
```

Then inside `start_server`, before the `HttpServer::new(move ||` closure, create the governor config. It must be created outside the closure — `Governor::new(&governor_conf)` clones share the same underlying rate limiter, which is what we want.

```rust
    let governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(12) // 5 per minute = 1 per 12 seconds replenish
        .burst_size(5)           // allow bursting up to 5
        .finish()
        .expect("invalid governor config");
```

Then inside the closure, replace the current auth routes block (lines 78-84, the two TODO comments + the 5 `.route()` lines) with:

```rust
            // TODO(debt): add CSRF protection on state-mutating POST forms (SameSite=Strict mitigates most vectors)
            // Auth routes (public)
            .route("/login", web::get().to(handlers::auth::login_page))
            .route("/logout", web::post().to(handlers::auth::logout))
            // Rate-limited auth routes (5 req/min/IP on login POST + register)
            .service(
                web::resource("/login")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::post().to(handlers::auth::login_submit))
            )
            .service(
                web::resource("/register")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::get().to(handlers::auth::register_page))
                    .route(web::post().to(handlers::auth::register_submit))
            )
```

This rate-limits `POST /login`, `GET /register`, and `POST /register` per the spec. `GET /login` (viewing the login form) and `POST /logout` are NOT rate-limited — spec only requires login + register. The `Governor::new(&governor_conf)` calls share the same underlying rate limiter via `GovernorConfig`'s internal `Arc`.

Note: `GET /login` is registered as a bare `.route()` before the `web::resource("/login")`. actix-web matches routes in registration order — the bare GET route matches first for GET requests, and the resource matches for POST requests.

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings or errors

- [ ] **Step 6: Verify it compiles**

Run: `cargo build`
Expected: compiles successfully. (Full server start requires an SPT directory, but compilation verifies the middleware wiring is correct.)

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/web/mod.rs
git commit -m "feat: add rate limiting on /login and /register

Uses actix-governor at 5 requests per minute per IP on login POST
and register endpoints to prevent brute-force attacks."
```

---

## Self-Review

**Spec coverage:**
- "Mod load verification — `GET /launcher/server/loadedServerMods`": Task 1 wires this into both CLI and web health checks, comparing installed vs loaded, detecting failures and untracked mods. ✅
- "If `[version]` not specified, pick latest version compatible with current SPT version": Task 2 makes the explicit version argument functional. ✅
- "Rate-limited to 5 requests per minute per IP via `actix-governor`": Task 3 adds `actix-governor` on login+register routes. ✅

**Placeholder scan:** No TBD/TODO/implement-later placeholders. All steps have concrete code.

**Type consistency:** `ModsHealth` new fields (`loaded_count: Option<usize>`, `load_failures: Vec<String>`, `untracked_loaded: Vec<String>`) are used consistently across `health.rs`, `status.rs`, `status_detail.html`, and `handlers/status.rs`. All inline `ModsHealth` constructions in existing tests are updated (Step 8). The `check_mods_health` signature change (added `loaded_mods: Option<&HashMap<...>>`) is applied at both call sites (`run_checks` in `health.rs` and `build_health_report` in `handlers/status.rs`). The `install::run` signature change (`version: Option<&str>`) matches the call site in `main.rs`.

**Notable decisions:**
- `check_mod_loads` is a pure function taking the two data structures, making it unit-testable without mocking HTTP or database.
- `loaded_count` is `Option<usize>` (not just `usize`) so the UI can distinguish "server unreachable" from "0 mods loaded".
- Mod name comparison is case-insensitive because the SPT server may report package names with different casing than the Forge display name stored in the DB.
- Rate limiter uses `actix-governor` 0.10 (GPL-3.0, compatible with project's intended AGPL license).
- Rate limiter wraps `POST /login`, `GET /register`, and `POST /register` per spec. `GET /login` and `POST /logout` are excluded (no reason to rate-limit viewing the login form or logging out). Uses `web::resource()` to apply middleware to specific routes without a conflicting `web::scope("")`.
- `seconds_per_request(12)` with `burst_size(5)` maps to the spec's "5 requests per minute per IP".
- When an explicit version is provided but not found in SPT-compatible results, we fall back to fetching all versions unfiltered. This lets users install a specific version even if it's not marked compatible (they'll still get the Fika compat warning separately).
- Removing `#![allow(dead_code)]` from `src/spt/server.rs` is cosmetic — the parent `src/spt/mod.rs` has the same suppression. But worth cleaning since `server.rs` no longer has dead code.
- `GET /login` is registered as a bare `.route()` before `web::resource("/login")` — actix-web matches in registration order, so GET hits the bare route and POST hits the rate-limited resource.
