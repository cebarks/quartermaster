// Config system is incrementally used by CLI commands (tasks 7-12).
// load_with_env and apply_env_overrides are used by resolve_context in common.rs
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::distr::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};

fn default_queue_changes() -> bool {
    true
}

fn default_web_bind() -> String {
    "0.0.0.0".to_string()
}

fn default_web_port() -> u16 {
    9190
}

fn default_session_secret() -> String {
    String::new()
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub spt_dir: Option<PathBuf>,

    #[serde(default)]
    pub forge_token: Option<String>,

    #[serde(default = "default_queue_changes")]
    pub queue_changes: bool,

    #[serde(default)]
    pub auto_drain_on_lifecycle: bool,

    #[serde(default = "default_session_secret")]
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

    #[serde(default)]
    #[serde(skip_serializing_if = "LoggingConfig::is_default")]
    pub logging: LoggingConfig,
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
            web_bind: "0.0.0.0".to_string(),
            web_port: 9190,
            logging: LoggingConfig::default(),
        }
    }
}

impl Config {
    /// Load config from a TOML file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        tracing::debug!(path = %path.display(), "loading config file");
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        let config: Config =
            toml::from_str(&contents).with_context(|| "failed to parse config TOML")?;
        Ok(config)
    }

    /// Save config to a TOML file at `path`, creating parent directories if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory: {}", parent.display())
            })?;
        }
        let contents =
            toml::to_string_pretty(self).with_context(|| "failed to serialize config to TOML")?;
        std::fs::write(path, &contents)
            .with_context(|| format!("failed to write config file: {}", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms).ok();
        }

        Ok(())
    }

    /// Load config from a TOML file, then apply QUMA_* environment variable overrides.
    pub fn load_with_env(path: &Path) -> Result<Self> {
        let mut config = Self::load(path)?;
        config.apply_env_overrides();
        tracing::debug!("applied environment variable overrides to config");
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
        Ok(config)
    }

    /// Override config fields from QUMA_* environment variables.
    ///
    /// Supported variables:
    /// - `QUMA_SPT_DIR` -> `spt_dir`
    /// - `QUMA_FORGE_TOKEN` -> `forge_token`
    /// - `QUMA_WEB_PORT` -> `web_port`
    /// - `QUMA_WEB_BIND` -> `web_bind`
    /// - `QUMA_SERVER_CONTAINER` -> `server_container`
    /// - `QUMA_SERVER_HOST` -> `server_host`
    /// - `QUMA_SERVER_PORT` -> `server_port`
    /// - `QUMA_LOG_LEVEL` -> `logging.level`
    /// - `QUMA_LOG_FILE_PATH` -> `logging.file.path`
    /// - `QUMA_LOG_FILE_ENABLED` -> `logging.file.enabled`
    pub fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("QUMA_SPT_DIR") {
            tracing::debug!(var = "QUMA_SPT_DIR", value = %val, "env var override applied");
            self.spt_dir = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("QUMA_FORGE_TOKEN") {
            tracing::debug!(
                var = "QUMA_FORGE_TOKEN",
                value = "<redacted>",
                "env var override applied"
            );
            self.forge_token = Some(val);
        }
        if let Ok(val) = std::env::var("QUMA_WEB_BIND") {
            tracing::debug!(var = "QUMA_WEB_BIND", value = %val, "env var override applied");
            self.web_bind = val;
        }
        if let Ok(val) = std::env::var("QUMA_WEB_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                tracing::debug!(var = "QUMA_WEB_PORT", value = %val, "env var override applied");
                self.web_port = port;
            }
        }
        if let Ok(val) = std::env::var("QUMA_SERVER_CONTAINER") {
            tracing::debug!(var = "QUMA_SERVER_CONTAINER", value = %val, "env var override applied");
            self.server_container = Some(val);
        }
        if let Ok(val) = std::env::var("QUMA_SERVER_HOST") {
            tracing::debug!(var = "QUMA_SERVER_HOST", value = %val, "env var override applied");
            self.server_host = Some(val);
        }
        if let Ok(val) = std::env::var("QUMA_SERVER_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                tracing::debug!(var = "QUMA_SERVER_PORT", value = %val, "env var override applied");
                self.server_port = Some(port);
            }
        }
        if let Ok(val) = std::env::var("QUMA_LOG_LEVEL") {
            tracing::debug!(var = "QUMA_LOG_LEVEL", value = %val, "env var override applied");
            self.logging.level = val;
        }
        if let Ok(val) = std::env::var("QUMA_LOG_FILE_PATH") {
            tracing::debug!(var = "QUMA_LOG_FILE_PATH", value = %val, "env var override applied");
            self.logging.file.path = val;
        }
        if let Ok(val) = std::env::var("QUMA_LOG_FILE_ENABLED") {
            if val.eq_ignore_ascii_case("true") {
                tracing::debug!(var = "QUMA_LOG_FILE_ENABLED", value = %val, "env var override applied");
                self.logging.file.enabled = true;
            } else if val.eq_ignore_ascii_case("false") {
                tracing::debug!(var = "QUMA_LOG_FILE_ENABLED", value = %val, "env var override applied");
                self.logging.file.enabled = false;
            }
        }
    }

    /// If `session_secret` is empty, generate a random 48-character alphanumeric secret.
    pub fn ensure_session_secret(&mut self) {
        if self.session_secret.is_empty() {
            self.session_secret = rand::rng()
                .sample_iter(Alphanumeric)
                .take(48)
                .map(char::from)
                .collect();
        }
    }

    /// Resolve the config file path using this priority:
    /// 1. Explicit CLI flag (`cli_config`)
    /// 2. `QUMA_CONFIG` environment variable
    /// 3. `<spt_dir>/quartermaster.toml`
    /// 4. `quartermaster.toml` (current directory)
    pub fn resolve_path(cli_config: Option<&Path>, spt_dir: Option<&Path>) -> PathBuf {
        if let Some(path) = cli_config {
            return path.to_path_buf();
        }

        if let Ok(env_path) = std::env::var("QUMA_CONFIG") {
            return PathBuf::from(env_path);
        }

        if let Some(dir) = spt_dir {
            return dir.join("quartermaster.toml");
        }

        PathBuf::from("quartermaster.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_full_config() {
        let toml_str = r#"
spt_dir = "/opt/spt"
forge_token = "tok_abc123"
queue_changes = false
auto_drain_on_lifecycle = true
session_secret = "supersecret"
server_container = "spt-server"
server_host = "192.168.1.100"
server_port = 6969
web_bind = "127.0.0.1"
web_port = 8080
"#;

        let config: Config = toml::from_str(toml_str).expect("should parse full TOML");

        assert_eq!(config.spt_dir, Some(PathBuf::from("/opt/spt")));
        assert_eq!(config.forge_token, Some("tok_abc123".to_string()));
        assert!(!config.queue_changes);
        assert!(config.auto_drain_on_lifecycle);
        assert_eq!(config.session_secret, "supersecret");
        assert_eq!(config.server_container, Some("spt-server".to_string()));
        assert_eq!(config.server_host, Some("192.168.1.100".to_string()));
        assert_eq!(config.server_port, Some(6969));
        assert_eq!(config.web_bind, "127.0.0.1");
        assert_eq!(config.web_port, 8080);
    }

    #[test]
    fn deserialize_minimal_config() {
        let config: Config = toml::from_str("").expect("should parse empty TOML");

        assert_eq!(config.spt_dir, None);
        assert_eq!(config.forge_token, None);
        assert!(config.queue_changes); // default: true
        assert!(!config.auto_drain_on_lifecycle); // default: false
        assert_eq!(config.session_secret, "");
        assert_eq!(config.server_container, None);
        assert_eq!(config.server_host, None);
        assert_eq!(config.server_port, None);
        assert_eq!(config.web_bind, "0.0.0.0");
        assert_eq!(config.web_port, 9190);
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("quartermaster.toml");

        let toml_content = r#"
spt_dir = "/srv/spt"
web_port = 3000
"#;
        std::fs::write(&config_path, toml_content).expect("write");

        let config = Config::load(&config_path).expect("should load from file");

        assert_eq!(config.spt_dir, Some(PathBuf::from("/srv/spt")));
        assert_eq!(config.web_port, 3000);
        // Defaults for unspecified fields
        assert!(config.queue_changes);
        assert_eq!(config.web_bind, "0.0.0.0");
    }

    #[test]
    fn save_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("nested/dir/quartermaster.toml");

        let mut config = Config::default();
        config.spt_dir = Some(PathBuf::from("/opt/game"));
        config.web_port = 7777;
        config.forge_token = Some("my-token".to_string());

        config.save(&config_path).expect("should save");
        let reloaded = Config::load(&config_path).expect("should reload");

        assert_eq!(config, reloaded);
    }

    #[test]
    fn env_var_overlay() {
        temp_env::with_vars(
            [
                ("QUMA_SPT_DIR", Some("/env/spt")),
                ("QUMA_FORGE_TOKEN", Some("env_token")),
                ("QUMA_WEB_PORT", Some("4000")),
                ("QUMA_WEB_BIND", Some("10.0.0.1")),
                ("QUMA_SERVER_CONTAINER", Some("env-container")),
                ("QUMA_SERVER_HOST", Some("env-host")),
                ("QUMA_SERVER_PORT", Some("6970")),
            ],
            || {
                let mut config = Config::default();
                config.apply_env_overrides();

                assert_eq!(config.spt_dir, Some(PathBuf::from("/env/spt")));
                assert_eq!(config.forge_token, Some("env_token".to_string()));
                assert_eq!(config.web_port, 4000);
                assert_eq!(config.web_bind, "10.0.0.1");
                assert_eq!(config.server_container, Some("env-container".to_string()));
                assert_eq!(config.server_host, Some("env-host".to_string()));
                assert_eq!(config.server_port, Some(6970));
            },
        );
    }

    #[test]
    fn generate_session_secret_if_empty() {
        let mut config = Config::default();
        assert!(config.session_secret.is_empty());

        config.ensure_session_secret();

        assert_eq!(config.session_secret.len(), 48);
        assert!(config
            .session_secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric()));

        // Calling again should not change an existing secret
        let first_secret = config.session_secret.clone();
        config.ensure_session_secret();
        assert_eq!(config.session_secret, first_secret);
    }

    #[test]
    fn resolve_config_path() {
        // When spt_dir is provided but no CLI flag, should use spt_dir/quartermaster.toml
        let spt = PathBuf::from("/opt/spt");
        let result = Config::resolve_path(None, Some(&spt));
        assert_eq!(result, PathBuf::from("/opt/spt/quartermaster.toml"));
    }

    #[test]
    fn resolve_config_path_explicit() {
        // Explicit CLI path should take precedence over spt_dir
        let explicit = PathBuf::from("/custom/config.toml");
        let spt = PathBuf::from("/opt/spt");
        let result = Config::resolve_path(Some(&explicit), Some(&spt));
        assert_eq!(result, PathBuf::from("/custom/config.toml"));
    }

    #[test]
    fn resolve_config_path_fallback() {
        temp_env::with_vars_unset(["QUMA_CONFIG"], || {
            let result = Config::resolve_path(None, None);
            assert_eq!(result, PathBuf::from("quartermaster.toml"));
        });
    }

    #[test]
    fn resolve_config_path_env_override() {
        temp_env::with_vars([("QUMA_CONFIG", Some("/env/path/config.toml"))], || {
            let result = Config::resolve_path(None, Some(Path::new("/opt/spt")));
            assert_eq!(result, PathBuf::from("/env/path/config.toml"));
        });
    }

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
        assert!(
            !serialized.contains("[logging]"),
            "default logging config should not be serialized"
        );
    }

    #[test]
    fn logging_config_serialized_when_non_default() {
        let mut config = Config::default();
        config.logging.level = "debug".to_string();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(
            serialized.contains("[logging]"),
            "non-default logging config should be serialized"
        );
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
}
