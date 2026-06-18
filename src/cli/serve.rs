use std::sync::Arc;

use anyhow::{Context, Result};

use super::Cli;
use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::logging;
use crate::spt::detect::{detect_spt_dir, read_spt_version};

pub async fn run(bind: Option<&str>, port: Option<u16>, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let spt_info = read_spt_version(&spt_dir)?;

    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = Config::load_with_env(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    if let Some(b) = bind {
        config.web_bind = b.to_string();
    }
    if let Some(p) = port {
        config.web_port = p;
    }

    // Create LogBroadcast with configured buffer size
    let log_broadcast = Arc::new(logging::LogBroadcast::new(config.logging.web.buffer_size));
    let reload_handles = logging::init_subscriber(&log_broadcast);

    // Reconfigure logging now that config is loaded
    let filter =
        logging::resolve_log_filter(&config.logging, cli.verbose, cli.log_level.as_deref());
    reload_handles.reconfigure(&config.logging, &filter, Some(&spt_dir));

    config.ensure_session_secret();
    config
        .save(&config_path)
        .context("failed to save config with session secret")?;

    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    if !db.admin_exists()? {
        anyhow::bail!("No admin user exists. Run `quma init` first to create an admin account.");
    }

    let forge = ForgeClient::new(config.forge_token.clone())?;

    crate::web::start_server(
        config,
        db,
        forge,
        spt_dir,
        spt_info,
        Arc::clone(&log_broadcast),
    )
    .await
}
