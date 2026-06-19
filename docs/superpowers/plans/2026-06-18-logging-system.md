# Logging System Improvements — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Overhaul quartermaster's logging to add config-driven output, CLI verbosity flags, comprehensive coverage across all modules, and a web UI log viewer with live-tail support.

**Architecture:** A layered `tracing` subscriber with reload-capable filter + format layers (console, file, broadcast). The broadcast layer feeds an in-memory ring buffer and `tokio::sync::broadcast` channel. The web UI consumes these via JSON and SSE endpoints. Container logs are streamed from `podman logs --follow`.

**Tech Stack:** `tracing` + `tracing-subscriber` (reload, env-filter), `tracing-appender` (file/rotation), `rolling-file` (size-based rotation), `actix-web-lab` (SSE), `tokio::sync::broadcast`, Askama templates, vanilla JS + `EventSource`.

## Global Constraints

- All serde fields must have defaults — omitting `[logging]` entirely must preserve current behavior.
- Never log sensitive values (`forge_token`, `session_secret`) at any level.
- Per-layer filters (`.with_filter()`) must NOT be applied to layers wrapped in `reload::Layer`.
- Use `std::sync::RwLock` (not `tokio::sync::RwLock`) for the ring buffer — `Layer::on_event()` is synchronous.
- Use `Box<dyn Layer<S>>` type erasure for format switching (text vs json) via reload handles.
- Follow existing patterns: Askama `#[derive(Template)]`, `web::Data<AppState>`, `RequireAuth` middleware.

---

### Task 1: Add Dependencies and LoggingConfig Structs

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/config.rs`

**Interfaces:**
- Produces: `LoggingConfig`, `ConsoleLogConfig`, `FileLogConfig`, `WebLogConfig`, `LogFormat`, `RotationPolicy` — all `Clone + Debug + Serialize + Deserialize + PartialEq + Default`
- Produces: `LoggingConfig::is_default(&self) -> bool` — used by serde skip on the `Config` struct
- Produces: `Config.logging: LoggingConfig` field
- Produces: `Config::apply_env_overrides` extended with `QUMA_LOG_LEVEL`, `QUMA_LOG_FILE_PATH`, `QUMA_LOG_FILE_ENABLED`

- [ ] **Step 1: Write failing tests for LoggingConfig deserialization**

Add to `src/config.rs` in the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn logging_config_defaults() {
    let config: Config = toml::from_str("").expect("empty config");
    assert_eq!(config.logging, LoggingConfig::default());
    assert_eq!(config.logging.level, "info");
    assert!(config.logging.console.enabled);
    assert_eq!(config.logging.console.format, LogFormat::Text);
    assert!(!config.logging.file.enabled);
    assert_eq!(config.logging.file.path, "quartermaster.log");
    assert_eq!(config.logging.file.format, LogFormat::Json);
    assert_eq!(config.logging.file.rotation, RotationPolicy::None);
    assert_eq!(config.logging.file.max_size_mb, 10);
    assert_eq!(config.logging.file.max_files, 5);
    assert_eq!(config.logging.web.buffer_size, 1000);
}

#[test]
fn logging_config_full_deserialization() {
    let toml_str = r#"
[logging]
level = "debug"

[logging.console]
enabled = false
format = "json"

[logging.file]
enabled = true
path = "/var/log/quma.log"
format = "text"
rotation = "size"
max_size_mb = 50
max_files = 10

[logging.web]
buffer_size = 5000
"#;
    let config: Config = toml::from_str(toml_str).expect("should parse");
    assert_eq!(config.logging.level, "debug");
    assert!(!config.logging.console.enabled);
    assert_eq!(config.logging.console.format, LogFormat::Json);
    assert!(config.logging.file.enabled);
    assert_eq!(config.logging.file.path, "/var/log/quma.log");
    assert_eq!(config.logging.file.format, LogFormat::Text);
    assert_eq!(config.logging.file.rotation, RotationPolicy::Size);
    assert_eq!(config.logging.file.max_size_mb, 50);
    assert_eq!(config.logging.file.max_files, 10);
    assert_eq!(config.logging.web.buffer_size, 5000);
}

#[test]
fn logging_config_skip_serializing_when_default() {
    let config = Config::default();
    let serialized = toml::to_string_pretty(&config).unwrap();
    assert!(!serialized.contains("[logging]"), "default logging config should not be serialized");
}

#[test]
fn logging_config_serialized_when_non_default() {
    let mut config = Config::default();
    config.logging.level = "debug".to_string();
    let serialized = toml::to_string_pretty(&config).unwrap();
    assert!(serialized.contains("[logging]"), "non-default logging config should be serialized");
}

#[test]
fn logging_env_var_overrides() {
    temp_env::with_vars(
        [
            ("QUMA_LOG_LEVEL", Some("trace")),
            ("QUMA_LOG_FILE_PATH", Some("/tmp/test.log")),
            ("QUMA_LOG_FILE_ENABLED", Some("true")),
        ],
        || {
            let mut config = Config::default();
            config.apply_env_overrides();
            assert_eq!(config.logging.level, "trace");
            assert_eq!(config.logging.file.path, "/tmp/test.log");
            assert!(config.logging.file.enabled);
        },
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib config::tests`
Expected: compilation errors — `LoggingConfig`, `LogFormat`, `RotationPolicy` don't exist yet.

- [ ] **Step 3: Add dependencies to Cargo.toml**

Add to `[dependencies]`:

```toml
tracing-appender = "0.2"
rolling-file = "0.2"
actix-web-lab = "0.24"
```

- [ ] **Step 4: Implement LoggingConfig structs and integrate into Config**

In `src/config.rs`, add before the `Config` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RotationPolicy {
    None,
    Size,
    Daily,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_console_enabled() -> bool {
    true
}

fn default_log_format_text() -> LogFormat {
    LogFormat::Text
}

fn default_log_format_json() -> LogFormat {
    LogFormat::Json
}

fn default_file_path() -> String {
    "quartermaster.log".to_string()
}

fn default_rotation() -> RotationPolicy {
    RotationPolicy::None
}

fn default_max_size_mb() -> u64 {
    10
}

fn default_max_files() -> usize {
    5
}

fn default_buffer_size() -> usize {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsoleLogConfig {
    #[serde(default = "default_console_enabled")]
    pub enabled: bool,
    #[serde(default = "default_log_format_text")]
    pub format: LogFormat,
}

impl Default for ConsoleLogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format: LogFormat::Text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileLogConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_file_path")]
    pub path: String,
    #[serde(default = "default_log_format_json")]
    pub format: LogFormat,
    #[serde(default = "default_rotation")]
    pub rotation: RotationPolicy,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u64,
    #[serde(default = "default_max_files")]
    pub max_files: usize,
}

impl Default for FileLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "quartermaster.log".to_string(),
            format: LogFormat::Json,
            rotation: RotationPolicy::None,
            max_size_mb: 10,
            max_files: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebLogConfig {
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
}

impl Default for WebLogConfig {
    fn default() -> Self {
        Self { buffer_size: 1000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub console: ConsoleLogConfig,
    #[serde(default)]
    pub file: FileLogConfig,
    #[serde(default)]
    pub web: WebLogConfig,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            console: ConsoleLogConfig::default(),
            file: FileLogConfig::default(),
            web: WebLogConfig::default(),
        }
    }
}

impl LoggingConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
```

Add the `logging` field to the `Config` struct:

```rust
#[serde(default)]
#[serde(skip_serializing_if = "LoggingConfig::is_default")]
pub logging: LoggingConfig,
```

Add to `Config::default()`:

```rust
logging: LoggingConfig::default(),
```

Extend `apply_env_overrides` with:

```rust
if let Ok(val) = std::env::var("QUMA_LOG_LEVEL") {
    self.logging.level = val;
}
if let Ok(val) = std::env::var("QUMA_LOG_FILE_PATH") {
    self.logging.file.path = val;
}
if let Ok(val) = std::env::var("QUMA_LOG_FILE_ENABLED") {
    if val.eq_ignore_ascii_case("true") {
        self.logging.file.enabled = true;
    } else if val.eq_ignore_ascii_case("false") {
        self.logging.file.enabled = false;
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib config::tests`
Expected: all tests pass, including the 5 new ones.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/config.rs
git commit -m "feat: add LoggingConfig structs with defaults, env var overrides, and skip_serializing_if"
```

---

### Task 2: CLI Verbosity Flags and Log Level Resolution

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/logging.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `LoggingConfig` from Task 1, `Cli` struct from `src/cli/mod.rs`
- Produces: `Cli.verbose: u8` (counting `-v` flags)
- Produces: `Cli.log_level: Option<String>` (`--log-level` flag)
- Produces: `resolve_log_filter(config: &LoggingConfig, verbose: u8, log_level: Option<&str>) -> String` — returns the `EnvFilter` string after applying the priority chain
- Produces: `mod logging` in `main.rs`

- [ ] **Step 1: Write failing tests for log level resolution**

Create `src/logging.rs`:

```rust
use crate::config::LoggingConfig;

/// Resolve the effective EnvFilter string from the priority chain:
/// 1. --log-level CLI flag (highest)
/// 2. -v / -vv CLI flags
/// 3. QUMA_LOG_LEVEL env var (already applied to config by apply_env_overrides)
/// 4. RUST_LOG env var
/// 5. config.logging.level
/// 6. hardcoded default: "info,quartermaster=debug"
pub fn resolve_log_filter(
    config: &LoggingConfig,
    verbose: u8,
    log_level: Option<&str>,
) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter() {
        let config = LoggingConfig::default();
        let result = resolve_log_filter(&config, 0, None);
        assert_eq!(result, "info,quartermaster=debug");
    }

    #[test]
    fn config_level_overrides_default() {
        let mut config = LoggingConfig::default();
        config.level = "warn".to_string();
        let result = resolve_log_filter(&config, 0, None);
        assert_eq!(result, "warn,quartermaster=debug");
    }

    #[test]
    fn verbose_flag_sets_crate_debug() {
        let config = LoggingConfig::default();
        let result = resolve_log_filter(&config, 1, None);
        assert_eq!(result, "info,quartermaster=debug");
    }

    #[test]
    fn double_verbose_sets_crate_trace() {
        let config = LoggingConfig::default();
        let result = resolve_log_filter(&config, 2, None);
        assert_eq!(result, "info,quartermaster=trace");
    }

    #[test]
    fn log_level_flag_overrides_everything() {
        let mut config = LoggingConfig::default();
        config.level = "warn".to_string();
        let result = resolve_log_filter(&config, 2, Some("error"));
        assert_eq!(result, "error");
    }

    #[test]
    fn rust_log_env_overrides_config() {
        temp_env::with_vars(
            [("RUST_LOG", Some("debug,hyper=warn")), ("QUMA_LOG_LEVEL", None)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "debug,hyper=warn");
            },
        );
    }

    #[test]
    fn verbose_overrides_rust_log() {
        temp_env::with_vars(
            [("RUST_LOG", Some("warn")), ("QUMA_LOG_LEVEL", None)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 1, None);
                assert_eq!(result, "warn,quartermaster=debug");
            },
        );
    }

    #[test]
    fn log_level_flag_overrides_rust_log() {
        temp_env::with_vars(
            [("RUST_LOG", Some("debug,hyper=warn")), ("QUMA_LOG_LEVEL", None)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 0, Some("trace"));
                assert_eq!(result, "trace");
            },
        );
    }

    #[test]
    fn quma_log_level_overrides_rust_log() {
        temp_env::with_vars(
            [("RUST_LOG", Some("debug,hyper=warn")), ("QUMA_LOG_LEVEL", Some("error"))],
            || {
                let mut config = LoggingConfig::default();
                config.level = "error".to_string(); // simulates apply_env_overrides
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "error,quartermaster=debug");
            },
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib logging::tests`
Expected: all tests fail with `not yet implemented` panic.

- [ ] **Step 3: Implement resolve_log_filter**

Replace the `todo!()` in `src/logging.rs`:

```rust
pub fn resolve_log_filter(
    config: &LoggingConfig,
    verbose: u8,
    log_level: Option<&str>,
) -> String {
    // Priority 1: --log-level flag (global, overrides everything)
    if let Some(level) = log_level {
        return level.to_string();
    }

    // Determine base filter from priority chain
    let base = if std::env::var("QUMA_LOG_LEVEL").is_ok() {
        // Priority 3: QUMA_LOG_LEVEL (already merged into config.level via apply_env_overrides)
        // Takes precedence over RUST_LOG
        format!("{},quartermaster=debug", config.level)
    } else if let Ok(rust_log) = std::env::var("RUST_LOG") {
        // Priority 4: RUST_LOG (full filter syntax, legacy escape hatch)
        rust_log
    } else if config.level != "info" {
        // Priority 5: config file level
        format!("{},quartermaster=debug", config.level)
    } else {
        // Priority 6: hardcoded default
        "info,quartermaster=debug".to_string()
    };

    // Priority 2: -v / -vv (crate-scoped override)
    match verbose {
        0 => base,
        1 => {
            let without_qm = strip_crate_directive(&base, "quartermaster");
            format!("{without_qm},quartermaster=debug")
        }
        _ => {
            let without_qm = strip_crate_directive(&base, "quartermaster");
            format!("{without_qm},quartermaster=trace")
        }
    }
}

fn strip_crate_directive(filter: &str, crate_name: &str) -> String {
    filter
        .split(',')
        .filter(|part| {
            !part
                .split('=')
                .next()
                .is_some_and(|key| key.trim() == crate_name)
        })
        .collect::<Vec<_>>()
        .join(",")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib logging::tests`
Expected: all 8 tests pass.

- [ ] **Step 5: Add CLI flags to the Cli struct**

In `src/cli/mod.rs`, add to the `Cli` struct before the `command` field:

```rust
/// Increase verbosity (-v for debug, -vv for trace)
#[arg(short, long, action = clap::ArgAction::Count, global = true)]
pub verbose: u8,

/// Set log level explicitly (trace, debug, info, warn, error)
#[arg(long, global = true)]
pub log_level: Option<String>,
```

- [ ] **Step 6: Register the logging module in main.rs**

Add `mod logging;` to the module declarations in `src/main.rs`.

- [ ] **Step 7: Run full test suite**

Run: `cargo test`
Expected: all tests pass, no regressions.

- [ ] **Step 8: Commit**

```bash
git add src/logging.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add CLI verbosity flags and log level resolution with priority chain"
```

---

### Task 3: Broadcast Layer, Ring Buffer, and Subscriber Initialization

**Files:**
- Modify: `src/logging.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `resolve_log_filter` from Task 2, `LoggingConfig` / `FileLogConfig` / `LogFormat` / `RotationPolicy` from Task 1
- Produces: `LogEntry` struct — `Clone + Serialize + Deserialize + Debug`, fields: `timestamp: DateTime<Utc>`, `level: String`, `target: String`, `message: String`, `fields: HashMap<String, serde_json::Value>`
- Produces: `LogBroadcast` struct — holds `broadcast::Sender<LogEntry>`, `Arc<std::sync::RwLock<VecDeque<LogEntry>>>`, `buffer_size: usize`
- Produces: `LogBroadcast::new(buffer_size: usize) -> Self`
- Produces: `LogBroadcast::subscribe(&self) -> broadcast::Receiver<LogEntry>`
- Produces: `LogBroadcast::recent(&self, limit: usize) -> Vec<LogEntry>`
- Produces: `BroadcastLayer` — implements `tracing_subscriber::Layer<S>`
- Produces: `init_subscriber(log_broadcast: &LogBroadcast) -> ReloadHandles`
- Produces: `ReloadHandles::reconfigure(&self, config: &LoggingConfig, filter_str: &str, spt_dir: Option<&Path>)`

- [ ] **Step 1: Write failing tests for LogBroadcast**

Add to `src/logging.rs`:

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: HashMap<String, serde_json::Value>,
}

pub struct LogBroadcast {
    sender: broadcast::Sender<LogEntry>,
    buffer: Arc<RwLock<VecDeque<LogEntry>>>,
    buffer_size: usize,
}

// ... tests follow
```

Add in the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn log_broadcast_stores_entries_in_ring_buffer() {
    let lb = LogBroadcast::new(3);
    for i in 0..5 {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: format!("msg {i}"),
            fields: HashMap::new(),
        };
        lb.send(entry);
    }
    let recent = lb.recent(10);
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].message, "msg 2");
    assert_eq!(recent[2].message, "msg 4");
}

#[test]
fn log_broadcast_recent_respects_limit() {
    let lb = LogBroadcast::new(100);
    for i in 0..10 {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: format!("msg {i}"),
            fields: HashMap::new(),
        };
        lb.send(entry);
    }
    let recent = lb.recent(3);
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].message, "msg 7");
}

#[test]
fn log_broadcast_subscribe_receives_new_entries() {
    let lb = LogBroadcast::new(100);
    let mut rx = lb.subscribe();

    let entry = LogEntry {
        timestamp: Utc::now(),
        level: "ERROR".to_string(),
        target: "test".to_string(),
        message: "hello".to_string(),
        fields: HashMap::new(),
    };
    lb.send(entry);

    let received = rx.try_recv().unwrap();
    assert_eq!(received.message, "hello");
    assert_eq!(received.level, "ERROR");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib logging::tests`
Expected: compilation error — `LogBroadcast::new`, `send`, `recent`, `subscribe` not implemented.

- [ ] **Step 3: Implement LogBroadcast**

In `src/logging.rs`:

```rust
impl LogBroadcast {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size.max(16));
        Self {
            sender,
            buffer: Arc::new(RwLock::new(VecDeque::with_capacity(buffer_size))),
            buffer_size,
        }
    }

    pub fn send(&self, entry: LogEntry) {
        {
            let mut buf = self.buffer.write().unwrap();
            if buf.len() >= self.buffer_size {
                buf.pop_front();
            }
            buf.push_back(entry.clone());
        }
        let _ = self.sender.send(entry);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.sender.subscribe()
    }

    pub fn recent(&self, limit: usize) -> Vec<LogEntry> {
        let buf = self.buffer.read().unwrap();
        let skip = buf.len().saturating_sub(limit);
        buf.iter().skip(skip).cloned().collect()
    }

    pub fn sender(&self) -> broadcast::Sender<LogEntry> {
        self.sender.clone()
    }

    pub fn buffer(&self) -> Arc<RwLock<VecDeque<LogEntry>>> {
        Arc::clone(&self.buffer)
    }
}
```

- [ ] **Step 4: Run LogBroadcast tests**

Run: `cargo test --lib logging::tests`
Expected: all LogBroadcast tests pass.

- [ ] **Step 5: Implement BroadcastLayer and field visitor**

In `src/logging.rs`:

```rust
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

struct FieldVisitor {
    fields: HashMap<String, serde_json::Value>,
    message: String,
}

impl FieldVisitor {
    fn new() -> Self {
        Self {
            fields: HashMap::new(),
            message: String::new(),
        }
    }
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(format!("{value:?}")),
            );
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Bool(value),
        );
    }
}

pub struct BroadcastLayer {
    broadcast: Arc<LogBroadcast>,
}

impl BroadcastLayer {
    pub fn new(broadcast: Arc<LogBroadcast>) -> Self {
        Self { broadcast }
    }
}

impl<S: Subscriber> Layer<S> for BroadcastLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = FieldVisitor::new();
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: Utc::now(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
            fields: visitor.fields,
        };

        self.broadcast.send(entry);
    }
}
```

- [ ] **Step 6: Implement init_subscriber and ReloadHandles**

In `src/logging.rs`:

```rust
use std::path::Path;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload;
use tracing_subscriber::EnvFilter;

use crate::config::{FileLogConfig, LogFormat, LoggingConfig, RotationPolicy};

pub struct ReloadHandles {
    filter_handle: reload::Handle<EnvFilter, tracing_subscriber::Registry>,
    console_handle: reload::Handle<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>, tracing_subscriber::Registry>,
    file_handle: reload::Handle<Option<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>>, tracing_subscriber::Registry>,
}

pub fn init_subscriber(log_broadcast: &Arc<LogBroadcast>) -> ReloadHandles {
    let filter = EnvFilter::new("info,quartermaster=debug");
    let (filter_layer, filter_handle) = reload::Layer::new(filter);

    let console_layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> =
        Box::new(fmt::layer().with_writer(std::io::stderr));
    let (console_reload, console_handle) = reload::Layer::new(console_layer);

    let file_layer: Option<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> = None;
    let (file_reload, file_handle) = reload::Layer::new(file_layer);

    let broadcast_layer = BroadcastLayer::new(Arc::clone(log_broadcast));

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(console_reload)
        .with(file_reload)
        .with(broadcast_layer)
        .init();

    ReloadHandles {
        filter_handle,
        console_handle,
        file_handle,
    }
}

impl ReloadHandles {
    pub fn reconfigure(
        &self,
        config: &LoggingConfig,
        filter_str: &str,
        spt_dir: Option<&Path>,
    ) {
        // Update filter
        if let Ok(new_filter) = EnvFilter::try_new(filter_str) {
            let _ = self.filter_handle.reload(new_filter);
        }

        // Update console layer
        if config.console.enabled {
            let layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> =
                match config.console.format {
                    LogFormat::Json => Box::new(
                        fmt::layer()
                            .json()
                            .with_writer(std::io::stderr),
                    ),
                    LogFormat::Text => Box::new(
                        fmt::layer()
                            .with_writer(std::io::stderr),
                    ),
                };
            let _ = self.console_handle.reload(layer);
        } else {
            let layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> =
                Box::new(tracing_subscriber::layer::Identity::new());
            let _ = self.console_handle.reload(layer);
        }

        // Update file layer
        if config.file.enabled {
            let file_path = if Path::new(&config.file.path).is_absolute() {
                std::path::PathBuf::from(&config.file.path)
            } else {
                spt_dir
                    .map(|d| d.join(&config.file.path))
                    .unwrap_or_else(|| std::path::PathBuf::from(&config.file.path))
            };

            let file_layer = build_file_layer(&config.file, &file_path);
            let _ = self.file_handle.reload(Some(file_layer));
        } else {
            let _ = self.file_handle.reload(None);
        }
    }
}

fn build_file_layer(
    config: &FileLogConfig,
    path: &Path,
) -> Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("quartermaster.log");

    match config.rotation {
        RotationPolicy::Daily => {
            let appender = tracing_appender::rolling::daily(dir, filename);
            let (non_blocking, _guard) = tracing_appender::non_blocking(appender);
            // NOTE: _guard is leaked intentionally — it must live for the program's lifetime.
            // Box::leak is acceptable here because the file layer is created at most once.
            let guard = Box::leak(Box::new(_guard));
            let _ = guard; // suppress unused warning
            match config.format {
                LogFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                LogFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
            }
        }
        RotationPolicy::Size | RotationPolicy::None => {
            // For Size and None, use a simple file writer
            // rolling-file crate handles size-based rotation
            let appender = if matches!(config.rotation, RotationPolicy::Size) {
                let max_bytes = config.max_size_mb * 1024 * 1024;
                rolling_file::BasicRollingFileAppender::new(
                    path,
                    rolling_file::RollingConditionBasic::new().max_size(max_bytes),
                    config.max_files,
                )
                .expect("failed to create rolling file appender")
            } else {
                // No rotation — just a single file
                rolling_file::BasicRollingFileAppender::new(
                    path,
                    rolling_file::RollingConditionBasic::new().max_size(u64::MAX),
                    1,
                )
                .expect("failed to create file appender")
            };
            let (non_blocking, _guard) = tracing_appender::non_blocking(appender);
            let guard = Box::leak(Box::new(_guard));
            let _ = guard;
            match config.format {
                LogFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                LogFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
            }
        }
    }
}
```

Note: The exact type signatures for the `reload::Handle` generics may need adjustment during implementation — the tracing-subscriber API is generic-heavy. The implementer should follow compiler guidance to get the concrete types right. The key pattern is: wrap each layer in `reload::Layer`, use `Box<dyn Layer<S> + Send + Sync>` for type erasure on format-variable layers, and use `Option<Box<...>>` for the conditionally-enabled file layer.

- [ ] **Step 7: Wire up subscriber init in main.rs**

Replace the existing subscriber setup in `src/main.rs` `main()` function. The new flow:

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // Early bootstrap: init subscriber with defaults + broadcast layer
    let log_broadcast = std::sync::Arc::new(logging::LogBroadcast::new(1000));
    let reload_handles = logging::init_subscriber(&log_broadcast);

    let cli = Cli::parse();

    // ... (existing match arms stay the same, but serve arm changes — see below)
```

For commands that load config (most arms call `resolve_context` or load config directly), insert reconfiguration after config loads. The cleanest approach: add a helper in main that reconfigures logging once config is available.

For the `Serve` command arm specifically, pass `log_broadcast` through to the web server (this is wired up in a later task).

For all other commands, reconfigure after `resolve_context`:

```rust
// Example for a command that uses resolve_context:
Command::Install { mod_ref, version, force } => {
    let ctx = cli::common::resolve_context(&cli)?;
    let filter = logging::resolve_log_filter(
        &ctx.config.logging,
        cli.verbose,
        cli.log_level.as_deref(),
    );
    reload_handles.reconfigure(&ctx.config.logging, &filter, Some(&ctx.spt_dir));
    cli::install::run(mod_ref, version.as_deref(), *force, &ctx).await
}
```

To avoid repeating this in every arm, extract a helper:

```rust
fn reconfigure_logging(
    handles: &logging::ReloadHandles,
    config: &Config,
    cli: &Cli,
    spt_dir: Option<&std::path::Path>,
) {
    let filter = logging::resolve_log_filter(
        &config.logging,
        cli.verbose,
        cli.log_level.as_deref(),
    );
    handles.reconfigure(&config.logging, &filter, spt_dir);
}
```

Then call it after each `resolve_context` or config load. For the `Serve` arm, reconfigure before calling `start_server` and pass `log_broadcast`.

- [ ] **Step 8: Verify compilation and existing tests**

Run: `cargo test`
Expected: all tests pass. The subscriber is initialized earlier now, but behavior should be identical with default config.

- [ ] **Step 9: Commit**

```bash
git add src/logging.rs src/main.rs
git commit -m "feat: broadcast layer, ring buffer, and reload-capable subscriber initialization"
```

---

### Task 4: Add Logging Coverage to Core Modules

**Files:**
- Modify: `src/podman.rs`
- Modify: `src/ops.rs`
- Modify: `src/config.rs`
- Modify: `src/cli/common.rs`

**Interfaces:**
- Consumes: existing module code
- Produces: no new public interfaces — this task adds `tracing::debug!`, `trace!`, `warn!`, `error!`, `info!` calls to existing functions

- [ ] **Step 1: Add logging to podman.rs**

In `PodmanClient::is_running`, add before the `tokio::process::Command` call:

```rust
tracing::debug!(container = %self.container, "checking container status");
```

After the output is captured, add:

```rust
tracing::trace!(
    container = %self.container,
    stdout = %String::from_utf8_lossy(&output.stdout),
    stderr = %String::from_utf8_lossy(&output.stderr),
    status = %output.status,
    "podman inspect output"
);
```

On the `not found` error path:

```rust
tracing::warn!(container = %self.container, "container not found");
```

On the generic failure path:

```rust
tracing::error!(container = %self.container, stderr = %stderr.trim(), "podman inspect failed");
```

Apply similar patterns to `start()`, `stop()`, `logs()`, and `detect_spt_containers()`:
- `debug!` before each command invocation with command + args
- `trace!` on raw stdout/stderr
- `error!` on failure paths (before `bail!`)

- [ ] **Step 2: Add logging to ops.rs**

In `install_mod_from_archive`:

```rust
tracing::info!(name, forge_mod_id, version, "installing mod from archive");
// ... after extract + DB insert:
tracing::debug!(db_id, file_count = extracted.len(), "mod installed, files recorded");
```

In `update_mod_from_archive`:

```rust
tracing::info!(mod_db_id, version_str, "updating mod from archive");
tracing::debug!(old_file_count = old_paths.len(), new_file_count = extracted.len(), "replacing mod files");
```

In `remove_mod_by_id`:

```rust
tracing::info!(mod_db_id, "removing mod");
tracing::debug!(file_count = paths.len(), "deleting mod files");
```

In `scan_and_record_runtime_files`:

```rust
tracing::debug!(mod_db_id, dir_count = mod_dirs.len(), "scanning for runtime files");
```

In `scan_runtime_recursive`, add per-file trace:

```rust
tracing::trace!(path = %rel_str, "recording runtime file");
```

- [ ] **Step 3: Add logging to config.rs**

In `Config::load`:

```rust
tracing::debug!(path = %path.display(), "loading config file");
```

In `Config::load_with_env` after `apply_env_overrides`:

```rust
tracing::debug!("applied environment variable overrides to config");
```

In `apply_env_overrides`, for each env var that is set:

```rust
// Example for QUMA_SPT_DIR:
tracing::debug!(var = "QUMA_SPT_DIR", value = %val, "env var override applied");
```

In `Config::resolve_path`, log which path was chosen:

```rust
// At the start of the function — but resolve_path is a static method called before
// logging is initialized. Add logging only to the parts that run after init.
// Actually, this is called by resolve_context, which runs after subscriber init.
// So logging here is fine.
```

Add `trace!` that logs the full parsed config with redacted sensitive fields:

```rust
// At the end of Config::load_with_env:
tracing::trace!(
    spt_dir = ?config.spt_dir,
    forge_token = "<redacted>",
    queue_changes = config.queue_changes,
    auto_drain_on_lifecycle = config.auto_drain_on_lifecycle,
    session_secret = "<redacted>",
    web_bind = %config.web_bind,
    web_port = config.web_port,
    "loaded config"
);
```

- [ ] **Step 4: Add logging to cli/common.rs**

In `resolve_context`:

```rust
tracing::debug!(spt_dir = %spt_dir.display(), "resolved SPT directory");
tracing::debug!(config_path = %config_path.display(), "resolved config path");
```

- [ ] **Step 5: Verify no sensitive values are logged**

Run: `rg 'forge_token|session_secret' src/ --glob '!config.rs' -l`
Expected: no files log these values. In `config.rs`, verify they're only logged as `"<redacted>"`.

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/podman.rs src/ops.rs src/config.rs src/cli/common.rs
git commit -m "feat: add structured logging to podman, ops, config, and CLI modules"
```

---

### Task 5: Web UI — AppState Integration and Log API Endpoints

**Files:**
- Modify: `src/web/state.rs`
- Create: `src/web/handlers/logs.rs`
- Modify: `src/web/handlers/mod.rs`
- Modify: `src/web/mod.rs`
- Modify: `src/cli/serve.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `LogBroadcast`, `LogEntry` from Task 3, `PodmanClient` from `src/podman.rs`, `AppState` from `src/web/state.rs`
- Produces: `AppState.log_broadcast: Arc<LogBroadcast>` field
- Produces: `GET /api/logs/app?limit=N` — returns `Vec<LogEntry>` as JSON
- Produces: `GET /api/logs/app/stream` — SSE stream of `LogEntry` JSON
- Produces: `GET /api/logs/server` — returns recent container log lines as JSON array of strings
- Produces: `GET /api/logs/server/stream` — SSE stream of container log lines
- Produces: `start_server()` now takes `log_broadcast: Arc<LogBroadcast>` parameter

- [ ] **Step 1: Add log_broadcast to AppState**

In `src/web/state.rs`, add:

```rust
use crate::logging::LogBroadcast;
```

Add field to `AppState`:

```rust
pub log_broadcast: std::sync::Arc<LogBroadcast>,
```

- [ ] **Step 2: Update start_server signature and AppState construction**

In `src/web/mod.rs`, update the `start_server` function signature to accept `log_broadcast`:

```rust
pub async fn start_server(
    config: Config,
    db: Database,
    forge: ForgeClient,
    spt_dir: std::path::PathBuf,
    spt_info: SptInfo,
    log_broadcast: std::sync::Arc<crate::logging::LogBroadcast>,
) -> Result<()> {
```

Update the `AppState` construction inside `start_server`:

```rust
let app_state = web::Data::new(AppState {
    db,
    forge,
    config: config.clone(),
    spt_dir,
    spt_info,
    tasks: crate::web::tasks::TaskTracker::new(),
    log_broadcast,
});
```

- [ ] **Step 3: Update cli/serve.rs to pass log_broadcast**

In `src/cli/serve.rs`, update the function signature to accept log_broadcast and pass it through:

```rust
pub async fn run(
    bind: Option<&str>,
    port: Option<u16>,
    cli: &super::Cli,
    log_broadcast: std::sync::Arc<crate::logging::LogBroadcast>,
) -> Result<()> {
    // ... existing code ...
    crate::web::start_server(config, db, forge, spt_dir, spt_info, log_broadcast).await
}
```

- [ ] **Step 4: Update main.rs Serve arm**

In `src/main.rs`, update the `Serve` arm to pass `log_broadcast`:

```rust
Command::Serve { bind, port } => {
    cli::serve::run(bind.as_deref(), *port, &cli, Arc::clone(&log_broadcast)).await
}
```

Add `use std::sync::Arc;` if not already present.

Also reconfigure logging in the `Serve` arm (after config loads but before `start_server`), similarly to other commands. The `serve.rs` `run` function should call `reconfigure_logging` — pass the `ReloadHandles` through or expose a function.

The simplest approach: move the `reconfigure_logging` call into `serve.rs` by accepting the handles as a parameter, or reconfigure in `main.rs` before calling `serve::run`. Since the Serve arm loads config inside `serve::run`, the cleanest solution is to have `main.rs` also accept `ReloadHandles` as something that gets passed, or restructure slightly. The implementer should choose the approach that minimizes signature bloat — a pragmatic option is to reconfigure in `serve.rs` directly using the public `logging::` functions.

- [ ] **Step 5: Create log handler file with JSON endpoint**

Create `src/web/handlers/logs.rs`:

```rust
use actix_web::web::{self, Data, Html, Query};
use actix_web::HttpResponse;
use actix_session::Session;
use serde::Deserialize;

use crate::web::auth::require_auth;
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct LogQuery {
    limit: Option<usize>,
}

pub async fn app_logs_json(
    state: Data<AppState>,
    session: Session,
    query: Query<LogQuery>,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;
    let limit = query.limit.unwrap_or(100);
    let entries = state.log_broadcast.recent(limit);
    Ok(HttpResponse::Ok().json(entries))
}
```

- [ ] **Step 6: Add SSE stream endpoint for app logs**

In `src/web/handlers/logs.rs`, add:

```rust
use std::convert::Infallible;
use std::time::Duration;

use actix_web_lab::sse;
use tokio::sync::broadcast::error::RecvError;

pub async fn app_logs_stream(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    require_auth(&session)?;
    let mut rx = state.log_broadcast.subscribe();

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(entry) => {
                    if let Ok(json) = serde_json::to_string(&entry) {
                        yield Ok(sse::Event::Data(sse::Data::new(json)));
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    yield Ok(sse::Event::Comment(format!("lagged:{n}").into()));
                }
                Err(RecvError::Closed) => break,
            }
        }
    };

    Ok(sse::Sse::from_stream(stream).with_keep_alive(Duration::from_secs(15)))
}
```

Add `async-stream` to `Cargo.toml`:

```toml
async-stream = "0.3"
```

- [ ] **Step 7: Add server log endpoints**

In `src/web/handlers/logs.rs`, add:

```rust
use crate::podman::PodmanClient;

pub async fn server_logs_json(
    state: Data<AppState>,
    session: Session,
    query: Query<LogQuery>,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;
    let container = state.config.server_container.as_deref().ok_or(WebError::NotFound)?;
    let tail = query.limit.unwrap_or(100);

    let output = tokio::process::Command::new("podman")
        .args(["logs", "--tail", &tail.to_string(), container])
        .output()
        .await
        .map_err(|e| WebError::Internal(anyhow::anyhow!("podman logs failed: {e}")))?;

    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();
    Ok(HttpResponse::Ok().json(lines))
}

pub async fn server_logs_stream(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    require_auth(&session)?;
    let container = state
        .config
        .server_container
        .clone()
        .ok_or(WebError::NotFound)?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<sse::Event, Infallible>>(64);

    tokio::spawn(async move {
        let mut child = match tokio::process::Command::new("podman")
            .args(["logs", "--follow", "--tail", "0", &container])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(Ok(sse::Event::Data(
                        sse::Data::new(format!("error: {e}"))
                            .event("error"),
                    )))
                    .await;
                return;
            }
        };

        if let Some(stdout) = child.stdout.take() {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(Ok(sse::Event::Data(sse::Data::new(line)))).await.is_err() {
                    break;
                }
            }
        }

        let _ = child.kill().await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(sse::Sse::from_stream(stream).with_keep_alive(Duration::from_secs(15)))
}
```

Add `tokio-stream` and `async-stream` to `Cargo.toml` if not already present:

```toml
tokio-stream = "0.1"
async-stream = "0.3"
```

- [ ] **Step 8: Register handler module and routes**

In `src/web/handlers/mod.rs`, add:

```rust
pub mod logs;
```

In `src/web/mod.rs`, add the log routes inside the `/api` scope:

```rust
.route("/logs/app", web::get().to(handlers::logs::app_logs_json))
.route("/logs/app/stream", web::get().to(handlers::logs::app_logs_stream))
.route("/logs/server", web::get().to(handlers::logs::server_logs_json))
.route("/logs/server/stream", web::get().to(handlers::logs::server_logs_stream))
```

- [ ] **Step 9: Verify compilation**

Run: `cargo build`
Expected: compiles successfully.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock src/web/state.rs src/web/handlers/logs.rs src/web/handlers/mod.rs src/web/mod.rs src/cli/serve.rs src/main.rs
git commit -m "feat: add log API endpoints (JSON + SSE) for app and server logs"
```

---

### Task 6: Web UI — Log Viewer Page Template and Frontend

**Files:**
- Create: `templates/logs.html`
- Modify: `src/web/handlers/logs.rs`
- Modify: `src/web/mod.rs`
- Modify: `templates/partials/nav.html`

**Interfaces:**
- Consumes: `/api/logs/app?limit=N`, `/api/logs/app/stream`, `/api/logs/server`, `/api/logs/server/stream` from Task 5
- Produces: `GET /logs` — renders the log viewer page

- [ ] **Step 1: Create the log viewer page template**

Create `templates/logs.html`:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% block title %}Logs — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("logs", user, csrf_token) %}{% endcall %}{% endblock %}
{% block content %}
<h1>Logs</h1>

<div class="tab-bar">
    <button class="tab-btn active" data-tab="app">Application Logs</button>
    <button class="tab-btn" data-tab="server">Server Logs</button>
</div>

<div id="tab-app" class="tab-panel">
    <div class="log-controls">
        <div class="log-controls-left">
            <button id="app-follow-btn" class="btn btn-sm btn-outline">Follow</button>
            <button id="app-refresh-btn" class="btn btn-sm btn-outline">Refresh</button>
            <button id="app-clear-btn" class="btn btn-sm btn-outline">Clear</button>
        </div>
        <div class="log-controls-right">
            <span class="log-filter-group">
                <button class="log-level-btn active" data-level="error">Error</button>
                <button class="log-level-btn active" data-level="warn">Warn</button>
                <button class="log-level-btn active" data-level="info">Info</button>
                <button class="log-level-btn" data-level="debug">Debug</button>
                <button class="log-level-btn" data-level="trace">Trace</button>
            </span>
            <input type="text" id="app-search" class="log-search" placeholder="Search...">
        </div>
    </div>
    <div id="app-log-container" class="log-container">
        <div class="empty-state"><p>Loading...</p></div>
    </div>
</div>

<div id="tab-server" class="tab-panel" style="display:none">
    <div class="log-controls">
        <div class="log-controls-left">
            <button id="server-follow-btn" class="btn btn-sm btn-outline">Follow</button>
            <button id="server-refresh-btn" class="btn btn-sm btn-outline">Refresh</button>
            <button id="server-clear-btn" class="btn btn-sm btn-outline">Clear</button>
        </div>
        <div class="log-controls-right">
            <input type="text" id="server-search" class="log-search" placeholder="Search...">
        </div>
    </div>
    <div id="server-status" style="display:none" class="text-muted text-sm"></div>
    <div id="server-log-container" class="log-container">
        <div class="empty-state"><p>Loading...</p></div>
    </div>
</div>

<script>
(function() {
    // Tab switching
    document.querySelectorAll('.tab-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
            document.querySelectorAll('.tab-panel').forEach(p => p.style.display = 'none');
            btn.classList.add('active');
            document.getElementById('tab-' + btn.dataset.tab).style.display = '';
        });
    });

    // --- App Logs ---
    const appContainer = document.getElementById('app-log-container');
    const appFollowBtn = document.getElementById('app-follow-btn');
    const appRefreshBtn = document.getElementById('app-refresh-btn');
    const appClearBtn = document.getElementById('app-clear-btn');
    const appSearch = document.getElementById('app-search');
    let appFollowing = false;
    let appEventSource = null;
    const activeLevels = new Set(['error', 'warn', 'info']);

    function levelClass(level) {
        const l = level.toLowerCase();
        if (l === 'error') return 'log-error';
        if (l === 'warn') return 'log-warn';
        if (l === 'debug') return 'log-debug';
        if (l === 'trace') return 'log-trace';
        return '';
    }

    function formatFields(fields) {
        return Object.entries(fields || {}).map(([k,v]) => k + '=' + v).join(' ');
    }

    function renderAppEntry(entry) {
        const div = document.createElement('div');
        div.className = 'log-entry ' + levelClass(entry.level);
        div.dataset.level = entry.level.toLowerCase();
        const ts = new Date(entry.timestamp).toLocaleTimeString();
        const fieldsStr = formatFields(entry.fields);
        div.innerHTML = '<span class="log-ts">' + ts + '</span> '
            + '<span class="log-level">' + entry.level + '</span> '
            + '<span class="log-target">' + entry.target + '</span> '
            + entry.message
            + (fieldsStr ? ' <span class="log-fields">' + fieldsStr + '</span>' : '');
        return div;
    }

    function applyFilters() {
        const search = appSearch.value.toLowerCase();
        appContainer.querySelectorAll('.log-entry').forEach(el => {
            const matchesLevel = activeLevels.has(el.dataset.level);
            const matchesSearch = !search || el.textContent.toLowerCase().includes(search);
            el.style.display = (matchesLevel && matchesSearch) ? '' : 'none';
        });
    }

    function loadAppLogs() {
        fetch('/api/logs/app?limit=100')
            .then(r => r.json())
            .then(entries => {
                appContainer.innerHTML = '';
                entries.forEach(e => appContainer.appendChild(renderAppEntry(e)));
                if (entries.length === 0) {
                    appContainer.innerHTML = '<div class="empty-state"><p>No log entries</p></div>';
                }
                applyFilters();
                appContainer.scrollTop = appContainer.scrollHeight;
            });
    }

    function startAppFollow() {
        if (appEventSource) return;
        appFollowing = true;
        appFollowBtn.classList.add('active');
        appEventSource = new EventSource('/api/logs/app/stream');
        appEventSource.onmessage = function(e) {
            const entry = JSON.parse(e.data);
            appContainer.appendChild(renderAppEntry(entry));
            applyFilters();
            appContainer.scrollTop = appContainer.scrollHeight;
        };
        appEventSource.onerror = function() {
            stopAppFollow();
        };
    }

    function stopAppFollow() {
        if (appEventSource) {
            appEventSource.close();
            appEventSource = null;
        }
        appFollowing = false;
        appFollowBtn.classList.remove('active');
    }

    appFollowBtn.addEventListener('click', () => {
        if (appFollowing) stopAppFollow();
        else startAppFollow();
    });
    appRefreshBtn.addEventListener('click', loadAppLogs);
    appClearBtn.addEventListener('click', () => { appContainer.innerHTML = ''; });
    appSearch.addEventListener('input', applyFilters);

    // Level filter buttons
    document.querySelectorAll('.log-level-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            const level = btn.dataset.level;
            if (activeLevels.has(level)) {
                activeLevels.delete(level);
                btn.classList.remove('active');
            } else {
                activeLevels.add(level);
                btn.classList.add('active');
            }
            applyFilters();
        });
    });

    loadAppLogs();

    // --- Server Logs ---
    const serverContainer = document.getElementById('server-log-container');
    const serverFollowBtn = document.getElementById('server-follow-btn');
    const serverRefreshBtn = document.getElementById('server-refresh-btn');
    const serverClearBtn = document.getElementById('server-clear-btn');
    const serverSearch = document.getElementById('server-search');
    const serverStatus = document.getElementById('server-status');
    let serverFollowing = false;
    let serverEventSource = null;

    function renderServerLine(line) {
        const div = document.createElement('div');
        div.className = 'log-entry';
        div.textContent = line;
        return div;
    }

    function applyServerFilters() {
        const search = serverSearch.value.toLowerCase();
        serverContainer.querySelectorAll('.log-entry').forEach(el => {
            el.style.display = (!search || el.textContent.toLowerCase().includes(search)) ? '' : 'none';
        });
    }

    function loadServerLogs() {
        fetch('/api/logs/server?limit=100')
            .then(r => {
                if (!r.ok) throw new Error('not available');
                return r.json();
            })
            .then(lines => {
                serverContainer.innerHTML = '';
                serverStatus.style.display = 'none';
                lines.forEach(l => serverContainer.appendChild(renderServerLine(l)));
                if (lines.length === 0) {
                    serverContainer.innerHTML = '<div class="empty-state"><p>No server logs</p></div>';
                }
                serverContainer.scrollTop = serverContainer.scrollHeight;
            })
            .catch(() => {
                serverContainer.innerHTML = '';
                serverStatus.textContent = 'No server container configured or container not available.';
                serverStatus.style.display = '';
            });
    }

    function startServerFollow() {
        if (serverEventSource) return;
        serverFollowing = true;
        serverFollowBtn.classList.add('active');
        serverEventSource = new EventSource('/api/logs/server/stream');
        serverEventSource.onmessage = function(e) {
            serverContainer.appendChild(renderServerLine(e.data));
            applyServerFilters();
            serverContainer.scrollTop = serverContainer.scrollHeight;
        };
        serverEventSource.addEventListener('error', function(e) {
            if (e.data) {
                serverStatus.textContent = e.data;
                serverStatus.style.display = '';
            }
            stopServerFollow();
        });
    }

    function stopServerFollow() {
        if (serverEventSource) {
            serverEventSource.close();
            serverEventSource = null;
        }
        serverFollowing = false;
        serverFollowBtn.classList.remove('active');
    }

    serverFollowBtn.addEventListener('click', () => {
        if (serverFollowing) stopServerFollow();
        else startServerFollow();
    });
    serverRefreshBtn.addEventListener('click', loadServerLogs);
    serverClearBtn.addEventListener('click', () => { serverContainer.innerHTML = ''; });
    serverSearch.addEventListener('input', applyServerFilters);

    loadServerLogs();
})();
</script>
{% endblock %}
```

- [ ] **Step 2: Add Askama template struct and handler**

In `src/web/handlers/logs.rs`, add:

```rust
use askama::Template;
use crate::web::auth::SessionUser;
use crate::web::flash::{take_flash, FlashMessage};

#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
}

pub async fn logs_page(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let tmpl = LogsTemplate {
        user,
        flash,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 3: Register the /logs route**

In `src/web/mod.rs`, add inside the authenticated scope (the `web::scope("")` block):

```rust
.route("/logs", web::get().to(handlers::logs::logs_page))
```

- [ ] **Step 4: Add Logs to the navigation**

In `templates/partials/nav.html`, add after the Status link:

```html
<a href="/logs"{% if active == "logs" %} class="active"{% endif %}>{% call icons::terminal() %}{% endcall %} Logs</a>
```

Check if the `terminal` icon macro exists in `partials/icons.html`. If not, add one or use an existing icon.

- [ ] **Step 5: Add CSS for the log viewer**

In the project's CSS file (`src/assets/style.css`), add styles for the log viewer:

```css
.tab-bar { display: flex; gap: 0.25rem; margin-bottom: 1rem; }
.tab-btn { padding: 0.5rem 1rem; border: 1px solid var(--border); background: transparent; cursor: pointer; border-radius: 4px 4px 0 0; }
.tab-btn.active { background: var(--bg-card); border-bottom-color: var(--bg-card); }

.log-controls { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem; flex-wrap: wrap; gap: 0.5rem; }
.log-controls-left, .log-controls-right { display: flex; align-items: center; gap: 0.25rem; }
.log-search { padding: 0.25rem 0.5rem; border: 1px solid var(--border); background: var(--bg); border-radius: 4px; font-size: 0.85rem; }

.log-filter-group { display: flex; gap: 2px; }
.log-level-btn { padding: 0.2rem 0.5rem; border: 1px solid var(--border); background: transparent; cursor: pointer; font-size: 0.75rem; border-radius: 3px; }
.log-level-btn.active { background: var(--bg-card); font-weight: bold; }

.log-container { font-family: monospace; font-size: 0.85rem; max-height: 70vh; overflow-y: auto; border: 1px solid var(--border); border-radius: 4px; padding: 0.5rem; background: var(--bg); }
.log-entry { white-space: pre-wrap; word-break: break-all; padding: 1px 0; }
.log-ts { color: var(--text-muted); }
.log-level { font-weight: bold; min-width: 5ch; display: inline-block; }
.log-target { color: var(--text-muted); }
.log-fields { color: var(--text-muted); font-size: 0.8em; }
.log-error { color: var(--error, #e74c3c); }
.log-warn { color: var(--warning, #f39c12); }
.log-debug, .log-trace { color: var(--text-muted); }
```

- [ ] **Step 6: Verify compilation and test page renders**

Run: `cargo build`
Expected: compiles. Then start the dev server and navigate to `/logs` in a browser. Verify:
- Tab switching works
- App logs load (may be empty if no activity)
- Follow toggle connects to SSE stream
- Server logs tab shows appropriate status when no container configured

- [ ] **Step 7: Commit**

```bash
git add templates/logs.html templates/partials/nav.html src/web/handlers/logs.rs src/web/mod.rs src/assets/style.css
git commit -m "feat: add web UI log viewer with tabs, level filter, search, and live tail"
```

---

### Task 7: Integration Tests

**Files:**
- Modify: `src/logging.rs` (add integration test)

**Interfaces:**
- Consumes: `LogBroadcast`, `BroadcastLayer`, `LogEntry` from Task 3

- [ ] **Step 1: Write integration test for broadcast layer receiving tracing events**

In `src/logging.rs`, add to `#[cfg(test)] mod tests`:

```rust
use tracing_subscriber::prelude::*;

#[test]
fn broadcast_layer_captures_tracing_events() {
    let lb = Arc::new(LogBroadcast::new(100));
    let layer = BroadcastLayer::new(Arc::clone(&lb));
    let mut rx = lb.subscribe();

    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::info!(target: "test_target", key = "value", "hello world");

    let entry = rx.try_recv().unwrap();
    assert_eq!(entry.level, "INFO");
    assert_eq!(entry.target, "test_target");
    assert_eq!(entry.message, "hello world");
    assert_eq!(
        entry.fields.get("key"),
        Some(&serde_json::Value::String("value".to_string()))
    );

    let recent = lb.recent(10);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].message, "hello world");
}

#[test]
fn broadcast_layer_captures_multiple_levels() {
    let lb = Arc::new(LogBroadcast::new(100));
    let layer = BroadcastLayer::new(Arc::clone(&lb));
    let filter = tracing_subscriber::EnvFilter::new("trace");

    let subscriber = tracing_subscriber::registry().with(filter).with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::error!("err msg");
    tracing::warn!("warn msg");
    tracing::debug!("debug msg");

    let recent = lb.recent(10);
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].level, "ERROR");
    assert_eq!(recent[1].level, "WARN");
    assert_eq!(recent[2].level, "DEBUG");
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --lib logging::tests`
Expected: all tests pass, including the two new integration tests.

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: all tests pass, no regressions.

- [ ] **Step 4: Commit**

```bash
git add src/logging.rs
git commit -m "test: add integration tests for broadcast layer capturing tracing events"
```
