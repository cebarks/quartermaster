mod cli;
mod config;
mod db;
mod error;
mod forge;
mod health;
mod ops;
mod podman;
mod queue;
mod server_detect;
mod spt;
mod web;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
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
            cli::install::run(mod_ref, version.as_deref(), *force, &ctx).await
        }
        Command::Update { mod_ref, force } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::update::run(mod_ref.as_deref(), *force, &ctx).await
        }
        Command::Remove { mod_ref, force } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::remove::run(mod_ref, *force, &ctx).await
        }
        Command::List { json } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::list::run(*json, &ctx)
        }
        Command::Track { path, forge_mod_id } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::track::run(path, forge_mod_id, &ctx).await
        }
        Command::Check => {
            let ctx = cli::common::resolve_context(&cli)?;
            let has_updates = cli::check::run(&ctx).await?;
            drop(ctx);
            if has_updates {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Apply { force } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::apply::run(*force, &ctx).await
        }
        Command::Status { json } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::status::run(*json, &ctx).await
        }
        Command::Server { action } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::server::run(action, &ctx).await
        }
        Command::Serve { bind, port } => cli::serve::run(bind.as_deref(), *port, &cli).await,
        Command::Generate { target } => cli::generate::run(target, &cli),
        Command::Invite { expires } => {
            let ctx = cli::common::resolve_context(&cli)?;
            cli::invite::run(expires.as_deref(), &ctx)
        }
        Command::Config { action } => cli::config_cmd::run(action, &cli),
    }
}
