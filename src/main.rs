#![deny(clippy::unwrap_used)]

mod backup;
mod cli;
mod client;
mod config;
mod config_mgmt;
mod container;
mod db;
mod fika;
mod forge;
mod headless_sync;
mod health;
mod invite;
mod logging;
mod modsync;
mod numa;
mod ops;
mod queue;
mod server_detect;
mod spt;
mod svm;
mod tls;
mod web;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

/// Resolve context and reconfigure logging for commands that need an SPT directory.
fn init_context(cli: &Cli, handles: &logging::ReloadHandles) -> Result<cli::common::CliContext> {
    let ctx = cli::common::resolve_context(cli)?;

    let mut logging_config = ctx.config.logging.clone();
    logging_config.file.enabled = false; // CLI commands don't file-log
    if let Some(ref fmt) = cli.log_format {
        if let Ok(format) = fmt.parse::<config::ConsoleFormat>() {
            logging_config.console.format = format;
        }
    }

    let filter =
        logging::resolve_log_filter(&logging_config, cli.verbose, cli.log_level.as_deref());
    handles.reconfigure(&logging_config, &filter, Some(&ctx.spt_dir));
    Ok(ctx)
}

/// Apply CLI verbosity to default logging config (for commands that run before config exists).
fn init_early_logging(cli: &Cli, handles: &logging::ReloadHandles) {
    let mut logging_config = config::LoggingConfig::default();
    logging_config.file.enabled = false; // CLI commands don't file-log
    let filter =
        logging::resolve_log_filter(&logging_config, cli.verbose, cli.log_level.as_deref());
    handles.reconfigure(&logging_config, &filter, None);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Serve command handles its own logging setup in serve.rs
    if let Command::Serve { bind, port } = &cli.command {
        return cli::serve::run(bind.as_deref(), *port, &cli).await;
    }

    // For non-serve commands, use default buffer size
    let log_broadcast = Arc::new(logging::LogBroadcast::new(1000));
    let reload_handles = logging::init_subscriber(&log_broadcast, None);

    match &cli.command {
        Command::Setup {
            path,
            no_fika,
            no_modsync,
            admin_password,
            dev,
            container_name,
        } => {
            init_early_logging(&cli, &reload_handles);
            cli::setup::run(
                cli::setup::SetupArgs {
                    path: path.clone(),
                    no_fika: *no_fika,
                    no_modsync: *no_modsync,
                    admin_password: admin_password.clone(),
                    dev: *dev,
                    container_name: container_name.clone(),
                },
                &cli,
            )
            .await
        }
        Command::Generate { target } => {
            init_early_logging(&cli, &reload_handles);
            cli::generate::run(target, &cli)
        }
        Command::Install {
            mod_ref,
            version,
            name,
            force,
            addon,
        } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::install::run(
                mod_ref,
                version.as_deref(),
                name.as_deref(),
                *force,
                *addon,
                &ctx,
            )
            .await
        }
        Command::Update {
            mod_ref,
            force,
            addon,
        } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::update::run(mod_ref.as_deref(), *force, *addon, &ctx).await
        }
        Command::Remove {
            mod_ref,
            force,
            yes,
            addon,
        } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::remove::run(mod_ref, *force, *yes, *addon, &ctx).await
        }
        Command::List { json } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::list::run(*json, &ctx)
        }
        Command::Check => {
            let ctx = init_context(&cli, &reload_handles)?;
            let has_updates = cli::check::run(&ctx).await?;
            drop(ctx);
            if has_updates {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Status { json } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::status::run(*json, &ctx).await
        }
        Command::Server { action } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::server::run(action, &ctx).await
        }
        Command::Headless { action } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::headless::run(action, &ctx).await
        }
        Command::Invite { expires } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::invite::run(expires.as_deref(), &ctx)
        }
        Command::Backup { mod_ref, list } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::backup::run(mod_ref.as_deref(), *list, &ctx)
        }
        Command::Restore {
            backup_id,
            latest,
            force,
        } => {
            let ctx = init_context(&cli, &reload_handles)?;
            cli::restore::run(*backup_id, latest.as_deref(), *force, &ctx).await
        }
        Command::Serve { .. } => unreachable!(),
    }
}
