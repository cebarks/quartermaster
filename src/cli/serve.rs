use anyhow::{Context, Result};

use super::Cli;
use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
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

    crate::web::start_server(config, config_path, db, forge, spt_dir, spt_info).await
}
