use std::collections::BTreeMap;
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

fn default_snapshots_enabled() -> bool {
    true
}

fn default_leaderboard_min_raids() -> u32 {
    5
}

fn default_auto_backup() -> bool {
    true
}

fn default_backup_dir() -> String {
    "quartermaster/backups".to_string()
}

fn default_max_backups() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    Text,
    Json,
}

impl std::fmt::Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogFormat::Text => write!(f, "text"),
            LogFormat::Json => write!(f, "json"),
        }
    }
}

impl std::str::FromStr for LogFormat {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "text" => Ok(LogFormat::Text),
            "json" => Ok(LogFormat::Json),
            _ => bail!("unknown log format: {s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RotationPolicy {
    None,
    Size,
    Daily,
}

impl std::fmt::Display for RotationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RotationPolicy::None => write!(f, "none"),
            RotationPolicy::Size => write!(f, "size"),
            RotationPolicy::Daily => write!(f, "daily"),
        }
    }
}

impl std::str::FromStr for RotationPolicy {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "none" => Ok(RotationPolicy::None),
            "size" => Ok(RotationPolicy::Size),
            "daily" => Ok(RotationPolicy::Daily),
            _ => bail!("unknown rotation policy: {s}"),
        }
    }
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

impl std::fmt::Display for RestartPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestartPolicy::Auto => write!(f, "auto"),
            RestartPolicy::Manual => write!(f, "manual"),
        }
    }
}

impl std::str::FromStr for RestartPolicy {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(RestartPolicy::Auto),
            "manual" => Ok(RestartPolicy::Manual),
            _ => bail!("unknown restart policy: {s}"),
        }
    }
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
pub struct BackupConfig {
    #[serde(default = "default_auto_backup")]
    pub auto_backup: bool,

    #[serde(default = "default_backup_dir")]
    pub backup_dir: String,

    #[serde(default = "default_max_backups")]
    pub max_backups: u32,

    #[serde(default)]
    pub require_backup: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            auto_backup: true,
            backup_dir: "quartermaster/backups".to_string(),
            max_backups: 3,
            require_backup: false,
        }
    }
}

impl BackupConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
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
    pub overrides: BTreeMap<String, ModSyncOverride>,

    #[serde(default)]
    pub groups: BTreeMap<String, ModSyncGroup>,
}

impl Default for ModSyncConfig {
    fn default() -> Self {
        Self {
            enforced: true,
            silent: false,
            restart_required: true,
            extra_sync_paths: Vec::new(),
            exclusions: Vec::new(),
            overrides: BTreeMap::new(),
            groups: BTreeMap::new(),
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

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncGroup {
    pub display_name: String,
    pub members: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforced: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclude_headless: bool,
}

pub fn validate_group_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        bail!("Group slug cannot be empty");
    }
    if slug.len() > 64 {
        bail!("Group slug too long (max 64 characters)");
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        bail!("Group slug cannot start or end with a hyphen");
    }
    for ch in slug.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '-' {
            bail!(
                "Group slug contains invalid character: '{}' (only a-z, 0-9, - allowed)",
                ch
            );
        }
    }
    Ok(())
}

pub fn slugify(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // Collapse consecutive hyphens, trim leading/trailing
    let mut result = String::new();
    let mut prev_hyphen = true; // treat start as hyphen to skip leading
    for ch in slug.chars() {
        if ch == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(ch);
            prev_hyphen = false;
        }
    }
    result.truncate(64);
    result.trim_end_matches('-').to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HeadlessClientDef {
    #[serde(default)]
    pub extra_isolated_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeadlessConfig {
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
    #[serde(default)]
    pub clients: Vec<HeadlessClientDef>,
}

impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            install_dir: PathBuf::new(),
            restart_policy: RestartPolicy::Auto,
            max_restart_attempts: 5,
            restart_backoff_cap: 300,
            base_udp_port: 25565,
            image: default_headless_image(),
            isolated_paths: default_isolated_paths(),
            clients: Vec::new(),
        }
    }
}

impl HeadlessConfig {
    pub fn client_count(&self) -> u32 {
        self.clients.len() as u32
    }

    pub fn effective_isolated_paths(&self, index: usize) -> Vec<String> {
        let mut paths = self.isolated_paths.clone();
        if let Some(client) = self.clients.get(index) {
            paths.extend(client.extra_isolated_paths.clone());
        }
        paths
    }

    pub fn validate(&self, config: &Config, spt_dir: &Path) -> Result<()> {
        if self.clients.is_empty() {
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
                "headless.install_dir '{}' does not exist",
                self.install_dir.display()
            );
        }
        let count = self.client_count();
        match (self.base_udp_port as u32).checked_add(count - 1) {
            Some(max_port) if max_port > 65535 => {
                bail!(
                    "headless.base_udp_port ({}) + client count ({}) exceeds port range (max port would be {})",
                    self.base_udp_port, count, max_port
                );
            }
            None => {
                bail!(
                    "headless.base_udp_port ({}) + client count ({}) exceeds port range",
                    self.base_udp_port,
                    count
                );
            }
            _ => {}
        }
        if config.server_container.is_none() {
            bail!("server_container must be configured for headless client management");
        }
        Ok(())
    }
}

pub fn is_fika_installed(spt_dir: &Path) -> bool {
    spt_dir.join("SPT/user/mods/fika-server").is_dir()
}

pub const NARCONET_FORGE_MOD_ID: i64 = 2441;
pub const FIKA_CLIENT_FORGE_ID: i64 = 2326;
pub const FIKA_SERVER_FORGE_ID: i64 = 2357;

pub fn find_narconet_dir(spt_dir: &Path) -> Option<PathBuf> {
    let mods_dir = spt_dir.join("SPT/user/mods");
    let entries = std::fs::read_dir(&mods_dir).ok()?;
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().ok().is_some_and(|ft| ft.is_dir()))
        .filter(|e| {
            let name_matches = e
                .file_name()
                .to_str()
                .is_some_and(|n| n.to_lowercase().contains("narconet"));
            name_matches && e.path().join("package.json").is_file()
        })
        .min_by(|a, b| {
            a.file_name()
                .to_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(
                    &b.file_name()
                        .to_str()
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                )
        })
        .map(|e| e.path())
}

pub fn is_modsync_installed(spt_dir: &Path) -> bool {
    find_narconet_dir(spt_dir).is_some()
}

pub fn find_svm_dir(spt_dir: &Path) -> Option<PathBuf> {
    let mods_dir = spt_dir.join("SPT/user/mods");
    let entries = std::fs::read_dir(&mods_dir).ok()?;
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().ok().is_some_and(|ft| ft.is_dir()))
        .filter(|e| {
            let name_matches = e
                .file_name()
                .to_str()
                .is_some_and(|n| n.to_lowercase().contains("svm"));
            name_matches && e.path().join("Loader/loader.json").is_file()
        })
        .min_by(|a, b| {
            a.file_name()
                .to_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(
                    &b.file_name()
                        .to_str()
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                )
        })
        .map(|e| e.path())
}

#[allow(dead_code)]
pub fn is_svm_installed(spt_dir: &Path) -> bool {
    find_svm_dir(spt_dir).is_some()
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
    pub headless: Option<HeadlessConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modsync: Option<ModSyncConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "LoggingConfig::is_default")]
    pub logging: LoggingConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "BackupConfig::is_default")]
    pub backup: BackupConfig,

    #[serde(default = "default_tls_enabled")]
    pub tls_enabled: bool,

    #[serde(default)]
    pub tls_cert: Option<PathBuf>,

    #[serde(default)]
    pub tls_key: Option<PathBuf>,

    #[serde(default = "default_proxy_enabled")]
    pub proxy_enabled: bool,

    #[serde(default = "default_snapshots_enabled")]
    pub snapshots_enabled: bool,

    #[serde(default = "default_leaderboard_min_raids")]
    pub leaderboard_min_raids: u32,

    #[serde(default)]
    pub external_url: Option<String>,

    #[serde(default)]
    pub server_name: Option<String>,
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
            headless: None,
            modsync: None,
            logging: LoggingConfig::default(),
            backup: BackupConfig::default(),
            tls_enabled: true,
            tls_cert: None,
            tls_key: None,
            proxy_enabled: true,
            snapshots_enabled: true,
            leaderboard_min_raids: 5,
            external_url: None,
            server_name: None,
        }
    }
}

macro_rules! env_override {
    (str: $field:expr, $var:literal) => {
        if let Ok(val) = std::env::var($var) {
            tracing::debug!(var = $var, value = %val, "env var override applied");
            $field = val;
        }
    };
    (opt_str: $field:expr, $var:literal) => {
        if let Ok(val) = std::env::var($var) {
            tracing::debug!(var = $var, value = %val, "env var override applied");
            $field = Some(val);
        }
    };
    (path: $field:expr, $var:literal) => {
        if let Ok(val) = std::env::var($var) {
            let path = std::path::absolute(PathBuf::from(&val)).unwrap_or_else(|_| PathBuf::from(&val));
            tracing::debug!(var = $var, value = %path.display(), "env var override applied");
            $field = path;
        }
    };
    (opt_path: $field:expr, $var:literal) => {
        if let Ok(val) = std::env::var($var) {
            let path = std::path::absolute(PathBuf::from(&val)).unwrap_or_else(|_| PathBuf::from(&val));
            tracing::debug!(var = $var, value = %path.display(), "env var override applied");
            $field = Some(path);
        }
    };
    (parse: $field:expr, $var:literal, $ty:ty) => {
        if let Ok(val) = std::env::var($var) {
            if let Ok(parsed) = val.parse::<$ty>() {
                tracing::debug!(var = $var, value = %val, "env var override applied");
                $field = parsed;
            }
        }
    };
    (opt_parse: $field:expr, $var:literal, $ty:ty) => {
        if let Ok(val) = std::env::var($var) {
            if let Ok(parsed) = val.parse::<$ty>() {
                tracing::debug!(var = $var, value = %val, "env var override applied");
                $field = Some(parsed);
            }
        }
    };
    (bool: $field:expr, $var:literal) => {
        if let Ok(val) = std::env::var($var) {
            if val.eq_ignore_ascii_case("true") {
                tracing::debug!(var = $var, value = %val, "env var override applied");
                $field = true;
            } else if val.eq_ignore_ascii_case("false") {
                tracing::debug!(var = $var, value = %val, "env var override applied");
                $field = false;
            }
        }
    };
    (redacted: $field:expr, $var:literal) => {
        if let Ok(val) = std::env::var($var) {
            tracing::debug!(var = $var, value = "<redacted>", "env var override applied");
            $field = Some(val);
        }
    };
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

    /// Override config fields from `QUMA_*` environment variables.
    pub fn apply_env_overrides(&mut self) {
        env_override!(opt_path: self.spt_dir, "QUMA_SPT_DIR");
        env_override!(redacted: self.forge_token, "QUMA_FORGE_TOKEN");
        env_override!(str: self.web_bind, "QUMA_WEB_BIND");
        env_override!(parse: self.web_port, "QUMA_WEB_PORT", u16);
        env_override!(opt_str: self.server_container, "QUMA_SERVER_CONTAINER");
        env_override!(opt_str: self.server_host, "QUMA_SERVER_HOST");
        env_override!(opt_parse: self.server_port, "QUMA_SERVER_PORT", u16);
        env_override!(parse: self.update_check_interval, "QUMA_UPDATE_CHECK_INTERVAL", u64);
        env_override!(opt_parse: self.forge_cache_ttl, "QUMA_FORGE_CACHE_TTL", u64);
        env_override!(bool: self.auto_start_server, "QUMA_AUTO_START_SERVER");
        env_override!(str: self.logging.level, "QUMA_LOG_LEVEL");
        env_override!(str: self.logging.file.path, "QUMA_LOG_FILE_PATH");
        env_override!(bool: self.logging.file.enabled, "QUMA_LOG_FILE_ENABLED");
        env_override!(parse: self.headless.get_or_insert_with(HeadlessConfig::default).restart_policy, "QUMA_HEADLESS_RESTART_POLICY", RestartPolicy);
        env_override!(path: self.headless.get_or_insert_with(HeadlessConfig::default).install_dir, "QUMA_HEADLESS_INSTALL_DIR");
        env_override!(bool: self.tls_enabled, "QUMA_TLS_ENABLED");
        env_override!(opt_path: self.tls_cert, "QUMA_TLS_CERT");
        env_override!(opt_path: self.tls_key, "QUMA_TLS_KEY");
        env_override!(bool: self.proxy_enabled, "QUMA_PROXY_ENABLED");
        env_override!(bool: self.snapshots_enabled, "QUMA_SNAPSHOTS_ENABLED");
        env_override!(parse: self.leaderboard_min_raids, "QUMA_LEADERBOARD_MIN_RAIDS", u32);
        env_override!(bool: self.backup.auto_backup, "QUMA_AUTO_BACKUP");
        env_override!(str: self.backup.backup_dir, "QUMA_BACKUP_DIR");
        env_override!(parse: self.backup.max_backups, "QUMA_MAX_BACKUPS", u32);
        env_override!(bool: self.backup.require_backup, "QUMA_REQUIRE_BACKUP");
        env_override!(opt_str: self.external_url, "QUMA_EXTERNAL_URL");
        env_override!(opt_str: self.server_name, "QUMA_SERVER_NAME");
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
#[allow(clippy::unwrap_used)]
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
    fn headless_config_full_deserialization() {
        let toml_str = r#"
[headless]
install_dir = "/opt/fika-client"
restart_policy = "auto"
max_restart_attempts = 10
restart_backoff_cap = 600
base_udp_port = 25565
image = "ghcr.io/zhliau/fika-headless-docker:v2.1.0"
isolated_paths = ["BepInEx/config", "BepInEx/cache"]

[[headless.clients]]

[[headless.clients]]
extra_isolated_paths = ["BepInEx/plugins/testing"]
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let headless = config.headless.unwrap();
        assert_eq!(headless.install_dir, PathBuf::from("/opt/fika-client"));
        assert_eq!(headless.restart_policy, RestartPolicy::Auto);
        assert_eq!(headless.max_restart_attempts, 10);
        assert_eq!(headless.restart_backoff_cap, 600);
        assert_eq!(headless.base_udp_port, 25565);
        assert_eq!(headless.image, "ghcr.io/zhliau/fika-headless-docker:v2.1.0");
        assert_eq!(
            headless.isolated_paths,
            vec!["BepInEx/config", "BepInEx/cache"]
        );
        assert_eq!(headless.clients.len(), 2);
        assert!(headless.clients[0].extra_isolated_paths.is_empty());
        assert_eq!(
            headless.clients[1].extra_isolated_paths,
            vec!["BepInEx/plugins/testing"]
        );
    }

    #[test]
    fn headless_config_minimal_with_defaults() {
        let toml_str = r#"
[headless]
install_dir = "/opt/fika"

[[headless.clients]]
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let headless = config.headless.unwrap();
        assert_eq!(headless.client_count(), 1);
        assert_eq!(headless.restart_policy, RestartPolicy::Auto);
        assert_eq!(headless.max_restart_attempts, 5);
        assert_eq!(headless.restart_backoff_cap, 300);
        assert_eq!(headless.base_udp_port, 25565);
        assert_eq!(headless.image, "ghcr.io/zhliau/fika-headless-docker:latest");
        assert_eq!(headless.isolated_paths, vec!["BepInEx/config".to_string()]);
    }

    #[test]
    fn headless_config_no_clients_defined() {
        let toml_str = r#"
[headless]
install_dir = "/opt/fika"
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        let headless = config.headless.unwrap();
        assert_eq!(headless.client_count(), 0);
        assert!(headless.clients.is_empty());
    }

    #[test]
    fn headless_effective_isolated_paths_merges() {
        let headless = HeadlessConfig {
            isolated_paths: vec!["BepInEx/config".to_string()],
            clients: vec![
                HeadlessClientDef {
                    extra_isolated_paths: vec![],
                },
                HeadlessClientDef {
                    extra_isolated_paths: vec!["BepInEx/cache".to_string()],
                },
            ],
            ..HeadlessConfig::default()
        };
        assert_eq!(
            headless.effective_isolated_paths(0),
            vec!["BepInEx/config".to_string()]
        );
        assert_eq!(
            headless.effective_isolated_paths(1),
            vec!["BepInEx/config".to_string(), "BepInEx/cache".to_string()]
        );
    }

    #[test]
    fn headless_config_validation_port_overflow() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/fika-server")).unwrap();
        // Create install_dir so validation reaches the port check
        let install_dir = tmp.path().join("fika-install");
        std::fs::create_dir_all(&install_dir).unwrap();
        let headless = HeadlessConfig {
            install_dir,
            base_udp_port: 65534,
            clients: vec![
                HeadlessClientDef::default(),
                HeadlessClientDef::default(),
                HeadlessClientDef::default(),
            ],
            ..HeadlessConfig::default()
        };
        let config = Config {
            server_container: Some("spt".to_string()),
            ..Config::default()
        };
        let result = headless.validate(&config, spt_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("port"));
    }

    #[test]
    fn headless_config_validation_no_fika() {
        let headless = HeadlessConfig {
            install_dir: PathBuf::from("/tmp"),
            clients: vec![HeadlessClientDef::default()],
            ..HeadlessConfig::default()
        };
        let config = Config {
            server_container: Some("spt".to_string()),
            ..Config::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let result = headless.validate(&config, tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Fika"));
    }

    #[test]
    fn headless_config_defaults() {
        let config: Config = toml::from_str("").expect("empty config");
        assert!(config.headless.is_none());
    }

    #[test]
    fn headless_config_skip_serializing_when_none() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(
            !serialized.contains("[headless]"),
            "None headless should not be serialized"
        );
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
        let other_mod = tmp.path().join("SPT/user/mods/some-other-mod");
        std::fs::create_dir_all(&other_mod).unwrap();
        // Create package.json but directory name doesn't match "narconet"
        std::fs::write(other_mod.join("package.json"), "{}").unwrap();
        assert!(!is_modsync_installed(tmp.path()));
        assert!(find_narconet_dir(tmp.path()).is_none());
    }

    #[test]
    fn narconet_detection_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let narconet_dir = tmp.path().join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();
        assert!(is_modsync_installed(tmp.path()));
        assert_eq!(
            find_narconet_dir(tmp.path()).unwrap(),
            tmp.path().join("SPT/user/mods/narconet-server")
        );
    }

    #[test]
    fn narconet_detection_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let narconet_dir = tmp.path().join("SPT/user/mods/MadManBeavis-NarcoNet");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();
        assert!(is_modsync_installed(tmp.path()));
        assert!(find_narconet_dir(tmp.path()).is_some());
    }

    #[test]
    fn narconet_detection_multiple_picks_first_alphabetically() {
        let tmp = tempfile::tempdir().unwrap();
        let narconet_server = tmp.path().join("SPT/user/mods/narconet-server");
        let narconet_debug = tmp.path().join("SPT/user/mods/NarcoNet-Debug");
        std::fs::create_dir_all(&narconet_server).unwrap();
        std::fs::create_dir_all(&narconet_debug).unwrap();
        std::fs::write(narconet_server.join("package.json"), "{}").unwrap();
        std::fs::write(narconet_debug.join("package.json"), "{}").unwrap();
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
    fn svm_detection_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods/some-other-mod")).unwrap();
        assert!(find_svm_dir(tmp.path()).is_none());
    }

    #[test]
    fn svm_detection_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let svm_dir = tmp.path().join("SPT/user/mods/[SVM] Server Value Modifier");
        std::fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        std::fs::write(svm_dir.join("Loader/loader.json"), "{}").unwrap();
        assert_eq!(
            find_svm_dir(tmp.path()).unwrap(),
            tmp.path().join("SPT/user/mods/[SVM] Server Value Modifier")
        );
    }

    #[test]
    fn svm_detection_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let svm_dir = tmp.path().join("SPT/user/mods/[svm] servervaluemodifier");
        std::fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        std::fs::write(svm_dir.join("Loader/loader.json"), "{}").unwrap();
        assert!(find_svm_dir(tmp.path()).is_some());
    }

    #[test]
    fn svm_detection_requires_loader_json() {
        let tmp = tempfile::tempdir().unwrap();
        let svm_dir = tmp.path().join("SPT/user/mods/[SVM] Server Value Modifier");
        std::fs::create_dir_all(&svm_dir).unwrap();
        // No Loader/loader.json
        assert!(find_svm_dir(tmp.path()).is_none());
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
        let narconet_dir = tmp.path().join("SPT/user/mods/narconet-server");
        std::fs::create_dir_all(&narconet_dir).unwrap();
        std::fs::write(narconet_dir.join("package.json"), "{}").unwrap();
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

    #[test]
    fn backup_config_defaults() {
        let config = Config::default();
        assert!(config.backup.auto_backup);
        assert_eq!(config.backup.backup_dir, "quartermaster/backups");
        assert_eq!(config.backup.max_backups, 3);
        assert!(!config.backup.require_backup);
    }

    #[test]
    fn backup_config_deserialization() {
        let toml_str = r#"
    [backup]
    auto_backup = false
    backup_dir = "custom/backups"
    max_backups = 5
    require_backup = true
    "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.backup.auto_backup);
        assert_eq!(config.backup.backup_dir, "custom/backups");
        assert_eq!(config.backup.max_backups, 5);
        assert!(config.backup.require_backup);
    }

    #[test]
    fn backup_config_skip_serializing_when_default() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(!serialized.contains("[backup]"));
    }

    #[test]
    fn modsync_group_serde_roundtrip() {
        let group = ModSyncGroup {
            display_name: "Optional Mods".to_string(),
            members: vec![100, 205],
            enabled: Some(false),
            enforced: None,
            silent: None,
            restart_required: None,
            exclude_headless: false,
        };
        let toml_str = toml::to_string_pretty(&group).unwrap();
        // exclude_headless=false should NOT appear in output
        assert!(!toml_str.contains("exclude_headless"));
        // enabled=false SHOULD appear
        assert!(toml_str.contains("enabled = false"));
        let parsed: ModSyncGroup = toml::from_str(&toml_str).unwrap();
        assert_eq!(group, parsed);
    }

    #[test]
    fn modsync_group_exclude_headless_true_serialized() {
        let group = ModSyncGroup {
            display_name: "No Headless".to_string(),
            members: vec![150],
            enabled: None,
            enforced: None,
            silent: None,
            restart_required: None,
            exclude_headless: true,
        };
        let toml_str = toml::to_string_pretty(&group).unwrap();
        assert!(toml_str.contains("exclude_headless = true"));
    }

    #[test]
    fn modsync_config_with_groups_roundtrip() {
        let toml_input = r#"
[modsync]
enforced = true

[modsync.groups.optional]
display_name = "Optional Mods"
members = [100, 205]
enabled = false

[modsync.groups.no-headless]
display_name = "No Headless"
members = [150]
exclude_headless = true

[modsync.overrides.100]
silent = true
"#;
        let config: Config = toml::from_str(toml_input).unwrap();
        let ms = config.modsync.unwrap();
        assert_eq!(ms.groups.len(), 2);
        assert_eq!(ms.groups["optional"].display_name, "Optional Mods");
        assert_eq!(ms.groups["optional"].members, vec![100, 205]);
        assert_eq!(ms.groups["optional"].enabled, Some(false));
        assert!(!ms.groups["optional"].exclude_headless);
        assert!(ms.groups["no-headless"].exclude_headless);
        assert!(ms.overrides.contains_key("100"));
    }

    #[test]
    fn validate_group_slug_accepts_valid() {
        assert!(validate_group_slug("optional").is_ok());
        assert!(validate_group_slug("no-headless").is_ok());
        assert!(validate_group_slug("group-2").is_ok());
        assert!(validate_group_slug("a").is_ok());
    }

    #[test]
    fn validate_group_slug_rejects_invalid() {
        assert!(validate_group_slug("").is_err());
        assert!(validate_group_slug("-leading").is_err());
        assert!(validate_group_slug("trailing-").is_err());
        assert!(validate_group_slug("UPPER").is_err());
        assert!(validate_group_slug("has space").is_err());
        assert!(validate_group_slug("has.dot").is_err());
        assert!(validate_group_slug(&"a".repeat(65)).is_err());
    }
}
