use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rand::distr::Alphanumeric;
use rand::RngExt;
use serde::{Deserialize, Serialize};

use crate::dirs::QumaDirs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnExit {
    #[default]
    Nothing,
    Stop,
    Remove,
}

impl std::fmt::Display for OnExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnExit::Nothing => write!(f, "nothing"),
            OnExit::Stop => write!(f, "stop"),
            OnExit::Remove => write!(f, "remove"),
        }
    }
}

impl std::str::FromStr for OnExit {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "nothing" => Ok(OnExit::Nothing),
            "stop" => Ok(OnExit::Stop),
            "remove" => Ok(OnExit::Remove),
            _ => bail!("unknown on_exit mode: {s} (expected nothing, stop, or remove)"),
        }
    }
}

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

fn default_proxy_auth() -> bool {
    true
}

fn default_proxy_rewrite_source_port() -> u16 {
    6969
}

fn default_proxy_rewrite_target_port() -> Option<u16> {
    None
}

fn default_proxy_rewrite_http_paths() -> Vec<String> {
    vec![
        "/launcher/server/connect".to_string(),
        "/client/game/config".to_string(),
        "/client/game/mode".to_string(),
    ]
}

fn default_proxy_rewrite_direct_paths() -> Vec<String> {
    vec!["/client/notifier/channel/create".to_string()]
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
    "backups".to_string()
}

fn default_max_backups() -> u32 {
    3
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleFormat {
    #[default]
    Compact,
    #[serde(alias = "text")]
    Full,
    Json,
}

impl std::fmt::Display for ConsoleFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsoleFormat::Compact => write!(f, "compact"),
            ConsoleFormat::Full => write!(f, "full"),
            ConsoleFormat::Json => write!(f, "json"),
        }
    }
}

impl std::str::FromStr for ConsoleFormat {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "compact" => Ok(ConsoleFormat::Compact),
            "full" | "text" => Ok(ConsoleFormat::Full),
            "json" => Ok(ConsoleFormat::Json),
            _ => bail!("unknown console format: {s}"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    Text,
    #[default]
    Json,
}

impl std::fmt::Display for FileFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileFormat::Text => write!(f, "text"),
            FileFormat::Json => write!(f, "json"),
        }
    }
}

impl std::str::FromStr for FileFormat {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "text" => Ok(FileFormat::Text),
            "json" => Ok(FileFormat::Json),
            _ => bail!("unknown file format: {s} (valid: text, json)"),
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

fn default_file_enabled() -> bool {
    true
}

fn default_file_path() -> String {
    "logs/quartermaster.log".to_string()
}

fn default_file_level() -> String {
    "debug".to_string()
}

fn default_rotation() -> RotationPolicy {
    RotationPolicy::Daily
}

fn default_max_size_mb() -> u64 {
    10
}

fn default_max_files() -> usize {
    7
}

fn default_buffer_size() -> usize {
    1000
}

fn default_web_level() -> String {
    "info".to_string()
}

fn default_retention_days() -> u64 {
    7
}

fn default_max_entries() -> u64 {
    100_000
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
    "ghcr.io/cebarks/quartermaster/headless:latest".to_string()
}
fn default_isolated_paths() -> Vec<String> {
    vec!["BepInEx/config".to_string()]
}
fn default_ntsync() -> bool {
    true
}
fn default_server_ready_timeout() -> u64 {
    120
}

fn default_memory_restart_threshold() -> u64 {
    20_000
}

fn default_server_image() -> String {
    crate::container::SPT_SERVER_IMAGE.to_string()
}

fn default_container_stop_timeout() -> u64 {
    10
}

fn default_scanner_guard_enabled() -> bool {
    true
}

fn default_scanner_guard_threshold() -> u32 {
    20
}

fn default_scanner_guard_ban_duration() -> u64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScannerGuardConfig {
    /// Enable scanner guard (ban IPs with excessive consecutive unhandled requests)
    #[serde(default = "default_scanner_guard_enabled")]
    pub enabled: bool,

    /// Number of consecutive 404/405 responses before banning an IP
    #[serde(default = "default_scanner_guard_threshold")]
    pub threshold: u32,

    /// Ban duration in seconds
    #[serde(default = "default_scanner_guard_ban_duration")]
    pub ban_duration: u64,
}

impl Default for ScannerGuardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 20,
            ban_duration: 3600,
        }
    }
}

impl ScannerGuardConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn default_enforced() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_restart_required() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SetupZipConfig {
    #[serde(default = "default_true")]
    pub exclude_server_files: bool,

    #[serde(default = "default_true")]
    pub exclude_non_essential: bool,

    #[serde(default)]
    pub exclude_patterns: Vec<String>,

    #[serde(default)]
    pub include_patterns: Vec<String>,
}

impl Default for SetupZipConfig {
    fn default() -> Self {
        Self {
            exclude_server_files: true,
            exclude_non_essential: true,
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
        }
    }
}

impl SetupZipConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
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
            backup_dir: "backups".to_string(),
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
pub struct ConvoyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default)]
    pub exclusions: Vec<String>,
}

impl Default for ConvoyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            exclusions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSyncConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

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

    #[serde(default, skip_serializing)]
    pub overrides: BTreeMap<String, ModSyncOverride>,

    #[serde(default)]
    pub groups: BTreeMap<String, ModSyncGroup>,
}

impl Default for ModSyncConfig {
    fn default() -> Self {
        Self {
            enabled: true,
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

impl ModSyncConfig {
    // No migration methods - modsync is deprecated, kept only for backward-compatible TOML deserialization
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
    pub image: Option<String>,
    #[serde(default)]
    pub extra_isolated_paths: Vec<String>,
    #[serde(default)]
    pub numa_node: Option<u32>,
    #[serde(default)]
    pub cpuset_cpus: Option<String>,
    #[serde(default)]
    pub cpuset_mems: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeadlessConfig {
    /// Deprecated: headless directory is derived from quma_dir.
    /// For legacy configs, this will be populated from TOML; for new configs, use dirs.headless.
    #[deprecated(note = "use QumaDirs.headless instead")]
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
    #[serde(default = "default_ntsync")]
    pub ntsync: bool,
    #[serde(default)]
    pub esync: bool,
    #[serde(default)]
    pub fsync: bool,
    #[serde(default)]
    pub numa_auto: bool,
    #[serde(default)]
    pub numa_node: Option<u32>,
    #[serde(default)]
    pub numa_pin_memory: bool,
    #[serde(default = "default_server_ready_timeout")]
    pub server_ready_timeout: u64,
    #[serde(default)]
    pub use_upnp: bool,
    #[serde(default)]
    pub physical_cores_only: bool,
    #[serde(default = "default_memory_restart_threshold")]
    pub memory_restart_threshold: u64,
}

impl Default for HeadlessConfig {
    #[allow(deprecated)]
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
            ntsync: default_ntsync(),
            esync: false,
            fsync: false,
            numa_auto: false,
            numa_node: None,
            numa_pin_memory: false,
            server_ready_timeout: 120,
            use_upnp: false,
            physical_cores_only: false,
            memory_restart_threshold: default_memory_restart_threshold(),
        }
    }
}

pub const MAX_HEADLESS_CLIENTS: u32 = 16;

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

    pub fn resolve_image(&self, client_index: usize) -> &str {
        self.clients
            .get(client_index)
            .and_then(|c| c.image.as_deref())
            .unwrap_or(&self.image)
    }

    pub fn validate(&self, config: &Config, dirs: &QumaDirs) -> Result<()> {
        if self.clients.is_empty() {
            return Ok(());
        }
        if self.numa_auto && self.numa_node.is_some() {
            bail!(
                "headless.numa_auto and headless.numa_node are mutually exclusive — \
                 use numa_auto for round-robin or numa_node for a fixed default, not both"
            );
        }

        // Warn if configured NUMA nodes don't exist on this system
        let topology = crate::numa::NumaTopology::detect().unwrap_or_else(|e| {
            tracing::warn!("Failed to detect NUMA topology: {e}");
            crate::numa::NumaTopology::empty()
        });

        if !topology.is_empty() {
            if let Some(node) = self.numa_node {
                if topology.cpuset_for_node(node).is_err() {
                    tracing::warn!(
                        "headless.numa_node = {node} does not match any detected NUMA node (available: {:?})",
                        topology.node_ids()
                    );
                }
            }
            for (i, client) in self.clients.iter().enumerate() {
                if let Some(node) = client.numa_node {
                    if topology.cpuset_for_node(node).is_err() {
                        tracing::warn!(
                            "headless.clients[{i}].numa_node = {node} does not match any detected NUMA node (available: {:?})",
                            topology.node_ids()
                        );
                    }
                }
            }
        }

        if !is_fika_installed(&dirs.spt_server) {
            bail!(
                "Fika server mod not found at {}. Dedicated client management requires Fika.",
                dirs.spt_server.join("SPT/user/mods/fika-server").display()
            );
        }
        // In legacy mode, headless install_dir must not be inside spt_server
        #[allow(deprecated)]
        if dirs.is_legacy() && !self.install_dir.as_os_str().is_empty() {
            // ponytail: canonicalize to handle symlinks/.. — starts_with on raw paths is unreliable
            if let (Ok(canon_install), Ok(canon_spt)) = (
                self.install_dir.canonicalize(),
                dirs.spt_server.canonicalize(),
            ) {
                if canon_install.starts_with(&canon_spt) {
                    bail!(
                        "headless directory ('{}') must not be inside spt_server ('{}') — \
                         the SPT server container mounts spt_server and its entrypoint chowns the \
                         entire tree, which fails on wine-prefix dirs relabeled by headless \
                         containers (SELinux MCS conflict). Move headless directory outside spt_server.",
                        self.install_dir.display(),
                        dirs.spt_server.display()
                    );
                }
            }
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

pub const FIKA_CLIENT_FORGE_ID: i64 = 2326;
pub const FIKA_SERVER_FORGE_ID: i64 = 2357;

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
    #[serde(default)]
    pub format: ConsoleFormat,
}

impl Default for ConsoleLogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format: ConsoleFormat::Compact,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileLogConfig {
    #[serde(default = "default_file_enabled")]
    pub enabled: bool,
    #[serde(default = "default_file_path")]
    pub path: String,
    #[serde(default)]
    pub format: FileFormat,
    #[serde(default = "default_file_level")]
    pub level: String,
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
            enabled: true,
            path: "logs/quartermaster.log".to_string(),
            format: FileFormat::Json,
            level: "debug".to_string(),
            rotation: RotationPolicy::Daily,
            max_size_mb: 10,
            max_files: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebLogConfig {
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    #[serde(default = "default_web_level")]
    pub level: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u64,
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
}

impl Default for WebLogConfig {
    fn default() -> Self {
        Self {
            buffer_size: 1000,
            level: "info".to_string(),
            retention_days: 7,
            max_entries: 100_000,
        }
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
    #[serde(default, alias = "spt_dir")]
    pub quma_dir: Option<PathBuf>,

    #[serde(default = "default_queue_changes")]
    pub queue_changes: bool,

    #[serde(default)]
    pub auto_drain_on_lifecycle: bool,

    #[serde(default = "default_auto_start_server")]
    pub auto_start_server: bool,

    #[serde(default)]
    pub on_exit: OnExit,

    #[serde(default = "default_session_secret")]
    pub session_secret: String,

    #[serde(default)]
    pub server_container: Option<String>,

    #[serde(default)]
    pub server_host: Option<String>,

    #[serde(default)]
    pub server_port: Option<u16>,

    #[serde(default = "default_server_image")]
    pub server_image: String,

    #[serde(default = "default_container_stop_timeout")]
    pub container_stop_timeout: u64,

    #[serde(default = "default_web_bind")]
    pub web_bind: String,

    #[serde(default = "default_web_port")]
    pub web_port: u16,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_workers: Option<usize>,

    #[serde(default = "default_update_check_interval")]
    pub update_check_interval: u64,

    #[serde(default)]
    pub update_disabled_mods: bool,

    #[serde(default = "default_forge_cache_ttl")]
    pub forge_cache_ttl: Option<u64>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headless: Option<HeadlessConfig>,

    #[serde(default)]
    #[serde(skip_serializing)]
    pub modsync: Option<ModSyncConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convoy: Option<ConvoyConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "LoggingConfig::is_default")]
    pub logging: LoggingConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "BackupConfig::is_default")]
    pub backup: BackupConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "SetupZipConfig::is_default")]
    pub setup_zip: SetupZipConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "ScannerGuardConfig::is_default")]
    pub scanner_guard: ScannerGuardConfig,

    #[serde(default = "default_tls_enabled")]
    pub tls_enabled: bool,

    #[serde(default)]
    pub tls_cert: Option<PathBuf>,

    #[serde(default)]
    pub tls_key: Option<PathBuf>,

    #[serde(default = "default_proxy_enabled")]
    pub proxy_enabled: bool,

    #[serde(default = "default_proxy_auth")]
    pub proxy_auth: bool,

    /// Deprecated: the proxy now auto-detects the origin from response bodies.
    /// Retained so existing TOML files with this field still parse.
    #[serde(default = "default_proxy_rewrite_source_port")]
    pub proxy_rewrite_source_port: u16,

    #[serde(default = "default_proxy_rewrite_target_port")]
    pub proxy_rewrite_target_port: Option<u16>,

    #[serde(default = "default_proxy_rewrite_http_paths")]
    pub proxy_rewrite_http_paths: Vec<String>,

    #[serde(default = "default_proxy_rewrite_direct_paths")]
    pub proxy_rewrite_direct_paths: Vec<String>,

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
            quma_dir: None,
            queue_changes: true,
            auto_drain_on_lifecycle: false,
            auto_start_server: true,
            on_exit: OnExit::Nothing,
            session_secret: String::new(),
            server_container: None,
            server_host: None,
            server_port: None,
            server_image: default_server_image(),
            container_stop_timeout: 10,
            web_bind: "0.0.0.0".to_string(),
            web_port: 9190,
            web_workers: None,
            update_check_interval: 300,
            update_disabled_mods: false,
            forge_cache_ttl: Some(86400),
            headless: None,
            modsync: None,
            convoy: None,
            logging: LoggingConfig::default(),
            backup: BackupConfig::default(),
            setup_zip: SetupZipConfig::default(),
            scanner_guard: ScannerGuardConfig::default(),
            tls_enabled: true,
            tls_cert: None,
            tls_key: None,
            proxy_enabled: true,
            proxy_auth: true,
            proxy_rewrite_source_port: 6969,
            proxy_rewrite_target_port: None,
            proxy_rewrite_http_paths: default_proxy_rewrite_http_paths(),
            proxy_rewrite_direct_paths: default_proxy_rewrite_direct_paths(),
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
    /// Load config from a TOML file at `path`. Returns defaults if the file doesn't exist.
    /// Loads config and performs modsync->convoy migration if needed.
    pub fn load(path: &Path) -> Result<Self> {
        tracing::debug!(path = %path.display(), "loading config file");
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let mut config: Config =
                    toml::from_str(&contents).with_context(|| "failed to parse config TOML")?;
                let mut save_needed = false;
                // Migrate [modsync] -> [convoy]
                if config.modsync.is_some() && config.convoy.is_none() {
                    if let Some(ref ms) = config.modsync {
                        if !ms.extra_sync_paths.is_empty() {
                            tracing::warn!(
                                "modsync extra_sync_paths ({}) not migrated to convoy — \
                                 convoy syncs at the mod level, not arbitrary paths",
                                ms.extra_sync_paths.join(", ")
                            );
                        }
                        config.convoy = Some(ConvoyConfig {
                            enabled: ms.enabled,
                            exclusions: ms.exclusions.clone(),
                        });
                        save_needed = true;
                        tracing::info!("migrated [modsync] config section to [convoy]");
                    }
                }
                if save_needed {
                    config.save(path)?;
                }
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "config file not found, using defaults");
                Ok(Config::default())
            }
            Err(e) => Err(anyhow::anyhow!(e)
                .context(format!("failed to read config file: {}", path.display()))),
        }
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
            quma_dir = ?config.quma_dir,
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
        env_override!(opt_path: self.quma_dir, "QUMA_DIR");
        if self.quma_dir.is_none() {
            if let Ok(val) = std::env::var("QUMA_SPT_DIR") {
                tracing::warn!("QUMA_SPT_DIR is deprecated, use QUMA_DIR instead");
                let path = std::path::absolute(PathBuf::from(&val))
                    .unwrap_or_else(|_| PathBuf::from(&val));
                self.quma_dir = Some(path);
            }
        }
        env_override!(str: self.web_bind, "QUMA_WEB_BIND");
        env_override!(parse: self.web_port, "QUMA_WEB_PORT", u16);
        env_override!(opt_parse: self.web_workers, "QUMA_WEB_WORKERS", usize);
        env_override!(opt_str: self.server_container, "QUMA_SERVER_CONTAINER");
        env_override!(opt_str: self.server_host, "QUMA_SERVER_HOST");
        env_override!(opt_parse: self.server_port, "QUMA_SERVER_PORT", u16);
        env_override!(parse: self.container_stop_timeout, "QUMA_CONTAINER_STOP_TIMEOUT", u64);
        env_override!(parse: self.update_check_interval, "QUMA_UPDATE_CHECK_INTERVAL", u64);
        env_override!(opt_parse: self.forge_cache_ttl, "QUMA_FORGE_CACHE_TTL", u64);
        env_override!(bool: self.auto_start_server, "QUMA_AUTO_START_SERVER");
        env_override!(parse: self.on_exit, "QUMA_ON_EXIT", OnExit);
        env_override!(str: self.logging.level, "QUMA_LOG_LEVEL");
        env_override!(str: self.logging.file.path, "QUMA_LOG_FILE_PATH");
        env_override!(bool: self.logging.file.enabled, "QUMA_LOG_FILE_ENABLED");
        env_override!(str: self.logging.file.level, "QUMA_LOG_FILE_LEVEL");
        if let Ok(val) = std::env::var("QUMA_LOG_CONSOLE_FORMAT") {
            if let Ok(fmt) = val.parse::<ConsoleFormat>() {
                self.logging.console.format = fmt;
            }
        }
        env_override!(parse: self.headless.get_or_insert_with(HeadlessConfig::default).restart_policy, "QUMA_HEADLESS_RESTART_POLICY", RestartPolicy);
        if std::env::var("QUMA_HEADLESS_INSTALL_DIR").is_ok() {
            tracing::warn!("QUMA_HEADLESS_INSTALL_DIR is deprecated and ignored — headless directory is derived from quma_dir");
        }
        env_override!(parse: self.headless.get_or_insert_with(HeadlessConfig::default).server_ready_timeout, "QUMA_HEADLESS_SERVER_READY_TIMEOUT", u64);
        env_override!(bool: self.tls_enabled, "QUMA_TLS_ENABLED");
        env_override!(opt_path: self.tls_cert, "QUMA_TLS_CERT");
        env_override!(opt_path: self.tls_key, "QUMA_TLS_KEY");
        env_override!(bool: self.proxy_enabled, "QUMA_PROXY_ENABLED");
        env_override!(bool: self.proxy_auth, "QUMA_PROXY_AUTH");
        env_override!(bool: self.snapshots_enabled, "QUMA_SNAPSHOTS_ENABLED");
        env_override!(parse: self.leaderboard_min_raids, "QUMA_LEADERBOARD_MIN_RAIDS", u32);
        env_override!(bool: self.backup.auto_backup, "QUMA_AUTO_BACKUP");
        env_override!(str: self.backup.backup_dir, "QUMA_BACKUP_DIR");
        env_override!(parse: self.backup.max_backups, "QUMA_MAX_BACKUPS", u32);
        env_override!(bool: self.backup.require_backup, "QUMA_REQUIRE_BACKUP");
        env_override!(opt_str: self.external_url, "QUMA_EXTERNAL_URL");
        env_override!(opt_str: self.server_name, "QUMA_SERVER_NAME");
        env_override!(bool: self.scanner_guard.enabled, "QUMA_SCANNER_GUARD_ENABLED");
        env_override!(parse: self.scanner_guard.threshold, "QUMA_SCANNER_GUARD_THRESHOLD", u32);
        env_override!(parse: self.scanner_guard.ban_duration, "QUMA_SCANNER_GUARD_BAN_DURATION", u64);
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
    /// 3. `<quma_dir>/quartermaster.toml`
    /// 4. `quartermaster.toml` (current directory)
    pub fn resolve_path(cli_config: Option<&Path>, quma_dir: Option<&Path>) -> PathBuf {
        if let Some(path) = cli_config {
            return path.to_path_buf();
        }

        if let Ok(env_path) = std::env::var("QUMA_CONFIG") {
            return PathBuf::from(env_path);
        }

        if let Some(dir) = quma_dir {
            return dir.join("quartermaster.toml");
        }

        PathBuf::from("quartermaster.toml")
    }
}

#[cfg(test)]
#[allow(deprecated)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_full_config() {
        let toml_str = r#"
spt_dir = "/opt/spt"
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

        assert_eq!(config.quma_dir, Some(PathBuf::from("/opt/spt")));
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

        assert_eq!(config.quma_dir, None);
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

        assert_eq!(config.quma_dir, Some(PathBuf::from("/srv/spt")));
        assert_eq!(config.web_port, 3000);
        // Defaults for unspecified fields
        assert!(config.queue_changes);
        assert_eq!(config.web_bind, "0.0.0.0");
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("nonexistent.toml");

        let config = Config::load(&config_path).expect("should return defaults for missing file");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn save_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("nested/dir/quartermaster.toml");

        let mut config = Config::default();
        config.quma_dir = Some(PathBuf::from("/opt/game"));
        config.web_port = 7777;
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
                ("QUMA_WEB_PORT", Some("4000")),
                ("QUMA_WEB_BIND", Some("10.0.0.1")),
                ("QUMA_SERVER_CONTAINER", Some("env-container")),
                ("QUMA_SERVER_HOST", Some("env-host")),
                ("QUMA_SERVER_PORT", Some("6970")),
            ],
            || {
                let mut config = Config::default();
                config.apply_env_overrides();

                assert_eq!(config.quma_dir, Some(PathBuf::from("/env/spt")));
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
        assert_eq!(config.logging.console.format, ConsoleFormat::Compact);
        assert!(config.logging.file.enabled);
        assert_eq!(config.logging.file.path, "logs/quartermaster.log");
        assert_eq!(config.logging.file.format, FileFormat::Json);
        assert_eq!(config.logging.file.level, "debug");
        assert_eq!(config.logging.file.rotation, RotationPolicy::Daily);
        assert_eq!(config.logging.file.max_size_mb, 10);
        assert_eq!(config.logging.file.max_files, 7);
        assert_eq!(config.logging.web.buffer_size, 1000);
        assert_eq!(config.logging.web.level, "info");
        assert_eq!(config.logging.web.retention_days, 7);
        assert_eq!(config.logging.web.max_entries, 100_000);
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
level = "trace"
rotation = "size"
max_size_mb = 50
max_files = 10

[logging.web]
buffer_size = 5000
level = "warn"
retention_days = 14
max_entries = 200000
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.logging.level, "debug");
        assert!(!config.logging.console.enabled);
        assert_eq!(config.logging.console.format, ConsoleFormat::Json);
        assert!(config.logging.file.enabled);
        assert_eq!(config.logging.file.path, "/var/log/quma.log");
        assert_eq!(config.logging.file.format, FileFormat::Text);
        assert_eq!(config.logging.file.level, "trace");
        assert_eq!(config.logging.file.rotation, RotationPolicy::Size);
        assert_eq!(config.logging.file.max_size_mb, 50);
        assert_eq!(config.logging.file.max_files, 10);
        assert_eq!(config.logging.web.buffer_size, 5000);
        assert_eq!(config.logging.web.level, "warn");
        assert_eq!(config.logging.web.retention_days, 14);
        assert_eq!(config.logging.web.max_entries, 200_000);
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
ntsync = false
esync = true
fsync = false
server_ready_timeout = 300
physical_cores_only = true

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
        assert!(!headless.ntsync);
        assert!(headless.esync);
        assert!(!headless.fsync);
        assert!(headless.physical_cores_only);
        assert_eq!(headless.server_ready_timeout, 300);
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
        assert_eq!(
            headless.image,
            "ghcr.io/cebarks/quartermaster/headless:latest"
        );
        assert_eq!(headless.isolated_paths, vec!["BepInEx/config".to_string()]);
        assert_eq!(headless.server_ready_timeout, 120);
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
    fn headless_config_new_field_defaults() {
        let config: HeadlessConfig = HeadlessConfig::default();
        assert!(config.ntsync);
        assert!(!config.esync);
        assert!(!config.fsync);
        assert!(!config.physical_cores_only);
        assert_eq!(config.server_ready_timeout, 120);
    }

    #[test]
    fn headless_effective_isolated_paths_merges() {
        let headless = HeadlessConfig {
            isolated_paths: vec!["BepInEx/config".to_string()],
            clients: vec![
                HeadlessClientDef {
                    extra_isolated_paths: vec![],
                    ..Default::default()
                },
                HeadlessClientDef {
                    extra_isolated_paths: vec!["BepInEx/cache".to_string()],
                    ..Default::default()
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
        let install_tmp = tempfile::tempdir().unwrap();
        let install_dir = install_tmp.path().to_path_buf();
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
        let dirs = QumaDirs::from_legacy(spt_dir.to_path_buf());
        let result = headless.validate(&config, &dirs);
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
        let dirs = QumaDirs::from_legacy(tmp.path().to_path_buf());
        let result = headless.validate(&config, &dirs);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Fika"));
    }

    #[test]
    fn headless_config_validation_install_dir_inside_spt_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/fika-server")).unwrap();
        let install_dir = spt_dir.join("headless");
        std::fs::create_dir_all(&install_dir).unwrap();
        let headless = HeadlessConfig {
            install_dir,
            clients: vec![HeadlessClientDef::default()],
            ..HeadlessConfig::default()
        };
        let config = Config {
            server_container: Some("spt".to_string()),
            ..Config::default()
        };
        let dirs = QumaDirs::from_legacy(spt_dir.to_path_buf());
        let result = headless.validate(&config, &dirs);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must not be inside"));
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
        assert_eq!(config.backup.backup_dir, "backups");
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
    fn on_exit_default_is_nothing() {
        let config: Config = toml::from_str("").expect("empty config");
        assert_eq!(config.on_exit, OnExit::Nothing);
    }

    #[test]
    fn on_exit_deserialize_all_variants() {
        let stop: Config = toml::from_str(r#"on_exit = "stop""#).expect("stop");
        assert_eq!(stop.on_exit, OnExit::Stop);

        let remove: Config = toml::from_str(r#"on_exit = "remove""#).expect("remove");
        assert_eq!(remove.on_exit, OnExit::Remove);

        let nothing: Config = toml::from_str(r#"on_exit = "nothing""#).expect("nothing");
        assert_eq!(nothing.on_exit, OnExit::Nothing);
    }

    #[test]
    fn on_exit_roundtrip() {
        let mut config = Config::default();
        config.on_exit = OnExit::Stop;

        let serialized = toml::to_string_pretty(&config).expect("serialize");
        let reloaded: Config = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(reloaded.on_exit, OnExit::Stop);
    }

    #[test]
    fn on_exit_env_override() {
        temp_env::with_vars([("QUMA_ON_EXIT", Some("remove"))], || {
            let mut config = Config::default();
            config.apply_env_overrides();
            assert_eq!(config.on_exit, OnExit::Remove);
        });
    }

    #[test]
    fn on_exit_from_str() {
        assert_eq!("nothing".parse::<OnExit>().unwrap(), OnExit::Nothing);
        assert_eq!("stop".parse::<OnExit>().unwrap(), OnExit::Stop);
        assert_eq!("remove".parse::<OnExit>().unwrap(), OnExit::Remove);
        assert!("invalid".parse::<OnExit>().is_err());
    }

    #[test]
    fn config_load_does_not_rewrite_when_no_migration_needed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("quartermaster.toml");

        let toml_content = r#"
[modsync]
extra_sync_paths = ["BepInEx/config"]

[modsync.groups.no-headless]
display_name = "No Headless"
members = []
exclude_headless = true

[convoy]
enabled = true
"#;
        std::fs::write(&config_path, toml_content).expect("write");
        let mtime_before = std::fs::metadata(&config_path).unwrap().modified().unwrap();

        // Small sleep to ensure mtime would differ if file were rewritten
        std::thread::sleep(std::time::Duration::from_millis(50));

        let _config = Config::load(&config_path).expect("should load");
        let mtime_after = std::fs::metadata(&config_path).unwrap().modified().unwrap();

        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn proxy_auth_defaults_true() {
        let config: Config = toml::from_str("").expect("empty config");
        assert!(config.proxy_auth);
    }

    #[test]
    fn proxy_auth_disabled_via_toml() {
        let config: Config = toml::from_str("proxy_auth = false").expect("should parse");
        assert!(!config.proxy_auth);
    }

    #[test]
    fn proxy_auth_env_override() {
        temp_env::with_vars([("QUMA_PROXY_AUTH", Some("false"))], || {
            let mut config = Config::default();
            config.apply_env_overrides();
            assert!(!config.proxy_auth);
        });
    }
}

#[cfg(test)]
mod logging_config_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn console_format_round_trips() {
        assert_eq!(
            "compact".parse::<ConsoleFormat>().unwrap(),
            ConsoleFormat::Compact
        );
        assert_eq!(
            "full".parse::<ConsoleFormat>().unwrap(),
            ConsoleFormat::Full
        );
        assert_eq!(
            "json".parse::<ConsoleFormat>().unwrap(),
            ConsoleFormat::Json
        );
        assert_eq!(ConsoleFormat::Compact.to_string(), "compact");
    }

    #[test]
    fn file_format_round_trips() {
        assert_eq!("text".parse::<FileFormat>().unwrap(), FileFormat::Text);
        assert_eq!("json".parse::<FileFormat>().unwrap(), FileFormat::Json);
        assert!(FileFormat::from_str("compact").is_err());
    }

    #[test]
    fn console_format_rejects_invalid() {
        assert!("compact".parse::<FileFormat>().is_err());
    }

    #[test]
    fn new_defaults_correct() {
        let config = LoggingConfig::default();
        assert_eq!(config.console.format, ConsoleFormat::Compact);
        assert_eq!(config.file.path, "logs/quartermaster.log");
        assert_eq!(config.file.rotation, RotationPolicy::Daily);
        assert_eq!(config.file.max_files, 7);
        assert_eq!(config.file.level, "debug");
        assert_eq!(config.web.level, "info");
        assert_eq!(config.web.retention_days, 7);
        assert_eq!(config.web.max_entries, 100_000);
    }

    #[test]
    fn legacy_text_format_deserializes_to_full() {
        let toml_str = r#"
[logging]
level = "info"

[logging.console]
enabled = true
format = "text"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.logging.console.format, ConsoleFormat::Full);
    }

    #[test]
    fn setup_zip_config_defaults() {
        let config: Config = toml::from_str("").expect("empty config");
        assert!(config.setup_zip.exclude_server_files);
        assert!(config.setup_zip.exclude_non_essential);
        assert!(config.setup_zip.exclude_patterns.is_empty());
        assert!(config.setup_zip.include_patterns.is_empty());
    }

    #[test]
    fn setup_zip_config_deserialization() {
        let toml_str = r#"
[setup_zip]
exclude_server_files = false
exclude_non_essential = false
exclude_patterns = ["**/*.pdb", "BepInEx/plugins/debug/**"]
include_patterns = ["user/mods/special/**"]
"#;
        let config: Config = toml::from_str(toml_str).expect("should parse");
        assert!(!config.setup_zip.exclude_server_files);
        assert!(!config.setup_zip.exclude_non_essential);
        assert_eq!(
            config.setup_zip.exclude_patterns,
            vec!["**/*.pdb", "BepInEx/plugins/debug/**"]
        );
        assert_eq!(
            config.setup_zip.include_patterns,
            vec!["user/mods/special/**"]
        );
    }

    #[test]
    fn setup_zip_config_skip_serializing_when_default() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(
            !serialized.contains("[setup_zip]"),
            "default setup_zip should not be serialized"
        );
    }

    #[test]
    fn setup_zip_config_roundtrip() {
        let mut config = Config::default();
        config.setup_zip.exclude_server_files = false;
        config.setup_zip.exclude_patterns = vec!["**/*.pdb".to_string()];
        let serialized = toml::to_string_pretty(&config).unwrap();
        let reloaded: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(config.setup_zip, reloaded.setup_zip);
    }

    #[test]
    fn headless_numa_node_parses() {
        let toml_str = r#"
[headless]
install_dir = "/opt/client"
numa_node = 2

[[headless.clients]]
extra_isolated_paths = []

[[headless.clients]]
numa_node = 3
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let h = config.headless.unwrap();
        assert_eq!(h.numa_node, Some(2));
        assert!(!h.numa_auto);
        assert_eq!(h.clients[0].numa_node, None);
        assert_eq!(h.clients[1].numa_node, Some(3));
    }

    #[test]
    fn headless_numa_auto_parses() {
        let toml_str = r#"
[headless]
install_dir = "/opt/client"
numa_auto = true

[[headless.clients]]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let h = config.headless.unwrap();
        assert!(h.numa_auto);
        assert_eq!(h.numa_node, None);
    }

    #[test]
    fn headless_client_cpuset_override_parses() {
        let toml_str = r#"
[headless]
install_dir = "/opt/client"

[[headless.clients]]
cpuset_cpus = "0-7,16-23"
cpuset_mems = "0"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let h = config.headless.unwrap();
        assert_eq!(h.clients[0].cpuset_cpus, Some("0-7,16-23".to_string()));
        assert_eq!(h.clients[0].cpuset_mems, Some("0".to_string()));
    }

    #[test]
    #[allow(deprecated)]
    fn headless_numa_auto_and_node_rejects() {
        let toml_str = r#"
[headless]
install_dir = "/tmp/test-client"
numa_auto = true
numa_node = 1

[[headless.clients]]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let h = config.headless.unwrap();
        let full_config = Config {
            headless: Some(h.clone()),
            server_container: Some("spt".to_string()),
            ..Default::default()
        };
        // Create a temp dir for install_dir so other validations pass
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("client");
        std::fs::create_dir_all(&install_dir).unwrap();
        // Create fika-server dir
        let spt_dir = tmp.path().join("spt");
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/fika-server")).unwrap();

        let mut h_mut = h;
        h_mut.install_dir = install_dir;
        let dirs = QumaDirs::from_legacy(spt_dir);
        let result = h_mut.validate(&full_config, &dirs);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("numa_auto"));
    }

    #[test]
    fn headless_config_without_numa_parses_unchanged() {
        let toml_str = r#"
[headless]
install_dir = "/opt/client"

[[headless.clients]]
extra_isolated_paths = ["BepInEx/plugins"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let h = config.headless.unwrap();
        assert_eq!(h.numa_node, None);
        assert!(!h.numa_auto);
        assert_eq!(h.clients[0].numa_node, None);
        assert_eq!(h.clients[0].cpuset_cpus, None);
        assert_eq!(h.clients[0].cpuset_mems, None);
    }

    #[test]
    fn headless_validate_warns_on_unknown_numa_node() {
        // This test verifies the validation doesn't error on unknown nodes
        // (it warns, but we can't easily capture tracing warns in a unit test).
        // Instead, verify validate() succeeds when a valid numa_node is set.
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path().join("spt");
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/fika-server")).unwrap();
        let install_dir = tmp.path().join("client");
        std::fs::create_dir_all(&install_dir).unwrap();

        let h = HeadlessConfig {
            install_dir,
            clients: vec![HeadlessClientDef::default()],
            numa_node: Some(99), // nonexistent node — should warn, not error
            ..Default::default()
        };
        let config = Config {
            server_container: Some("spt".to_string()),
            ..Default::default()
        };
        // validate() should succeed (warn, not bail)
        let dirs = QumaDirs::from_legacy(spt_dir);
        assert!(h.validate(&config, &dirs).is_ok());
    }

    #[test]
    fn resolve_image_per_client_override() {
        let config = HeadlessConfig {
            image: "global:latest".to_string(),
            clients: vec![
                HeadlessClientDef {
                    image: Some("custom:v1".to_string()),
                    ..Default::default()
                },
                HeadlessClientDef::default(),
            ],
            ..Default::default()
        };
        assert_eq!(config.resolve_image(0), "custom:v1");
        assert_eq!(config.resolve_image(1), "global:latest");
        assert_eq!(config.resolve_image(99), "global:latest");
    }
}
