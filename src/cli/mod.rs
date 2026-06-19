use std::path::PathBuf;

use clap::{Parser, Subcommand};

pub mod apply;
pub mod check;
pub mod common;
pub mod config_cmd;
pub mod generate;
pub mod init;
pub mod install;
pub mod invite;
pub mod list;
pub mod remove;
pub mod serve;
pub mod server;
pub mod setup;
pub mod status;
pub mod track;
pub mod update;

#[derive(Parser)]
#[command(name = "quma", version, about = "Quartermaster — SPT/Fika mod manager")]
pub struct Cli {
    /// Explicit SPT server directory
    #[arg(long, global = true)]
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

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Interactive guided setup for Fika multiplayer
    Setup {
        /// Accept all defaults, skip prompts
        #[arg(long)]
        non_interactive: bool,
        /// Skip Fika installation (server management only)
        #[arg(long)]
        skip_fika: bool,
    },

    /// Initialize Quartermaster for an SPT server
    Init {
        /// SPT directory path (auto-detects if omitted)
        path: Option<PathBuf>,
    },

    /// Install a mod and its dependencies
    Install {
        /// Mod name, Forge ID, or slug
        mod_ref: String,
        /// Specific version (latest compatible if omitted)
        version: Option<String>,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
    },

    /// Update installed mods
    Update {
        /// Specific mod to update (all if omitted)
        mod_ref: Option<String>,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
    },

    /// Remove an installed mod
    Remove {
        /// Mod name, Forge ID, or slug
        mod_ref: String,
        /// Bypass queue and apply immediately
        #[arg(long)]
        force: bool,
    },

    /// List installed mods
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Associate an unmanaged mod with a Forge entry
    Track {
        /// Relative path from SPT root (e.g. user/mods/SAIN)
        path: String,
        /// Forge mod ID or slug
        forge_mod_id: String,
    },

    /// Check all installed mods for updates
    Check,

    /// Apply pending queued operations
    Apply {
        /// Apply even if SPT server is running
        #[arg(long)]
        force: bool,
    },

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

    /// View and modify configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
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
    /// Alias for `quma status`
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Create a new SPT server container
    Create {
        /// Container name
        #[arg(long, default_value = "spt-server")]
        name: String,
        /// Host port to map to container port 6969
        #[arg(long, default_value = "6969")]
        port: u16,
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

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a config value
    Set { key: String, value: String },
    /// Get a config value
    Get { key: String },
}
