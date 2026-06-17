mod cli;
mod config;
mod db;
mod error;
mod forge;
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
        Command::Update { .. } => todo!("update"),
        Command::Remove { .. } => todo!("remove"),
        Command::List { .. } => todo!("list"),
        Command::Track { .. } => todo!("track"),
        Command::Check => todo!("check"),
        Command::Apply { .. } => todo!("apply"),
        Command::Status { .. } => todo!("status"),
        Command::Server { .. } => todo!("server"),
        Command::Serve { .. } => todo!("serve"),
        Command::Generate { .. } => todo!("generate"),
        Command::Invite { .. } => todo!("invite"),
        Command::Config { .. } => todo!("config"),
    }
}
