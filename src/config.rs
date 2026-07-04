use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rand::distr::Alphanumeric;
use rand::RngExt;
use serde::{Deserialize, Serialize};

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
    "quartermaster/backups".to_string()
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
    "localhost/fika-headless:latest".to_string()
}
fn default_isolated_paths() -> Vec<String> {
    vec!["BepInEx/config".to_string()]
}
fn default_ntsync() -> bool {
    true
}
fn default_save_log_on_exit() -> bool {
    true
}
fn default_overwrite_fika() -> bool {
    true
}
fn default_server_ready_timeout() -> u64 {
    120
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
    /// Migrate deprecated path formats in `extra_sync_paths` and `exclusions`
    /// to gameroot-relative format. Returns `true` if any paths were changed.
    ///
    /// Deprecated formats:
    /// - `../BepInEx/plugins` → `BepInEx/plugins` (strip `../` prefix)
    /// - `user/mods` → `SPT/user/mods` (SPT-relative → gameroot-relative)
    pub fn migrate_deprecated_paths(&mut self) -> bool {
        let mut changed = false;
        for path in &mut self.extra_sync_paths {
            if let Some(stripped) = path.strip_prefix("../") {
                *path = stripped.to_string();
                changed = true;
            } else if path.starts_with("user/") || *path == "user" {
                *path = format!("SPT/{path}");
                changed = true;
            }
        }
        for path in &mut self.exclusions {
            if let Some(stripped) = path.strip_prefix("../") {
                *path = stripped.to_string();
                changed = true;
            } else if path.starts_with("user/") || *path == "user" {
                *path = format!("SPT/{path}");
                changed = true;
            }
        }
        changed
    }

    /// Migrate per-mod overrides into groups. Each override becomes a
    /// single-member group. If the mod already belongs to a group, the
    /// override is merged (single-member) or the mod is split out
    /// (multi-member). Returns `true` if any migration occurred.
    pub fn migrate_overrides_to_groups(&mut self) -> bool {
        if self.overrides.is_empty() {
            return false;
        }

        // Build reverse lookup: forge_mod_id → group slug
        let mod_to_group: std::collections::HashMap<i64, String> = self
            .groups
            .iter()
            .flat_map(|(slug, g)| g.members.iter().map(move |&id| (id, slug.clone())))
            .collect();

        let overrides = std::mem::take(&mut self.overrides);
        for (forge_id_str, ovr) in overrides {
            let forge_id: i64 = match forge_id_str.parse() {
                Ok(id) => id,
                Err(_) => continue,
            };

            if let Some(group_slug) = mod_to_group.get(&forge_id) {
                let group = match self.groups.get_mut(group_slug) {
                    Some(g) => g,
                    None => continue,
                };

                if group.members.len() == 1 {
                    // Single-member group: merge override values directly
                    if ovr.enabled.is_some() {
                        group.enabled = ovr.enabled;
                    }
                    if ovr.enforced.is_some() {
                        group.enforced = ovr.enforced;
                    }
                    if ovr.silent.is_some() {
                        group.silent = ovr.silent;
                    }
                    if ovr.restart_required.is_some() {
                        group.restart_required = ovr.restart_required;
                    }
                } else {
                    // Multi-member group: split this mod out into its own group
                    group.members.retain(|&id| id != forge_id);
                    let mut new_group = ModSyncGroup {
                        display_name: format!("Mod #{forge_id}"),
                        members: vec![forge_id],
                        enabled: group.enabled,
                        enforced: group.enforced,
                        silent: group.silent,
                        restart_required: group.restart_required,
                        exclude_headless: group.exclude_headless,
                    };
                    // Apply override on top
                    if ovr.enabled.is_some() {
                        new_group.enabled = ovr.enabled;
                    }
                    if ovr.enforced.is_some() {
                        new_group.enforced = ovr.enforced;
                    }
                    if ovr.silent.is_some() {
                        new_group.silent = ovr.silent;
                    }
                    if ovr.restart_required.is_some() {
                        new_group.restart_required = ovr.restart_required;
                    }
                    self.groups
                        .insert(format!("override-{forge_id}"), new_group);
                }
            } else {
                // Mod not in any group: create a new single-member group
                let group = ModSyncGroup {
                    display_name: format!("Mod #{forge_id}"),
                    members: vec![forge_id],
                    enabled: ovr.enabled,
                    enforced: ovr.enforced,
                    silent: ovr.silent,
                    restart_required: ovr.restart_required,
                    exclude_headless: false,
                };
                self.groups.insert(format!("override-{forge_id}"), group);
            }
        }

        true
    }

    /// Ensure predefined groups exist with correct invariants.
    /// Seeds `no-headless` if missing; forces `exclude_headless = true` on it
    /// if hand-edited. Returns `true` if any changes were made.
    pub fn ensure_predefined_groups(&mut self) -> bool {
        let mut changed = false;
        if let Some(nh) = self.groups.get_mut("no-headless") {
            if nh.display_name != "No Headless" {
                nh.display_name = "No Headless".to_string();
                changed = true;
            }
            if !nh.exclude_headless {
                nh.exclude_headless = true;
                changed = true;
            }
        } else {
            self.groups.insert(
                "no-headless".to_string(),
                ModSyncGroup {
                    display_name: "No Headless".to_string(),
                    members: Vec::new(),
                    enabled: None,
                    enforced: None,
                    silent: None,
                    restart_required: None,
                    exclude_headless: true,
                },
            );
            changed = true;
        }
        changed
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
    if slug == "default" {
        bail!("\"default\" is a reserved group slug");
    }
    if slug == "no-headless" {
        bail!("\"no-headless\" is a reserved group slug");
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
    #[serde(default)]
    pub numa_node: Option<u32>,
    #[serde(default)]
    pub cpuset_cpus: Option<String>,
    #[serde(default)]
    pub cpuset_mems: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessRunner {
    #[default]
    Umu,
    Wine,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessDisplayServer {
    #[default]
    Gamescope,
    Xvfb,
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
    #[serde(default)]
    pub runner: HeadlessRunner,
    #[serde(default = "default_ntsync")]
    pub ntsync: bool,
    #[serde(default)]
    pub esync: bool,
    #[serde(default)]
    pub fsync: bool,
    #[serde(default)]
    pub display_server: HeadlessDisplayServer,
    #[serde(default = "default_save_log_on_exit")]
    pub save_log_on_exit: bool,
    #[serde(default)]
    pub enable_log_purge: bool,
    #[serde(default = "default_overwrite_fika")]
    pub overwrite_fika: bool,
    #[serde(default)]
    pub numa_auto: bool,
    #[serde(default)]
    pub numa_node: Option<u32>,
    #[serde(default = "default_server_ready_timeout")]
    pub server_ready_timeout: u64,
    #[serde(default)]
    pub use_upnp: bool,
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
            runner: HeadlessRunner::default(),
            ntsync: default_ntsync(),
            esync: false,
            fsync: false,
            display_server: HeadlessDisplayServer::default(),
            save_log_on_exit: default_save_log_on_exit(),
            enable_log_purge: false,
            overwrite_fika: default_overwrite_fika(),
            numa_auto: false,
            numa_node: None,
            server_ready_timeout: 120,
            use_upnp: false,
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
        // ponytail: canonicalize to handle symlinks/.. — starts_with on raw paths is unreliable
        if let (Ok(canon_install), Ok(canon_spt)) =
            (self.install_dir.canonicalize(), spt_dir.canonicalize())
        {
            if canon_install.starts_with(&canon_spt) {
                bail!(
                    "headless.install_dir ('{}') must not be inside spt_dir ('{}') — \
                     the SPT server container mounts spt_dir and its entrypoint chowns the \
                     entire tree, which fails on wine-prefix dirs relabeled by headless \
                     containers (SELinux MCS conflict). Move install_dir outside spt_dir.",
                    self.install_dir.display(),
                    spt_dir.display()
                );
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modsync: Option<ModSyncConfig>,

    #[serde(default)]
    #[serde(skip_serializing_if = "LoggingConfig::is_default")]
    pub logging: LoggingConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "BackupConfig::is_default")]
    pub backup: BackupConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "SetupZipConfig::is_default")]
    pub setup_zip: SetupZipConfig,

    #[serde(default = "default_tls_enabled")]
    pub tls_enabled: bool,

    #[serde(default)]
    pub tls_cert: Option<PathBuf>,

    #[serde(default)]
    pub tls_key: Option<PathBuf>,

    #[serde(default = "default_proxy_enabled")]
    pub proxy_enabled: bool,

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
            spt_dir: None,
            forge_token: None,
            queue_changes: true,
            auto_drain_on_lifecycle: false,
            auto_start_server: true,
            on_exit: OnExit::Nothing,
            session_secret: String::new(),
            server_container: None,
            server_host: None,
            server_port: None,
            web_bind: "0.0.0.0".to_string(),
            web_port: 9190,
            web_workers: None,
            update_check_interval: 300,
            update_disabled_mods: false,
            forge_cache_ttl: Some(86400),
            headless: None,
            modsync: None,
            logging: LoggingConfig::default(),
            backup: BackupConfig::default(),
            setup_zip: SetupZipConfig::default(),
            tls_enabled: true,
            tls_cert: None,
            tls_key: None,
            proxy_enabled: true,
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
    /// Automatically migrates deprecated modsync path formats and saves back if needed.
    pub fn load(path: &Path) -> Result<Self> {
        tracing::debug!(path = %path.display(), "loading config file");
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let mut config: Config =
                    toml::from_str(&contents).with_context(|| "failed to parse config TOML")?;
                let mut save_needed = false;
                if let Some(ref mut ms) = config.modsync {
                    if ms.migrate_deprecated_paths() {
                        tracing::warn!(
                            "NarcoNet config contained deprecated path formats — \
                             migrated to gameroot-relative format and saved to {}",
                            path.display()
                        );
                        save_needed = true;
                    }
                    if ms.migrate_overrides_to_groups() {
                        tracing::warn!(
                            "migrated per-mod NarcoNet overrides to groups — \
                             overrides are deprecated, saved to {}",
                            path.display()
                        );
                        save_needed = true;
                    }
                    if ms.groups.remove("default").is_some() {
                        tracing::info!(
                            "removed reserved \"default\" group from modsync config — \
                             ungrouped mods now use global settings automatically"
                        );
                        save_needed = true;
                    }
                    if ms.ensure_predefined_groups() {
                        tracing::info!("seeded predefined modsync groups");
                        save_needed = true;
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
        env_override!(opt_parse: self.web_workers, "QUMA_WEB_WORKERS", usize);
        env_override!(opt_str: self.server_container, "QUMA_SERVER_CONTAINER");
        env_override!(opt_str: self.server_host, "QUMA_SERVER_HOST");
        env_override!(opt_parse: self.server_port, "QUMA_SERVER_PORT", u16);
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
        env_override!(path: self.headless.get_or_insert_with(HeadlessConfig::default).install_dir, "QUMA_HEADLESS_INSTALL_DIR");
        env_override!(parse: self.headless.get_or_insert_with(HeadlessConfig::default).server_ready_timeout, "QUMA_HEADLESS_SERVER_READY_TIMEOUT", u64);
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
runner = "wine"
ntsync = false
esync = true
fsync = false
display_server = "xvfb"
save_log_on_exit = false
enable_log_purge = true
overwrite_fika = false
server_ready_timeout = 300

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
        assert_eq!(headless.runner, HeadlessRunner::Wine);
        assert!(!headless.ntsync);
        assert!(headless.esync);
        assert!(!headless.fsync);
        assert_eq!(headless.display_server, HeadlessDisplayServer::Xvfb);
        assert!(!headless.save_log_on_exit);
        assert!(headless.enable_log_purge);
        assert!(!headless.overwrite_fika);
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
        assert_eq!(headless.image, "localhost/fika-headless:latest");
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
        assert_eq!(config.runner, HeadlessRunner::Umu);
        assert!(config.ntsync);
        assert!(!config.esync);
        assert!(!config.fsync);
        assert_eq!(config.display_server, HeadlessDisplayServer::Gamescope);
        assert!(config.save_log_on_exit);
        assert!(!config.enable_log_purge);
        assert!(config.overwrite_fika);
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
        let result = headless.validate(&config, spt_dir);
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
        assert!(validate_group_slug("default").is_err());
    }

    #[test]
    fn validate_group_slug_rejects_no_headless() {
        assert!(validate_group_slug("no-headless").is_err());
    }

    #[test]
    fn ensure_predefined_groups_seeds_no_headless() {
        let mut ms = ModSyncConfig::default();
        assert!(!ms.groups.contains_key("no-headless"));
        assert!(ms.ensure_predefined_groups());
        assert!(ms.groups.contains_key("no-headless"));
        let nh = &ms.groups["no-headless"];
        assert_eq!(nh.display_name, "No Headless");
        assert!(nh.members.is_empty());
        assert!(nh.exclude_headless);
        assert_eq!(nh.enabled, None);
        assert_eq!(nh.enforced, None);
        assert_eq!(nh.silent, None);
        assert_eq!(nh.restart_required, None);
    }

    #[test]
    fn ensure_predefined_groups_preserves_existing_no_headless() {
        let mut ms = ModSyncConfig::default();
        ms.groups.insert(
            "no-headless".to_string(),
            ModSyncGroup {
                display_name: "No Headless".to_string(),
                members: vec![100, 200],
                enabled: Some(false),
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: true,
            },
        );
        assert!(!ms.ensure_predefined_groups());
        assert_eq!(ms.groups["no-headless"].members, vec![100, 200]);
        assert_eq!(ms.groups["no-headless"].enabled, Some(false));
    }

    #[test]
    fn ensure_predefined_groups_forces_exclude_headless_true() {
        let mut ms = ModSyncConfig::default();
        ms.groups.insert(
            "no-headless".to_string(),
            ModSyncGroup {
                display_name: "No Headless".to_string(),
                members: vec![100],
                enabled: None,
                enforced: None,
                silent: None,
                restart_required: None,
                exclude_headless: false, // hand-edited to false
            },
        );
        assert!(ms.ensure_predefined_groups());
        assert!(ms.groups["no-headless"].exclude_headless);
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
    fn modsync_migrate_strips_dotdot_prefix() {
        let mut ms = ModSyncConfig {
            extra_sync_paths: vec!["../BepInEx/plugins".to_string()],
            exclusions: vec!["../BepInEx/plugins/spt".to_string()],
            ..ModSyncConfig::default()
        };
        assert!(ms.migrate_deprecated_paths());
        assert_eq!(ms.extra_sync_paths, vec!["BepInEx/plugins"]);
        assert_eq!(ms.exclusions, vec!["BepInEx/plugins/spt"]);
    }

    #[test]
    fn modsync_migrate_converts_user_to_spt_user() {
        let mut ms = ModSyncConfig {
            extra_sync_paths: vec!["user/mods".to_string()],
            ..ModSyncConfig::default()
        };
        assert!(ms.migrate_deprecated_paths());
        assert_eq!(ms.extra_sync_paths, vec!["SPT/user/mods"]);
    }

    #[test]
    fn modsync_migrate_converts_bare_user() {
        let mut ms = ModSyncConfig {
            exclusions: vec!["user".to_string()],
            ..ModSyncConfig::default()
        };
        assert!(ms.migrate_deprecated_paths());
        assert_eq!(ms.exclusions, vec!["SPT/user"]);
    }

    #[test]
    fn modsync_migrate_leaves_gameroot_relative_unchanged() {
        let mut ms = ModSyncConfig {
            extra_sync_paths: vec!["BepInEx/plugins".to_string()],
            exclusions: vec!["**/*.nosync".to_string(), "SPT/user/mods".to_string()],
            ..ModSyncConfig::default()
        };
        assert!(!ms.migrate_deprecated_paths());
        assert_eq!(ms.extra_sync_paths, vec!["BepInEx/plugins"]);
        assert_eq!(ms.exclusions, vec!["**/*.nosync", "SPT/user/mods"]);
    }

    #[test]
    fn modsync_migrate_mixed_paths() {
        let mut ms = ModSyncConfig {
            extra_sync_paths: vec![
                "../BepInEx/config".to_string(),
                "BepInEx/plugins".to_string(),
                "user/mods".to_string(),
            ],
            ..ModSyncConfig::default()
        };
        assert!(ms.migrate_deprecated_paths());
        assert_eq!(
            ms.extra_sync_paths,
            vec!["BepInEx/config", "BepInEx/plugins", "SPT/user/mods"]
        );
    }

    #[test]
    fn config_load_auto_migrates_modsync_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("quartermaster.toml");

        let toml_content = r#"
[modsync]
extra_sync_paths = ["../BepInEx/config", "user/mods"]
exclusions = ["../BepInEx/plugins/spt"]
"#;
        std::fs::write(&config_path, toml_content).expect("write");

        let config = Config::load(&config_path).expect("should load and migrate");
        let ms = config.modsync.unwrap();
        assert_eq!(ms.extra_sync_paths, vec!["BepInEx/config", "SPT/user/mods"]);
        assert_eq!(ms.exclusions, vec!["BepInEx/plugins/spt"]);

        // Verify the file was updated on disk
        let reloaded: Config =
            toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        let ms2 = reloaded.modsync.unwrap();
        assert_eq!(
            ms2.extra_sync_paths,
            vec!["BepInEx/config", "SPT/user/mods"]
        );
        assert_eq!(ms2.exclusions, vec!["BepInEx/plugins/spt"]);
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
"#;
        std::fs::write(&config_path, toml_content).expect("write");
        let mtime_before = std::fs::metadata(&config_path).unwrap().modified().unwrap();

        // Small sleep to ensure mtime would differ if file were rewritten
        std::thread::sleep(std::time::Duration::from_millis(50));

        let _config = Config::load(&config_path).expect("should load");
        let mtime_after = std::fs::metadata(&config_path).unwrap().modified().unwrap();

        assert_eq!(mtime_before, mtime_after);
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
    fn migrate_overrides_no_overrides_returns_false() {
        let mut ms = ModSyncConfig::default();
        assert!(!ms.migrate_overrides_to_groups());
    }

    #[test]
    fn migrate_overrides_standalone_override_creates_group() {
        let mut ms = ModSyncConfig::default();
        ms.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(false),
                silent: Some(true),
                restart_required: None,
                enabled: None,
            },
        );

        assert!(ms.migrate_overrides_to_groups());
        assert!(ms.overrides.is_empty());
        assert_eq!(ms.groups.len(), 1);

        let group = &ms.groups["override-100"];
        assert_eq!(group.display_name, "Mod #100");
        assert_eq!(group.members, vec![100]);
        assert_eq!(group.enforced, Some(false));
        assert_eq!(group.silent, Some(true));
        assert_eq!(group.restart_required, None);
        assert_eq!(group.enabled, None);
    }

    #[test]
    fn migrate_overrides_single_member_group_merges() {
        let mut ms = ModSyncConfig::default();
        ms.groups.insert(
            "grp".to_string(),
            ModSyncGroup {
                display_name: "Group".to_string(),
                members: vec![100],
                enabled: None,
                enforced: Some(true),
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );
        ms.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(false),
                silent: None,
                restart_required: None,
                enabled: None,
            },
        );

        assert!(ms.migrate_overrides_to_groups());
        assert!(ms.overrides.is_empty());
        assert_eq!(ms.groups.len(), 1);
        assert_eq!(ms.groups["grp"].enforced, Some(false));
    }

    #[test]
    fn migrate_overrides_multi_member_group_splits() {
        let mut ms = ModSyncConfig::default();
        ms.groups.insert(
            "grp".to_string(),
            ModSyncGroup {
                display_name: "Group".to_string(),
                members: vec![100, 200],
                enabled: None,
                enforced: Some(true),
                silent: None,
                restart_required: None,
                exclude_headless: false,
            },
        );
        ms.overrides.insert(
            "100".to_string(),
            ModSyncOverride {
                enforced: Some(false),
                silent: None,
                restart_required: None,
                enabled: None,
            },
        );

        assert!(ms.migrate_overrides_to_groups());
        assert!(ms.overrides.is_empty());
        assert_eq!(ms.groups.len(), 2);

        // Original group keeps member 200 only
        assert_eq!(ms.groups["grp"].members, vec![200]);
        assert_eq!(ms.groups["grp"].enforced, Some(true));

        // New group has member 100 with merged flags
        let new_grp = &ms.groups["override-100"];
        assert_eq!(new_grp.members, vec![100]);
        assert_eq!(new_grp.enforced, Some(false)); // override wins
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
        let result = h_mut.validate(&full_config, &spt_dir);
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
        assert!(h.validate(&config, &spt_dir).is_ok());
    }
}
