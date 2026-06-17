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
        Command::Install { .. } => todo!("install"),
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
