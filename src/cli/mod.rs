use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

pub mod apply;
pub mod backup;
pub mod check;
pub mod common;
pub mod generate;
pub mod headless;
pub mod install;
pub mod invite;
pub mod list;
pub mod migrate;
pub mod reindex;
pub mod remove;
pub mod restore;
pub mod serve;
pub mod server;
pub mod setup;
pub mod spt;
pub mod status;
pub mod update;

#[derive(Parser)]
#[command(name = "quma", version, about = "Quartermaster — SPT/Fika mod manager")]
pub struct Cli {
    /// Explicit Quartermaster data directory
    #[arg(long, global = true)]
    pub quma_dir: Option<PathBuf>,

    /// Deprecated: use --quma-dir instead
    #[arg(long, global = true, hide = true)]
    pub spt_dir: Option<PathBuf>,

    /// Config file path override
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Increase verbosity (-v for debug, -vv for trace)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Set log level explicitly (trace, debug, info, warn, error)
    #[arg(long, global = true)]
    pub log_level: Option<String>,

    /// Console log format (compact, full, json)
    #[arg(long, global = true)]
    pub log_format: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn effective_quma_dir(&self) -> Option<&Path> {
        self.quma_dir.as_deref().or_else(|| {
            if self.spt_dir.is_some() {
                tracing::warn!("--spt-dir is deprecated, use --quma-dir instead");
            }
            self.spt_dir.as_deref()
        })
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Bootstrap or initialize Quartermaster for an SPT server
    Setup {
        /// Quartermaster data directory (default: ~/spt-server)
        #[arg(long)]
        quma_dir: Option<PathBuf>,
        /// Deprecated: use --quma-dir instead
        #[arg(long, hide = true)]
        path: Option<PathBuf>,
        /// Skip Fika installation
        #[arg(long)]
        no_fika: bool,
        /// Set admin password non-interactively (min 8 chars)
        #[arg(long)]
        admin_password: Option<String>,
        /// Use a separate container name for development (won't collide with production)
        #[arg(long)]
        dev: bool,
        /// Override the container name (useful for parallel dev environments)
        #[arg(long)]
        container_name: Option<String>,
        /// SPT server version to install (prompts if omitted)
        #[arg(long)]
        spt_version: Option<String>,
    },

    /// Install a mod and its dependencies
    Install {
        /// Mod name, Forge ID, slug, URL, or file path
        mod_ref: String,
        /// Specific version (latest compatible if omitted)
        version: Option<String>,
        /// Override the mod name (used with URL/file installs)
        #[arg(long)]
        name: Option<String>,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
        /// Operate on a Forge addon instead of a mod
        #[arg(long)]
        addon: bool,
    },

    /// Update installed mods
    Update {
        /// Specific mod to update (all if omitted)
        mod_ref: Option<String>,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
        /// Operate on a Forge addon instead of a mod
        #[arg(long)]
        addon: bool,
    },

    /// Remove an installed mod
    Remove {
        /// Mod name, Forge ID, or slug
        mod_ref: String,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
        /// Operate on a Forge addon instead of a mod
        #[arg(long)]
        addon: bool,
    },

    /// List installed mods
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Check all installed mods for updates
    Check,

    /// Run health checks against SPT server and mod integrity
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage the SPT server container
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },

    /// Manage Fika headless clients
    Headless {
        #[command(subcommand)]
        action: headless::HeadlessAction,
    },

    /// Manage the SPT server installation
    Spt {
        #[command(subcommand)]
        action: spt::SptAction,
    },

    /// Start the Quartermaster web UI
    Serve {
        /// Bind address
        #[arg(long)]
        bind: Option<String>,
        /// Port number
        #[arg(long)]
        port: Option<u16>,
    },

    /// Generate configuration files
    Generate {
        #[command(subcommand)]
        target: GenerateTarget,
    },

    /// Generate an invite code for a player
    Invite {
        /// Expiry duration (e.g. 24h, 7d)
        #[arg(long)]
        expires: Option<String>,
    },

    /// Backup mods, profiles, and config
    Backup {
        /// Specific mod to backup (full snapshot if omitted)
        mod_ref: Option<String>,
        /// List existing backups instead of creating one
        #[arg(long)]
        list: bool,
    },

    /// Rebuild file tracking index by re-downloading archives from Forge
    Reindex {
        /// Actually apply changes (dry-run by default)
        #[arg(long)]
        apply: bool,
    },

    /// Restore from a backup
    Restore {
        /// Backup ID (numeric) to restore
        backup_id: Option<i64>,
        /// Restore the latest backup for a specific mod
        #[arg(long)]
        latest: Option<String>,
        /// Skip confirmation prompt
        #[arg(long, short)]
        force: bool,
    },

    /// Migrate from legacy directory layout to new layout
    Migrate {
        #[arg(long, help = "Show migration plan without making changes")]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Start the SPT server container
    Start {
        /// Ping timeout in seconds
        #[arg(long, default_value = "60")]
        timeout: u64,
    },
    /// Stop the SPT server container
    Stop,
    /// Restart the SPT server container
    Restart {
        /// Force drain queue regardless of config
        #[arg(long)]
        drain: bool,
        /// Skip queue drain regardless of config
        #[arg(long)]
        skip_queue: bool,
    },
    /// Tail container logs
    Logs {
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
}

#[derive(Subcommand)]
pub enum GenerateTarget {
    /// Emit a systemd service file for `quma serve`
    Systemd {
        /// Write directly to /etc/systemd/system/ and enable
        #[arg(long)]
        install: bool,
    },
}
