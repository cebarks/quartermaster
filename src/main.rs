mod cli;
mod client;
mod config;
mod container;
mod db;
mod error;
mod forge;
mod health;
mod invite;
mod logging;
mod ops;
mod queue;
mod server_detect;
mod spt;
mod tls;
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
    let cli = Cli::parse();

    // For Serve command, we'll create LogBroadcast after config loads.
    // For other commands, create with default buffer size (doesn't matter for non-web commands).
    match &cli.command {
        Command::Serve { bind, port } => {
            // Serve command creates LogBroadcast in serve.rs after loading config
            cli::serve::run(bind.as_deref(), *port, &cli).await
        }
        _ => {
            // For non-serve commands, use default buffer size
            let log_broadcast = Arc::new(logging::LogBroadcast::new(1000));
            let reload_handles = logging::init_subscriber(&log_broadcast);

            match &cli.command {
                Command::Setup { path, no_fika } => {
                    // Apply CLI verbosity to default config for early commands
                    let filter = logging::resolve_log_filter(
                        &config::LoggingConfig::default(),
                        cli.verbose,
                        cli.log_level.as_deref(),
                    );
                    reload_handles.reconfigure(&config::LoggingConfig::default(), &filter, None);
                    cli::setup::run(path.clone(), *no_fika, &cli).await
                }
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
                Command::Client { action } => {
                    let ctx = cli::common::resolve_context(&cli)?;
                    reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
                    cli::client::run(action, &ctx).await
                }
                Command::Generate { target } => {
                    // Apply CLI verbosity to default config for early commands
                    let filter = logging::resolve_log_filter(
                        &config::LoggingConfig::default(),
                        cli.verbose,
                        cli.log_level.as_deref(),
                    );
                    reload_handles.reconfigure(&config::LoggingConfig::default(), &filter, None);
                    cli::generate::run(target, &cli)
                }
                Command::Invite { expires } => {
                    let ctx = cli::common::resolve_context(&cli)?;
                    reconfigure_logging(&reload_handles, &ctx.config, &cli, Some(&ctx.spt_dir));
                    cli::invite::run(expires.as_deref(), &ctx)
                }
                Command::Serve { .. } => unreachable!(),
            }
        }
    }
}
