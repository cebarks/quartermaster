use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rand::distr::Alphanumeric;
use rand::RngExt;
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

fn default_update_check_interval() -> u64 {
    300
}

fn default_forge_cache_ttl() -> Option<u64> {
    Some(86400)
}

fn default_auto_start_server() -> bool {
    true
}

fn default_tls_enabled() -> bool {
    true
}

fn default_proxy_enabled() -> bool {
    true
}

fn default_leaderboard_min_raids() -> u32 {
    5
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
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    Auto,
    Manual,
}

fn default_restart_policy() -> RestartPolicy {
    RestartPolicy::Auto
}
fn default_max_restart_attempts() -> u32 {
    5
}
fn default_restart_backoff_cap() -> u64 {
    300
}
fn default_base_udp_port() -> u16 {
    25565
}
fn default_headless_image() -> String {
    "ghcr.io/zhliau/fika-headless-docker:latest".to_string()
}
fn default_isolated_paths() -> Vec<String> {
    vec!["BepInEx/config".to_string()]
}

fn default_enforced() -> bool {
    true
}

fn default_restart_required() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncConfig {
    #[serde(default = "default_enforced")]
    pub enforced: bool,

    #[serde(default)]
    pub silent: bool,

    #[serde(default = "default_restart_required")]
    pub restart_required: bool,

    #[serde(default)]
    pub extra_sync_paths: Vec<String>,

    #[serde(default)]
    pub exclusions: Vec<String>,

    #[serde(default)]
    pub overrides: HashMap<String, ModSyncOverride>,
}

impl Default for ModSyncConfig {
    fn default() -> Self {
        Self {
            enforced: true,
            silent: false,
            restart_required: true,
            extra_sync_paths: Vec::new(),
            exclusions: Vec::new(),
            overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClientsConfig {
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub install_dir: PathBuf,
    #[serde(default = "default_restart_policy")]
    pub restart_policy: RestartPolicy,
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,
    #[serde(default = "default_restart_backoff_cap")]
    pub restart_backoff_cap: u64,
    #[serde(default = "default_base_udp_port")]
    pub base_udp_port: u16,
    #[serde(default = "default_headless_image")]
    pub image: String,
    #[serde(default = "default_isolated_paths")]
    pub isolated_paths: Vec<String>,
}

impl Default for ClientsConfig {
    fn default() -> Self {
        Self {
            count: 0,
            install_dir: PathBuf::new(),
            restart_policy: RestartPolicy::Auto,
            max_restart_attempts: 5,
            restart_backoff_cap: 300,
            base_udp_port: 25565,
            image: default_headless_image(),
            isolated_paths: default_isolated_paths(),
        }
    }
}

impl ClientsConfig {
    pub fn validate(&self, config: &Config, spt_dir: &Path) -> Result<()> {
        if self.count == 0 {
            return Ok(());
        }
        if !is_fika_installed(spt_dir) {
            bail!(
                "Fika server mod not found at {}. Dedicated client management requires Fika.",
                spt_dir.join("SPT/user/mods/fika-server").display()
            );
        }
        if self.install_dir.as_os_str().is_empty() || !self.install_dir.exists() {
            bail!(
                "clients.install_dir '{}' does not exist",
                self.install_dir.display()
            );
        }
        let max_port = self.base_udp_port as u32 + self.count - 1;
        if max_port > 65535 {
            bail!(
                "clients.base_udp_port ({}) + count ({}) exceeds port range (max port would be {})",
                self.base_udp_port,
                self.count,
                max_port
            );
        }
        if config.server_container.is_none() {
            bail!("server_container must be configured for dedicated client management — convergence needs to restart the server");
        }
        Ok(())
    }
}

pub fn is_fika_installed(spt_dir: &Path) -> bool {
    spt_dir.join("SPT/user/mods/fika-server").is_dir()
}

#[allow(dead_code)] // Used in modsync.rs and setup.rs (tasks 2-3)
pub const NARCONET_FORGE_MOD_ID: i64 = 2441;

pub fn find_narconet_dir(spt_dir: &Path) -> Option<PathBuf> {
    let mods_dir = spt_dir.join("SPT/user/mods");
    let entries = std::fs::read_dir(&mods_dir).ok()?;
    let mut matches: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().ok().is_some_and(|ft| ft.is_dir()))
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.to_lowercase().contains("narconet"))
        })
        .map(|e| e.path())
        .collect();
    matches.sort_by(|a, b| {
        a.file_name()
            .unwrap()
            .to_ascii_lowercase()
            .cmp(&b.file_name().unwrap().to_ascii_lowercase())
    });
    matches.into_iter().next()
}

pub fn is_modsync_installed(spt_dir: &Path) -> bool {
    find_narconet_dir(spt_dir).is_some()
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

    #[serde(default = "default_auto_start_server")]
    pub auto_start_server: bool,

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

    #[serde(default = "default_update_check_interval")]
    pub update_check_interval: u64,

    #[serde(default = "default_forge_cache_ttl")]
    pub forge_cache_ttl: Option<u64>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clients: Option<ClientsConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modsync: Option<ModSyncConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "LoggingConfig::is_default")]
    pub logging: LoggingConfig,

    #[serde(default = "default_tls_enabled")]
    pub tls_enabled: bool,

    #[serde(default)]
    pub tls_cert: Option<PathBuf>,

    #[serde(default)]
    pub tls_key: Option<PathBuf>,

    #[serde(default = "default_proxy_enabled")]
    pub proxy_enabled: bool,

    #[serde(default = "default_leaderboard_min_raids")]
    pub leaderboard_min_raids: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spt_dir: None,
            forge_token: None,
            queue_changes: true,
            auto_drain_on_lifecycle: false,
            auto_start_server: true,
            session_secret: String::new(),
            server_container: None,
            server_host: None,
            server_port: None,
            web_bind: "0.0.0.0".to_string(),
            web_port: 9190,
            update_check_interval: 300,
            forge_cache_ttl: Some(86400),
            clients: None,
            modsync: None,
            logging: LoggingConfig::default(),
            tls_enabled: true,
            tls_cert: None,
            tls_key: None,
            proxy_enabled: true,
            leaderboard_min_raids: 5,
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
    /// - `QUMA_AUTO_START_SERVER` -> `auto_start_server`
    /// - `QUMA_CLIENTS_COUNT` -> `clients.count`
    /// - `QUMA_CLIENTS_INSTALL_DIR` -> `clients.install_dir`
    /// - `QUMA_CLIENTS_RESTART_POLICY` -> `clients.restart_policy`
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
        if let Ok(val) = std::env::var("QUMA_UPDATE_CHECK_INTERVAL") {
            if let Ok(secs) = val.parse::<u64>() {
                self.update_check_interval = secs;
            }
        }
        if let Ok(val) = std::env::var("QUMA_FORGE_CACHE_TTL") {
            if let Ok(secs) = val.parse::<u64>() {
                tracing::debug!(var = "QUMA_FORGE_CACHE_TTL", value = %val, "env var override applied");
                self.forge_cache_ttl = Some(secs);
            }
        }
        if let Ok(val) = std::env::var("QUMA_AUTO_START_SERVER") {
            if val.eq_ignore_ascii_case("true") {
                tracing::debug!(var = "QUMA_AUTO_START_SERVER", value = %val, "env var override applied");
                self.auto_start_server = true;
            } else if val.eq_ignore_ascii_case("false") {
                tracing::debug!(var = "QUMA_AUTO_START_SERVER", value = %val, "env var override applied");
                self.auto_start_server = false;
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
        if let Ok(val) = std::env::var("QUMA_CLIENTS_COUNT") {
            if let Ok(count) = val.parse::<u32>() {
                self.clients
                    .get_or_insert_with(ClientsConfig::default)
                    .count = count;
            }
        }
        if let Ok(val) = std::env::var("QUMA_CLIENTS_INSTALL_DIR") {
            self.clients
                .get_or_insert_with(ClientsConfig::default)
                .install_dir = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("QUMA_CLIENTS_RESTART_POLICY") {
            if let Ok(policy) = serde_json::from_str::<RestartPolicy>(&format!("\"{val}\"")) {
                self.clients
                    .get_or_insert_with(ClientsConfig::default)
                    .restart_policy = policy;
            }
        }
        if let Ok(val) = std::env::var("QUMA_TLS_ENABLED") {
            if val.eq_ignore_ascii_case("true") {
                self.tls_enabled = true;
            } else if val.eq_ignore_ascii_case("false") {
                self.tls_enabled = false;
            }
        }
        if let Ok(val) = std::env::var("QUMA_TLS_CERT") {
            self.tls_cert = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("QUMA_TLS_KEY") {
            self.tls_key = Some(PathBuf::from(val));
        }
        if let Ok(val) = std::env::var("QUMA_PROXY_ENABLED") {
            if val.eq_ignore_ascii_case("true") {
                self.proxy_enabled = true;
            } else if val.eq_ignore_ascii_case("false") {
                self.proxy_enabled = false;
            }
        }
        if let Ok(val) = std::env::var("QUMA_LEADERBOARD_MIN_RAIDS") {
            if let Ok(v) = val.parse::<u32>() {
                self.leaderboard_min_raids = v;
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
auto_start_server = false
session_secret = "supersecret"
server_container = "spt-server"
server_host = "192.168.1.100"
server_port = 6969
web_bind = "127.0.0.1"
web_port = 8080
update_check_interval = 600
"#;

        let config: Config = toml::from_str(toml_str).expect("should parse full TOML");

        assert_eq!(config.spt_dir, Some(PathBuf::from("/opt/spt")));
        assert_eq!(config.forge_token, Some("tok_abc123".to_string()));
        assert!(!config.queue_changes);
        assert!(config.auto_drain_on_lifecycle);
        assert!(!config.auto_start_server);
        assert_eq!(config.session_secret, "supersecret");
        assert_eq!(config.server_container, Some("spt-server".to_string()));
        assert_eq!(config.server_host, Some("192.168.1.100".to_string()));
        assert_eq!(config.server_port, Some(6969));
        assert_eq!(config.web_bind, "127.0.0.1");
        assert_eq!(config.web_port, 8080);
        assert_eq!(config.update_check_interval, 600);
    }

    #[test]
    fn deserialize_minimal_config() {
        let config: Config = toml::from_str("").expect("should parse empty TOML");

        assert_eq!(config.spt_dir, None);
        assert_eq!(config.forge_token, None);
        assert!(config.queue_changes); // default: true
        assert!(!config.auto_drain_on_lifecycle); // default: false
        assert!(config.auto_start_server); // default: true
        assert_eq!(config.session_secret, "");
        assert_eq!(config.server_container, None);
        assert_eq!(config.server_host, None);
        assert_eq!(config.server_port, None);
        assert_eq!(config.web_bind, "0.0.0.0");
        assert_eq!(config.web_port, 9190);
        assert_eq!(config.update_check_interval, 300);
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
        config.update_check_interval = 120;

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
    fn update_check_interval_default() {
        let config: Config = toml::from_str("").expect("should parse empty TOML");
        assert_eq!(config.update_check_interval, 300);
    }

    #[test]
    fn update_check_interval_custom() {
        let config: Config = toml::from_str("update_check_interval = 60").expect("should parse");
        assert_eq!(config.update_check_interval, 60);
    }

    #[test]
    fn update_check_interval_env_override() {
        temp_env::with_vars([("QUMA_UPDATE_CHECK_INTERVAL", Some("120"))], || {
            let mut config = Config::default();
            config.apply_env_overrides();
            assert_eq!(config.update_check_interval, 120);
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

    #[test]
    fn auto_start_server_env_override() {
        temp_env::with_vars([("QUMA_AUTO_START_SERVER", Some("false"))], || {
            let mut config = Config::default();
            config.apply_env_overrides();
            assert!(!config.auto_start_server);
        });
    }

    #[test]
    fn clients_config_defaults() {
        let config: Config = toml::from_str("").expect("empty config");
        assert!(config.clients.is_none());
    }

    #[test]
    fn clients_config_full_deserialization() {
        let toml_str = r#"
[clients]
count = 3
install_dir = "/opt/fika-client"
restart_policy = "auto"
max_restart_attempts = 10
restart_backoff_cap = 600
base_udp_port = 25565
image = "ghcr.io/zhliau/fika-headless-docker:v2.1.0"
isolated_paths = ["BepInEx/config", "BepInEx/cache"]
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let clients = config.clients.unwrap();
        assert_eq!(clients.count, 3);
        assert_eq!(clients.install_dir, PathBuf::from("/opt/fika-client"));
        assert_eq!(clients.restart_policy, RestartPolicy::Auto);
        assert_eq!(clients.max_restart_attempts, 10);
        assert_eq!(clients.restart_backoff_cap, 600);
        assert_eq!(clients.base_udp_port, 25565);
        assert_eq!(clients.image, "ghcr.io/zhliau/fika-headless-docker:v2.1.0");
        assert_eq!(
            clients.isolated_paths,
            vec!["BepInEx/config", "BepInEx/cache"]
        );
    }

    #[test]
    fn clients_config_minimal_with_defaults() {
        let toml_str = r#"
[clients]
count = 2
install_dir = "/opt/fika"
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let clients = config.clients.unwrap();
        assert_eq!(clients.count, 2);
        assert_eq!(clients.restart_policy, RestartPolicy::Auto);
        assert_eq!(clients.max_restart_attempts, 5);
        assert_eq!(clients.restart_backoff_cap, 300);
        assert_eq!(clients.base_udp_port, 25565);
        assert_eq!(clients.image, "ghcr.io/zhliau/fika-headless-docker:latest");
        assert_eq!(clients.isolated_paths, vec!["BepInEx/config".to_string()]);
    }

    #[test]
    fn clients_config_validation_port_overflow() {
        let clients = ClientsConfig {
            count: 3,
            install_dir: PathBuf::from("/tmp/fake"),
            restart_policy: RestartPolicy::Auto,
            max_restart_attempts: 5,
            restart_backoff_cap: 300,
            base_udp_port: 65534,
            image: "test".to_string(),
            isolated_paths: vec![],
        };
        let config = Config {
            server_container: Some("spt".to_string()),
            ..Config::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        // Create fika-server dir so fika detection passes
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/fika-server")).unwrap();
        // Create install_dir
        std::fs::create_dir_all(&clients.install_dir).ok();

        let result = clients.validate(&config, spt_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("port"));
    }

    #[test]
    fn clients_config_validation_no_fika() {
        let clients = ClientsConfig {
            count: 1,
            install_dir: PathBuf::from("/tmp"),
            restart_policy: RestartPolicy::Auto,
            max_restart_attempts: 5,
            restart_backoff_cap: 300,
            base_udp_port: 25565,
            image: "test".to_string(),
            isolated_paths: vec![],
        };
        let config = Config {
            server_container: Some("spt".to_string()),
            ..Config::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        // No fika-server dir
        let result = clients.validate(&config, tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Fika"));
    }

    #[test]
    fn clients_config_env_override_count() {
        temp_env::with_vars([("QUMA_CLIENTS_COUNT", Some("5"))], || {
            let mut config = Config::default();
            config.clients = Some(ClientsConfig::default());
            config.apply_env_overrides();
            assert_eq!(config.clients.unwrap().count, 5);
        });
    }

    #[test]
    fn fika_detection() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_fika_installed(tmp.path()));
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/fika-server")).unwrap();
        assert!(is_fika_installed(tmp.path()));
    }

    #[test]
    fn modsync_config_defaults() {
        let config: Config = toml::from_str("").expect("empty config");
        assert!(config.modsync.is_none());
    }

    #[test]
    fn narconet_detection_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/some-other-mod")).unwrap();
        assert!(!is_modsync_installed(tmp.path()));
        assert!(find_narconet_dir(tmp.path()).is_none());
    }

    #[test]
    fn narconet_detection_installed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/narconet-server")).unwrap();
        assert!(is_modsync_installed(tmp.path()));
        assert_eq!(
            find_narconet_dir(tmp.path()).unwrap(),
            tmp.path().join("SPT/user/mods/narconet-server")
        );
    }

    #[test]
    fn narconet_detection_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/MadManBeavis-NarcoNet")).unwrap();
        assert!(is_modsync_installed(tmp.path()));
        assert!(find_narconet_dir(tmp.path()).is_some());
    }

    #[test]
    fn narconet_detection_multiple_picks_first_alphabetically() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/narconet-server")).unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/NarcoNet-Debug")).unwrap();
        let found = find_narconet_dir(tmp.path()).unwrap();
        // "NarcoNet-Debug" sorts before "narconet-server" case-insensitively
        assert!(found
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_lowercase()
            .starts_with("narconet"),);
    }

    #[test]
    fn modsync_config_full_deserialization() {
        let toml_str = r#"
[modsync]
enforced = false
silent = true
restart_required = false
extra_sync_paths = ["BepInEx/config", "BepInEx/patchers"]
exclusions = ["**/*.nosync", "BepInEx/plugins/spt"]

[modsync.overrides.12345]
enforced = true
silent = false

[modsync.overrides.67890]
enabled = false
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let ms = config.modsync.unwrap();
        assert!(!ms.enforced);
        assert!(ms.silent);
        assert!(!ms.restart_required);
        assert_eq!(
            ms.extra_sync_paths,
            vec!["BepInEx/config", "BepInEx/patchers"]
        );
        assert_eq!(ms.exclusions, vec!["**/*.nosync", "BepInEx/plugins/spt"]);
        assert_eq!(ms.overrides.len(), 2);

        let o1 = &ms.overrides["12345"];
        assert_eq!(o1.enforced, Some(true));
        assert_eq!(o1.silent, Some(false));
        assert_eq!(o1.restart_required, None);
        assert_eq!(o1.enabled, None);

        let o2 = &ms.overrides["67890"];
        assert_eq!(o2.enabled, Some(false));
        assert_eq!(o2.enforced, None);
    }

    #[test]
    fn modsync_config_minimal_with_defaults() {
        let toml_str = "[modsync]\n";
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let ms = config.modsync.unwrap();
        assert!(ms.enforced); // default: true
        assert!(!ms.silent); // default: false
        assert!(ms.restart_required); // default: true
        assert!(ms.extra_sync_paths.is_empty());
        assert!(ms.exclusions.is_empty());
        assert!(ms.overrides.is_empty());
    }

    #[test]
    fn modsync_config_skip_serializing_when_none() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(
            !serialized.contains("[modsync]"),
            "None modsync should not be serialized"
        );
    }

    #[test]
    fn modsync_config_roundtrip() {
        let mut config = Config::default();
        config.modsync = Some(ModSyncConfig {
            enforced: false,
            silent: true,
            ..ModSyncConfig::default()
        });
        let serialized = toml::to_string_pretty(&config).unwrap();
        let reloaded: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(config.modsync, reloaded.modsync);
    }

    #[test]
    fn modsync_detection() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_modsync_installed(tmp.path()));
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/Corter-ModSync")).unwrap();
        assert!(is_modsync_installed(tmp.path()));
    }

    #[test]
    fn restart_policy_serde() {
        assert_eq!(
            serde_json::from_str::<RestartPolicy>(r#""auto""#).unwrap(),
            RestartPolicy::Auto
        );
        assert_eq!(
            serde_json::from_str::<RestartPolicy>(r#""manual""#).unwrap(),
            RestartPolicy::Manual
        );
    }

    #[test]
    fn tls_config_defaults() {
        let config: Config = toml::from_str("").expect("empty config");
        assert!(config.tls_enabled);
        assert!(config.proxy_enabled);
        assert_eq!(config.tls_cert, None);
        assert_eq!(config.tls_key, None);
    }

    #[test]
    fn tls_config_deserialization() {
        let toml_str = r#"
tls_enabled = false
tls_cert = "/etc/ssl/cert.pem"
tls_key = "/etc/ssl/key.pem"
proxy_enabled = false
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert!(!config.tls_enabled);
        assert_eq!(config.tls_cert, Some(PathBuf::from("/etc/ssl/cert.pem")));
        assert_eq!(config.tls_key, Some(PathBuf::from("/etc/ssl/key.pem")));
        assert!(!config.proxy_enabled);
    }

    #[test]
    fn tls_env_var_overrides() {
        temp_env::with_vars(
            [
                ("QUMA_TLS_ENABLED", Some("false")),
                ("QUMA_TLS_CERT", Some("/env/cert.pem")),
                ("QUMA_TLS_KEY", Some("/env/key.pem")),
                ("QUMA_PROXY_ENABLED", Some("false")),
            ],
            || {
                let mut config = Config::default();
                config.apply_env_overrides();
                assert!(!config.tls_enabled);
                assert_eq!(config.tls_cert, Some(PathBuf::from("/env/cert.pem")));
                assert_eq!(config.tls_key, Some(PathBuf::from("/env/key.pem")));
                assert!(!config.proxy_enabled);
            },
        );
    }

    #[test]
    fn leaderboard_min_raids_default() {
        let config: Config = toml::from_str("").expect("empty config");
        assert_eq!(config.leaderboard_min_raids, 5);
    }

    #[test]
    fn leaderboard_min_raids_custom() {
        let config: Config = toml::from_str("leaderboard_min_raids = 10").expect("should parse");
        assert_eq!(config.leaderboard_min_raids, 10);
    }

    #[test]
    fn leaderboard_min_raids_env_override() {
        temp_env::with_vars([("QUMA_LEADERBOARD_MIN_RAIDS", Some("3"))], || {
            let mut config = Config::default();
            config.apply_env_overrides();
            assert_eq!(config.leaderboard_min_raids, 3);
        });
    }
}
