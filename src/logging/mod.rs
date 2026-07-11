use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload;
use tracing_subscriber::Layer;

use crate::config::{ConsoleFormat, FileFormat, LoggingConfig, RotationPolicy};
use crate::dirs::QumaDirs;

mod compact;
pub mod writer;

// ---------------------------------------------------------------------------
// LogEntry — the structured log record shared via broadcast + ring buffer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// LogBroadcast — tokio broadcast channel for real-time log streaming
// ---------------------------------------------------------------------------

pub struct LogBroadcast {
    sender: broadcast::Sender<LogEntry>,
}

impl LogBroadcast {
    pub fn new(buffer_size: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size.max(16));
        Self { sender }
    }

    pub fn send(&self, entry: LogEntry) {
        let _ = self.sender.send(entry);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.sender.subscribe()
    }
}

// ---------------------------------------------------------------------------
// FieldVisitor — extracts tracing event fields into a HashMap + message
// ---------------------------------------------------------------------------

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
        self.fields
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }
}

// ---------------------------------------------------------------------------
// BroadcastLayer — tracing Layer that feeds events into LogBroadcast
// ---------------------------------------------------------------------------

pub struct BroadcastLayer {
    broadcast: Arc<LogBroadcast>,
    db_sender: Option<mpsc::UnboundedSender<LogEntry>>,
}

impl BroadcastLayer {
    pub fn new(broadcast: Arc<LogBroadcast>) -> Self {
        Self {
            broadcast,
            db_sender: None,
        }
    }

    pub fn with_db_sender(mut self, sender: mpsc::UnboundedSender<LogEntry>) -> Self {
        self.db_sender = Some(sender);
        self
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

        self.broadcast.send(entry.clone());

        if let Some(ref sender) = self.db_sender {
            let _ = sender.send(entry);
        }
    }
}

// ---------------------------------------------------------------------------
// LevelFilterLayer — wraps an inner layer, filtering events by level
// ---------------------------------------------------------------------------
//
// This provides per-layer level filtering without using `Filtered<L, F, S>`,
// which breaks after `reload()` because `FilterId` is only assigned once at
// subscriber registration. Instead, we check the event level in `on_event()`
// and delegate all other `Layer` trait methods unconditionally.

struct LevelFilterLayer<L> {
    inner: L,
    max_level: tracing::Level,
}

impl<S, L> Layer<S> for LevelFilterLayer<L>
where
    S: Subscriber,
    L: Layer<S>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().level() <= &self.max_level {
            self.inner.on_event(event, ctx);
        }
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        self.inner.on_new_span(attrs, id, ctx);
    }

    fn on_record(
        &self,
        span: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        self.inner.on_record(span, values, ctx);
    }

    fn on_enter(&self, id: &tracing::span::Id, ctx: Context<'_, S>) {
        self.inner.on_enter(id, ctx);
    }

    fn on_exit(&self, id: &tracing::span::Id, ctx: Context<'_, S>) {
        self.inner.on_exit(id, ctx);
    }

    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        self.inner.on_close(id, ctx);
    }
}

// ---------------------------------------------------------------------------
// Floor filter computation — union of all layer needs
// ---------------------------------------------------------------------------

/// Compute the global floor `Targets` filter that allows through events at the
/// most permissive level required by any active output layer. Each layer then
/// independently checks its own threshold in `LevelFilterLayer::on_event()`.
fn compute_floor_filter(config: &LoggingConfig, console_filter_str: &str) -> Targets {
    let console_level = parse_most_permissive_level(console_filter_str);
    let file_level = if config.file.enabled {
        parse_level_str(&config.file.level).unwrap_or(tracing::Level::DEBUG)
    } else {
        tracing::Level::ERROR // disabled layer doesn't need floor events
    };
    let web_level = parse_level_str(&config.web.level).unwrap_or(tracing::Level::INFO);

    let floor = most_permissive(&[console_level, file_level, web_level]);

    // quartermaster crate always gets at least debug in the floor so that
    // file/web layers can capture crate-level debug even when console is info
    let qm_floor = most_permissive(&[floor, tracing::Level::DEBUG]);

    Targets::new()
        .with_default(floor)
        .with_target("quartermaster", qm_floor)
}

fn parse_level_str(s: &str) -> Option<tracing::Level> {
    s.parse::<tracing::Level>().ok()
}

/// Return the most permissive (most verbose) level in the slice.
/// tracing::Level ordering: ERROR < WARN < INFO < DEBUG < TRACE (greater = more verbose).
fn most_permissive(levels: &[tracing::Level]) -> tracing::Level {
    levels.iter().copied().max().unwrap_or(tracing::Level::INFO)
}

/// Parse a comma-separated filter string and return the most permissive level
/// mentioned anywhere (including per-target directives). Used for the floor
/// filter which needs to pass events from ALL targets.
fn parse_most_permissive_level(filter_str: &str) -> tracing::Level {
    let mut most = tracing::Level::ERROR;
    for part in filter_str.split(',') {
        let part = part.trim();
        let level_str = part.split_once('=').map(|(_, l)| l).unwrap_or(part);
        if let Ok(level) = level_str.parse::<tracing::Level>() {
            if level > most {
                most = level;
            }
        }
    }
    most
}

/// Parse a comma-separated filter string and return only the DEFAULT level
/// (directives without a target= prefix). Per-target directives like
/// "quartermaster=debug" are ignored. Used for the console LevelFilterLayer
/// which should show INFO for all crates even when specific crates have DEBUG.
fn parse_default_level(filter_str: &str) -> tracing::Level {
    for part in filter_str.split(',') {
        let part = part.trim();
        if !part.contains('=') {
            if let Ok(level) = part.parse::<tracing::Level>() {
                return level;
            }
        }
    }
    tracing::Level::INFO
}

// ---------------------------------------------------------------------------
// ReloadHandles — holds reload handles for runtime reconfiguration
// ---------------------------------------------------------------------------

// The subscriber is built as a layered stack. Each reload::Handle stores the
// subscriber type `S` at the point where its layer was inserted:
//
//   Registry
//     + reload::Layer<Targets>            → S0 = Registry
//     + reload::Layer<BoxedConsole>       → S1 = Layered<reload::Layer<Targets, S0>, S0>
//     + reload::Layer<Option<BoxedFile>>  → S2 = Layered<reload::Layer<BoxedConsole, S1>, S1>
//     + BroadcastLayer                   → (not reloadable)
//
// The global Targets filter is the "floor" — set to the most permissive level
// needed by any active output layer. Each output layer then independently
// checks event level via LevelFilterLayer.

type S0 = tracing_subscriber::Registry;
type S1 = tracing_subscriber::layer::Layered<reload::Layer<Targets, S0>, S0>;
type BoxedConsole = Box<dyn Layer<S1> + Send + Sync>;
type S2 = tracing_subscriber::layer::Layered<reload::Layer<BoxedConsole, S1>, S1>;
type BoxedFile = Option<Box<dyn Layer<S2> + Send + Sync>>;

pub struct ReloadHandles {
    filter_handle: reload::Handle<Targets, S0>,
    console_handle: reload::Handle<BoxedConsole, S1>,
    file_handle: reload::Handle<BoxedFile, S2>,
    // Store the file worker guard to prevent premature flush/drop
    file_guard: std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
}

impl ReloadHandles {
    pub fn reconfigure(&self, config: &LoggingConfig, filter_str: &str, dirs: Option<&QumaDirs>) {
        // Update global floor filter to the most permissive level any layer needs
        let floor = compute_floor_filter(config, filter_str);
        let _ = self.filter_handle.reload(floor);

        // Console level uses only the default level from the filter string,
        // not per-target directives. "info,quartermaster=debug" → console at INFO.
        let console_level = parse_default_level(filter_str);

        // Update console layer (with per-layer level filtering)
        if config.console.enabled {
            let layer: BoxedConsole = match config.console.format {
                ConsoleFormat::Json => Box::new(LevelFilterLayer {
                    inner: fmt::layer().json().with_writer(std::io::stderr),
                    max_level: console_level,
                }),
                ConsoleFormat::Full => Box::new(LevelFilterLayer {
                    inner: fmt::layer().with_writer(std::io::stderr),
                    max_level: console_level,
                }),
                ConsoleFormat::Compact => {
                    let formatter = compact::CompactFormatter {
                        use_ansi: std::io::stderr().is_terminal(),
                    };
                    Box::new(LevelFilterLayer {
                        inner: fmt::layer()
                            .event_format(formatter)
                            .with_writer(std::io::stderr)
                            .with_ansi(false), // formatter handles its own color
                        max_level: console_level,
                    })
                }
            };
            let _ = self.console_handle.reload(layer);
        } else {
            // Disabled: use an identity layer (no-op passthrough)
            let layer: BoxedConsole = Box::new(tracing_subscriber::layer::Identity::new());
            let _ = self.console_handle.reload(layer);
        }

        // Update file layer (with per-layer level filtering)
        if config.file.enabled {
            let file_path = resolve_file_path(&config.file.path, dirs);
            let file_level = parse_level_str(&config.file.level).unwrap_or(tracing::Level::DEBUG);
            let (file_layer, guard) = build_file_layer(&config.file, &file_path);
            let filtered: BoxedFile = Some(Box::new(LevelFilterLayer {
                inner: file_layer,
                max_level: file_level,
            }));
            let _ = self.file_handle.reload(filtered);
            // Store the new guard, dropping the old one (if any)
            *self.file_guard.lock().expect("mutex poisoned") = Some(guard);
        } else {
            let _ = self.file_handle.reload(None);
            // Clear the guard when file logging is disabled
            *self.file_guard.lock().expect("mutex poisoned") = None;
        }
    }
}

fn resolve_file_path(path: &str, dirs: Option<&QumaDirs>) -> std::path::PathBuf {
    if Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        dirs.map(|d| d.root.join(path))
            .unwrap_or_else(|| std::path::PathBuf::from(path))
    }
}

// ---------------------------------------------------------------------------
// init_subscriber — bootstrap the layered subscriber with reload handles
// ---------------------------------------------------------------------------

pub fn init_subscriber(
    log_broadcast: &Arc<LogBroadcast>,
    db_sender: Option<mpsc::UnboundedSender<LogEntry>>,
) -> ReloadHandles {
    // Default floor filter: before config loads we use a sensible baseline.
    // This will be replaced by compute_floor_filter() on first reconfigure().
    let floor = Targets::new()
        .with_default(tracing::Level::INFO)
        .with_target("quartermaster", tracing::Level::DEBUG);
    let (floor_layer, filter_handle) = reload::Layer::new(floor);

    // Console layer (default: compact format, info level)
    let console_layer: BoxedConsole = Box::new(LevelFilterLayer {
        inner: fmt::layer()
            .event_format(compact::CompactFormatter {
                use_ansi: std::io::stderr().is_terminal(),
            })
            .with_writer(std::io::stderr)
            .with_ansi(false),
        max_level: tracing::Level::INFO,
    });
    let (console_reload, console_handle) = reload::Layer::new(console_layer);

    // File layer (default: disabled / None)
    let file_layer: BoxedFile = None;
    let (file_reload, file_handle) = reload::Layer::new(file_layer);

    // Broadcast layer — always active, feeds the web UI ring buffer
    let mut broadcast_layer = BroadcastLayer::new(Arc::clone(log_broadcast));
    if let Some(sender) = db_sender {
        broadcast_layer = broadcast_layer.with_db_sender(sender);
    }

    tracing_subscriber::registry()
        .with(floor_layer)
        .with(console_reload)
        .with(file_reload)
        .with(broadcast_layer)
        .init();

    ReloadHandles {
        filter_handle,
        console_handle,
        file_handle,
        file_guard: std::sync::Mutex::new(None),
    }
}

/// Create ReloadHandles without setting the global subscriber.
/// Used in tests where the global subscriber may already be set.
#[allow(dead_code)]
pub fn init_reload_handles_only() -> ReloadHandles {
    let floor = Targets::new().with_default(tracing::Level::INFO);
    let (_floor_layer, filter_handle) = reload::Layer::new(floor);
    let console_layer: BoxedConsole = Box::new(fmt::layer().with_writer(std::io::stderr));
    let (_console_reload, console_handle) = reload::Layer::new(console_layer);
    let file_layer: BoxedFile = None;
    let (_file_reload, file_handle) = reload::Layer::new(file_layer);
    ReloadHandles {
        filter_handle,
        console_handle,
        file_handle,
        file_guard: std::sync::Mutex::new(None),
    }
}

// ---------------------------------------------------------------------------
// resolve_log_filter — priority chain for log filter string
// ---------------------------------------------------------------------------

/// Resolve the effective log filter string from the priority chain:
/// 1. --log-level CLI flag (highest)
/// 2. -v / -vv CLI flags
/// 3. QUMA_LOG_LEVEL env var (already applied to config by apply_env_overrides)
/// 4. RUST_LOG env var
/// 5. config.logging.level
/// 6. hardcoded default: "info,quartermaster=debug"
pub fn resolve_log_filter(config: &LoggingConfig, verbose: u8, log_level: Option<&str>) -> String {
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
            part.split('=')
                .next()
                .is_none_or(|key| key.trim() != crate_name)
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// build_file_layer — construct a file-writing layer from config
// ---------------------------------------------------------------------------

fn build_file_layer(
    config: &crate::config::FileLogConfig,
    path: &Path,
) -> (
    Box<dyn Layer<S2> + Send + Sync>,
    tracing_appender::non_blocking::WorkerGuard,
) {
    let dir = path.parent().unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("quartermaster.log");

    match config.rotation {
        RotationPolicy::Daily => {
            let appender = tracing_appender::rolling::daily(dir, filename);
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let layer: Box<dyn Layer<S2> + Send + Sync> = match config.format {
                FileFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                FileFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
            };
            (layer, guard)
        }
        RotationPolicy::Size => {
            let max_bytes = config.max_size_mb * 1024 * 1024;
            let appender = rolling_file::BasicRollingFileAppender::new(
                path,
                rolling_file::RollingConditionBasic::new().max_size(max_bytes),
                config.max_files,
            )
            .expect("failed to create rolling file appender");
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let layer: Box<dyn Layer<S2> + Send + Sync> = match config.format {
                FileFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                FileFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
            };
            (layer, guard)
        }
        RotationPolicy::None => {
            // No rotation — use a single-file appender (max_size = u64::MAX, 1 file)
            let appender = rolling_file::BasicRollingFileAppender::new(
                path,
                rolling_file::RollingConditionBasic::new().max_size(u64::MAX),
                1,
            )
            .expect("failed to create file appender");
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let layer: Box<dyn Layer<S2> + Send + Sync> = match config.format {
                FileFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                FileFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
            };
            (layer, guard)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_filter() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "info,quartermaster=debug");
            },
        );
    }

    #[test]
    fn config_level_overrides_default() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let mut config = LoggingConfig::default();
                config.level = "warn".to_string();
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "warn,quartermaster=debug");
            },
        );
    }

    #[test]
    fn verbose_flag_sets_crate_debug() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 1, None);
                assert_eq!(result, "info,quartermaster=debug");
            },
        );
    }

    #[test]
    fn double_verbose_sets_crate_trace() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 2, None);
                assert_eq!(result, "info,quartermaster=trace");
            },
        );
    }

    #[test]
    fn log_level_flag_overrides_everything() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let mut config = LoggingConfig::default();
                config.level = "warn".to_string();
                let result = resolve_log_filter(&config, 2, Some("error"));
                assert_eq!(result, "error");
            },
        );
    }

    #[test]
    fn rust_log_env_overrides_config() {
        temp_env::with_vars(
            [
                ("RUST_LOG", Some("debug,hyper=warn")),
                ("QUMA_LOG_LEVEL", None),
            ],
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
            [
                ("RUST_LOG", Some("debug,hyper=warn")),
                ("QUMA_LOG_LEVEL", None),
            ],
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
            [
                ("RUST_LOG", Some("debug,hyper=warn")),
                ("QUMA_LOG_LEVEL", Some("error")),
            ],
            || {
                let mut config = LoggingConfig::default();
                config.level = "error".to_string(); // simulates apply_env_overrides
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "error,quartermaster=debug");
            },
        );
    }

    // --- LogBroadcast tests ---

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

    // --- BroadcastLayer integration tests ---

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
    }

    #[test]
    fn broadcast_layer_captures_multiple_levels() {
        let lb = Arc::new(LogBroadcast::new(100));
        let layer = BroadcastLayer::new(Arc::clone(&lb));
        let mut rx = lb.subscribe();
        let filter = Targets::new().with_default(tracing::Level::TRACE);

        let subscriber = tracing_subscriber::registry().with(filter).with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::error!("err msg");
        tracing::warn!("warn msg");
        tracing::debug!("debug msg");

        let e1 = rx.try_recv().unwrap();
        let e2 = rx.try_recv().unwrap();
        let e3 = rx.try_recv().unwrap();
        assert_eq!(e1.level, "ERROR");
        assert_eq!(e2.level, "WARN");
        assert_eq!(e3.level, "DEBUG");
    }

    // --- Floor filter computation tests ---

    #[test]
    fn compute_floor_filter_uses_most_permissive() {
        // console=info, file=debug (enabled), web=info → floor should be debug
        let config = LoggingConfig {
            level: "info".to_string(),
            file: crate::config::FileLogConfig {
                enabled: true,
                level: "debug".to_string(),
                ..crate::config::FileLogConfig::default()
            },
            web: crate::config::WebLogConfig {
                level: "info".to_string(),
                ..crate::config::WebLogConfig::default()
            },
            ..LoggingConfig::default()
        };
        let floor = compute_floor_filter(&config, "info,quartermaster=debug");
        // Floor should allow debug-level events through (file needs them)
        assert!(floor.would_enable("quartermaster::ops", &tracing::Level::DEBUG));
        assert!(floor.would_enable("some_crate", &tracing::Level::INFO));
    }

    #[test]
    fn compute_floor_filter_file_disabled() {
        // console=info, file disabled (level=debug ignored), web=info → floor = info
        let config = LoggingConfig {
            level: "info".to_string(),
            file: crate::config::FileLogConfig {
                enabled: false,
                level: "debug".to_string(),
                ..crate::config::FileLogConfig::default()
            },
            web: crate::config::WebLogConfig {
                level: "info".to_string(),
                ..crate::config::WebLogConfig::default()
            },
            ..LoggingConfig::default()
        };
        let floor = compute_floor_filter(&config, "info");
        // Floor should be info for non-quartermaster targets
        assert!(floor.would_enable("some_crate", &tracing::Level::INFO));
        assert!(!floor.would_enable("some_crate", &tracing::Level::DEBUG));
        // quartermaster still gets debug in the floor (hardcoded minimum)
        assert!(floor.would_enable("quartermaster::ops", &tracing::Level::DEBUG));
    }

    #[test]
    fn compute_floor_filter_trace_console() {
        // console=trace → floor should be trace everywhere
        let config = LoggingConfig {
            level: "trace".to_string(),
            file: crate::config::FileLogConfig {
                enabled: false,
                ..crate::config::FileLogConfig::default()
            },
            ..LoggingConfig::default()
        };
        let floor = compute_floor_filter(&config, "trace");
        assert!(floor.would_enable("some_crate", &tracing::Level::TRACE));
        assert!(floor.would_enable("quartermaster", &tracing::Level::TRACE));
    }

    // --- Helper function tests ---

    #[test]
    fn parse_most_permissive_level_picks_lowest() {
        assert_eq!(
            parse_most_permissive_level("info,quartermaster=debug"),
            tracing::Level::DEBUG
        );
        assert_eq!(
            parse_most_permissive_level("warn,hyper=error"),
            tracing::Level::WARN
        );
        assert_eq!(parse_most_permissive_level("trace"), tracing::Level::TRACE);
    }

    #[test]
    fn most_permissive_returns_min_level() {
        assert_eq!(
            most_permissive(&[tracing::Level::INFO, tracing::Level::DEBUG]),
            tracing::Level::DEBUG
        );
        assert_eq!(
            most_permissive(&[
                tracing::Level::ERROR,
                tracing::Level::WARN,
                tracing::Level::TRACE
            ]),
            tracing::Level::TRACE
        );
    }
}
