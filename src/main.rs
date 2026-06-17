mod cli;
mod config;
mod db;
mod error;
mod forge;
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
        Command::Setup { .. } => todo!("setup"),
        Command::Init { path } => cli::init::run(path.clone(), &cli),
        Command::Install {
            mod_ref,
            version: _,
            force,
        } => {
            // TODO(debt): version selection is handled inside run() for now;
            // wire explicit version arg when CLI dispatch is refactored
            let ctx = cli::common::resolve_context(&cli)?;
            cli::install::run(mod_ref, *force, &ctx).await
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
        Command::Status { .. } => todo!("status"),
        Command::Server { .. } => todo!("server"),
        Command::Serve { .. } => todo!("serve"),
        Command::Generate { .. } => todo!("generate"),
        Command::Invite { .. } => todo!("invite"),
        Command::Config { .. } => todo!("config"),
    }
}
