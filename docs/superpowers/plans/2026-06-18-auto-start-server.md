# Auto-Start Server Container on `quma serve`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When `quma serve` starts the web UI, automatically start the configured Podman container if it exists and isn't already running, with a config opt-out via `auto_start_server = false`.

**Architecture:** Add an `auto_start_server` boolean to `Config` (default `true`). In `serve.rs`, after loading config and before calling `start_server()`, check if a `server_container` is configured AND `auto_start_server` is true — if so, inspect the container and start it if not running. Failures log a warning but don't block the web server from starting.

**Tech Stack:** Rust, serde (TOML config), tokio (async podman calls), tracing (logging)

## Global Constraints

- `auto_start_server` defaults to `true` — existing configs without the field get auto-start behavior
- Container start failures must NOT prevent the web server from starting (warn and continue)
- Env var override: `QUMA_AUTO_START_SERVER` (`true`/`false`)

---

### Task 1: Add `auto_start_server` config field

**Files:**
- Modify: `src/config.rs:170-227` (Config struct + Default impl)
- Modify: `src/config.rs:281-352` (apply_env_overrides)
- Modify: `src/config.rs:387-664` (tests)

**Interfaces:**
- Produces: `Config.auto_start_server: bool` (default `true`), `QUMA_AUTO_START_SERVER` env override

- [ ] **Step 1: Write failing test for deserialization**

Add to existing `deserialize_full_config` test and add a new dedicated test:

```rust
#[test]
fn auto_start_server_default_true() {
    let config: Config = toml::from_str("").expect("should parse empty TOML");
    assert!(config.auto_start_server);
}

#[test]
fn auto_start_server_explicit_false() {
    let config: Config = toml::from_str("auto_start_server = false").expect("should parse");
    assert!(!config.auto_start_server);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p quartermaster auto_start_server`
Expected: compilation error — `auto_start_server` field doesn't exist on `Config`

- [ ] **Step 3: Add the field to Config struct, default fn, Default impl, and env override**

Add default function:

```rust
fn default_auto_start_server() -> bool {
    true
}
```

Add field to `Config` struct (after `auto_drain_on_lifecycle`):

```rust
#[serde(default = "default_auto_start_server")]
pub auto_start_server: bool,
```

Add to `Default` impl:

```rust
auto_start_server: true,
```

Add to `apply_env_overrides`:

```rust
if let Ok(val) = std::env::var("QUMA_AUTO_START_SERVER") {
    if val.eq_ignore_ascii_case("true") {
        tracing::debug!(var = "QUMA_AUTO_START_SERVER", value = %val, "env var override applied");
        self.auto_start_server = true;
    } else if val.eq_ignore_ascii_case("false") {
        tracing::debug!(var = "QUMA_AUTO_START_SERVER", value = %val, "env var override applied");
        self.auto_start_server = false;
    }
}
```

Update the `deserialize_full_config` test's TOML to include `auto_start_server = false` and add the corresponding assert:

```rust
assert!(!config.auto_start_server);
```

Update the `deserialize_minimal_config` test to assert the default:

```rust
assert!(config.auto_start_server); // default: true
```

- [ ] **Step 4: Write and run env override test**

```rust
#[test]
fn auto_start_server_env_override() {
    temp_env::with_vars([("QUMA_AUTO_START_SERVER", Some("false"))], || {
        let mut config = Config::default();
        config.apply_env_overrides();
        assert!(!config.auto_start_server);
    });
}
```

- [ ] **Step 5: Run all config tests to verify everything passes**

Run: `cargo test -p quartermaster config`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat: add auto_start_server config option (default true)"
```

---

### Task 2: Auto-start container in `serve.rs`

**Files:**
- Modify: `src/cli/serve.rs:1-60`

**Interfaces:**
- Consumes: `Config.auto_start_server: bool`, `Config.server_container: Option<String>`, `PodmanClient::new(container) -> PodmanClient`, `PodmanClient::is_running() -> Result<bool>`, `PodmanClient::start() -> Result<()>`

- [ ] **Step 1: Add auto-start logic to `serve.rs`**

After the admin check (line 47) and before the `ForgeClient::new` call (line 49), add:

```rust
if config.auto_start_server {
    if let Some(ref container) = config.server_container {
        let podman = PodmanClient::new(container);
        match podman.is_running().await {
            Ok(true) => {
                tracing::info!(container, "server container already running");
            }
            Ok(false) => {
                tracing::info!(container, "auto-starting server container");
                if let Err(e) = podman.start().await {
                    tracing::warn!(container, error = %e, "failed to auto-start server container — web UI will start anyway");
                }
            }
            Err(e) => {
                tracing::warn!(container, error = %e, "failed to check container status — skipping auto-start");
            }
        }
    }
}
```

Add the import at the top of the file:

```rust
use crate::podman::PodmanClient;
```

- [ ] **Step 2: Run `cargo check` to verify compilation**

Run: `cargo check`
Expected: compiles without errors

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: all existing tests pass

- [ ] **Step 4: Manual smoke test**

Run `just serve` from the SPT directory with a configured container to verify the auto-start log message appears and the container starts. Then test with `auto_start_server = false` in `quartermaster.toml` to confirm the container is NOT started.

- [ ] **Step 5: Commit**

```bash
git add src/cli/serve.rs
git commit -m "feat: auto-start server container on quma serve"
```

---

### Task 3: Update `apply_env_overrides` doc comment

**Files:**
- Modify: `src/config.rs:281-291` (doc comment on `apply_env_overrides`)

**Interfaces:**
- None (documentation only)

- [ ] **Step 1: Add `QUMA_AUTO_START_SERVER` to the doc comment**

Add this line to the supported variables list:

```rust
/// - `QUMA_AUTO_START_SERVER` -> `auto_start_server`
```

- [ ] **Step 2: Commit**

```bash
git add src/config.rs
git commit -m "docs: document QUMA_AUTO_START_SERVER env var"
```
