mod cli;
mod config;
mod db;
mod error;
mod forge;
mod health;
mod logging;
mod ops;
mod podman;
mod queue;
mod server_detect;
mod spt;
mod web;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};
use config::Config;

/// Reconfigure logging once config (and spt_dir) are available.
fn reconfigure_logging(
    handles: &logging::ReloadHandles,
    config: &Config,
    cli: &Cli,
    spt_dir: Option<&std::path::Path>,
) {
    let filter =
        logging::resolve_log_filter(&config.logging, cli.verbose, cli.log_level.as_deref());
    handles.reconfigure(&config.logging, &filter, spt_dir);
}

#[tokio::main]
async fn main() -> Result<()> {
    // Early bootstrap: init subscriber with defaults + broadcast layer.
    // Config hasn't loaded yet, so we start with "info,quartermaster=debug".
    let log_broadcast = Arc::new(logging::LogBroadcast::new(1000));
    let reload_handles = logging::init_subscriber(&log_broadcast);

    let cli = Cli::parse();

    match &cli.command {
        Command::Setup {
            non_interactive,
            skip_fika,
        } => cli::setup::run(*non_interactive, *skip_fika, &cli).await,
        Command::Init { path } => cli::init::run(path.clone(), &cli),
        Command::Install {
            mod_ref,
            version,
            force,
        } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::install::run(mod_ref, version.as_deref(), *force, &ctx).await
        }
        Command::Update { mod_ref, force } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::update::run(mod_ref.as_deref(), *force, &ctx).await
        }
        Command::Remove { mod_ref, force } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::remove::run(mod_ref, *force, &ctx).await
        }
        Command::List { json } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::list::run(*json, &ctx)
        }
        Command::Track { path, forge_mod_id } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::track::run(path, forge_mod_id, &ctx).await
        }
        Command::Check => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            let has_updates = cli::check::run(&ctx).await?;
            drop(ctx);
            if has_updates {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Apply { force } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::apply::run(*force, &ctx).await
        }
        Command::Status { json } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::status::run(*json, &ctx).await
        }
        Command::Server { action } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::server::run(action, &ctx).await
        }
        Command::Serve { bind, port } => {
            cli::serve::run(
                bind.as_deref(),
                *port,
                &cli,
                &log_broadcast,
                &reload_handles,
            )
            .await
        }
        Command::Generate { target } => cli::generate::run(target, &cli),
        Command::Invite { expires } => {
            let ctx = cli::common::resolve_context(&cli)?;
            reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
            cli::invite::run(expires.as_deref(), &ctx)
        }
        Command::Config { action } => cli::config_cmd::run(action, &cli),
    }
}
