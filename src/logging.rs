use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;

use crate::config::{LogFormat, LoggingConfig, RotationPolicy};

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
// LogBroadcast — tokio broadcast channel + bounded ring buffer for recent()
// ---------------------------------------------------------------------------

pub struct LogBroadcast {
    sender: broadcast::Sender<LogEntry>,
    buffer: Arc<RwLock<VecDeque<LogEntry>>>,
    buffer_size: usize,
}

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

// ---------------------------------------------------------------------------
// ReloadHandles — holds reload handles for runtime reconfiguration
// ---------------------------------------------------------------------------

// The subscriber is built as a layered stack. Each reload::Handle stores the
// subscriber type `S` at the point where its layer was inserted:
//
//   Registry
//     + reload::Layer<EnvFilter>          → S0 = Registry
//     + reload::Layer<BoxedConsole>       → S1 = Layered<reload::Layer<EnvFilter, S0>, S0>
//     + reload::Layer<Option<BoxedFile>>  → S2 = Layered<reload::Layer<BoxedConsole, S1>, S1>
//     + BroadcastLayer                   → (not reloadable)
//
// We define type aliases for each subscriber level to keep handle types correct.

type S0 = tracing_subscriber::Registry;
type S1 = tracing_subscriber::layer::Layered<reload::Layer<EnvFilter, S0>, S0>;
type BoxedConsole = Box<dyn Layer<S1> + Send + Sync>;
type S2 = tracing_subscriber::layer::Layered<reload::Layer<BoxedConsole, S1>, S1>;
type BoxedFile = Option<Box<dyn Layer<S2> + Send + Sync>>;

pub struct ReloadHandles {
    filter_handle: reload::Handle<EnvFilter, S0>,
    console_handle: reload::Handle<BoxedConsole, S1>,
    file_handle: reload::Handle<BoxedFile, S2>,
    // Store the file worker guard to prevent premature flush/drop
    file_guard: std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
}

impl ReloadHandles {
    pub fn reconfigure(&self, config: &LoggingConfig, filter_str: &str, spt_dir: Option<&Path>) {
        // Update filter
        if let Ok(new_filter) = EnvFilter::try_new(filter_str) {
            let _ = self.filter_handle.reload(new_filter);
        }

        // Update console layer
        if config.console.enabled {
            let layer: BoxedConsole = match config.console.format {
                LogFormat::Json => Box::new(fmt::layer().json().with_writer(std::io::stderr)),
                LogFormat::Text => Box::new(fmt::layer().with_writer(std::io::stderr)),
            };
            let _ = self.console_handle.reload(layer);
        } else {
            // Disabled: use an identity layer (no-op passthrough)
            let layer: BoxedConsole = Box::new(tracing_subscriber::layer::Identity::new());
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

            let (file_layer, guard) = build_file_layer(&config.file, &file_path);
            let _ = self.file_handle.reload(Some(file_layer));
            // Store the new guard, dropping the old one (if any)
            *self.file_guard.lock().unwrap() = Some(guard);
        } else {
            let _ = self.file_handle.reload(None);
            // Clear the guard when file logging is disabled
            *self.file_guard.lock().unwrap() = None;
        }
    }
}

// ---------------------------------------------------------------------------
// init_subscriber — bootstrap the layered subscriber with reload handles
// ---------------------------------------------------------------------------

pub fn init_subscriber(log_broadcast: &Arc<LogBroadcast>) -> ReloadHandles {
    // Default filter: before config loads we use a sensible baseline.
    let filter = EnvFilter::new("info,quartermaster=debug");
    let (filter_layer, filter_handle) = reload::Layer::new(filter);

    // Console layer (default: text to stderr)
    let console_layer: BoxedConsole = Box::new(fmt::layer().with_writer(std::io::stderr));
    let (console_reload, console_handle) = reload::Layer::new(console_layer);

    // File layer (default: disabled / None)
    let file_layer: BoxedFile = None;
    let (file_reload, file_handle) = reload::Layer::new(file_layer);

    // Broadcast layer — always active, feeds the web UI ring buffer
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
        file_guard: std::sync::Mutex::new(None),
    }
}

// ---------------------------------------------------------------------------
// resolve_log_filter — priority chain for log filter string
// ---------------------------------------------------------------------------

/// Resolve the effective EnvFilter string from the priority chain:
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
                LogFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                LogFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
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
                LogFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                LogFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
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
                LogFormat::Json => Box::new(fmt::layer().json().with_writer(non_blocking)),
                LogFormat::Text => Box::new(fmt::layer().with_writer(non_blocking)),
            };
            (layer, guard)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
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
}
