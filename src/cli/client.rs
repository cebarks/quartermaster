use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use futures_util::StreamExt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::client::converge::client_container_name;
use crate::config::{is_fika_installed, ClientsConfig, Config};
use crate::container::ContainerManager;
use crate::server_detect;
use crate::spt::headless::EHeadlessStatus;
use crate::spt::server::SptClient;

use super::common::{confirm, CliContext};

#[derive(Subcommand)]
pub enum ClientAction {
    /// Show dedicated client status
    Status {
        /// Client number for detailed view
        client: Option<u32>,
    },
    /// Stream container logs for a client
    Logs {
        /// Client number
        client: u32,
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
    /// Restart a dedicated client
    Restart {
        /// Client number
        client: u32,
    },
    /// Set the desired number of dedicated clients and converge
    Scale {
        /// Desired number of clients
        count: u32,
    },
}

pub async fn run(action: &ClientAction, ctx: &CliContext) -> Result<()> {
    // All commands check Fika detection first
    if !is_fika_installed(&ctx.spt_dir) {
        bail!(
            "Fika server mod is not installed.\n\
             Install Fika with: quma install fika-server\n\
             Or run setup: quma setup"
        );
    }

    match action {
        ClientAction::Status { client } => status(ctx, *client).await,
        ClientAction::Logs { client, follow } => logs(ctx, *client, *follow).await,
        ClientAction::Restart { client } => restart(ctx, *client).await,
        ClientAction::Scale { count } => scale(ctx, *count).await,
    }
}

async fn status(ctx: &CliContext, client: Option<u32>) -> Result<()> {
    let clients_config = require_clients_config(&ctx.config)?;
    let container_mgr = require_container_manager(ctx)?;

    // Get live data from containers (including PROFILE_ID for Fika correlation)
    let mut states: Vec<(u32, String, &str, Option<String>)> = Vec::new();
    for index in 1..=clients_config.count {
        let name = client_container_name(index);

        let (container_status, profile_id) = match container_mgr.inspect(&name).await {
            Ok(info) => {
                let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
                let status = if running { "running" } else { "stopped" };
                let pid = if running {
                    info.config.and_then(|c| c.env).and_then(|env| {
                        env.iter()
                            .find(|e| e.starts_with("PROFILE_ID="))
                            .and_then(|e| e.strip_prefix("PROFILE_ID="))
                            .map(String::from)
                    })
                } else {
                    None
                };
                (status, pid)
            }
            Err(_) => ("not found", None),
        };

        states.push((index, name, container_status, profile_id));
    }

    // Get Fika API data if server is running
    let (server_host, server_port) = server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&server_host, server_port)?;

    let headless_map = match spt_client.headless_clients().await {
        Ok(resp) => Some(resp.headlesses),
        Err(_) => None,
    };

    match client {
        None => {
            // Table of all clients
            println!(
                "{:<8} {:<20} {:<15} {:<10}",
                "CLIENT", "CONTAINER", "STATUS", "FIKA STATE"
            );
            println!("{}", "-".repeat(60));

            for (index, name, container_status, profile_id) in &states {
                let fika_state: String = if let Some(ref map) = headless_map {
                    match profile_id {
                        Some(pid) => match map.get(pid) {
                            Some(info) => match &info.state {
                                EHeadlessStatus::Ready => "Ready".into(),
                                EHeadlessStatus::InRaid => "In Raid".into(),
                                EHeadlessStatus::Unknown(s) => format!("Unknown({s})"),
                            },
                            None => "no data".into(),
                        },
                        None => "awaiting profile".into(),
                    }
                } else {
                    "server offline".into()
                };

                println!(
                    "{:<8} {:<20} {:<15} {:<10}",
                    index, name, container_status, fika_state
                );
            }
        }
        Some(index) => {
            // Detailed single client view
            let name = client_container_name(index);

            println!("Client {}", index);
            println!("  Container: {}", name);

            let inspect = container_mgr
                .inspect(&name)
                .await
                .with_context(|| format!("container '{}' not found", name))?;

            let running = inspect
                .state
                .as_ref()
                .and_then(|s| s.running)
                .unwrap_or(false);

            println!("  Status: {}", if running { "running" } else { "stopped" });

            if let Some(state) = &inspect.state {
                if let Some(started_at) = &state.started_at {
                    println!("  Started: {}", started_at);
                }
            }
            if let Some(restart_count) = &inspect.restart_count {
                println!("  Restart count: {}", restart_count);
            }

            // Extract PROFILE_ID and show per-client Fika data
            let profile_id = if running {
                inspect.config.and_then(|c| c.env).and_then(|env| {
                    env.iter()
                        .find(|e| e.starts_with("PROFILE_ID="))
                        .and_then(|e| e.strip_prefix("PROFILE_ID="))
                        .map(String::from)
                })
            } else {
                None
            };

            if let Some(ref pid) = profile_id {
                println!("  Profile ID: {}", pid);
            }

            if let Some(ref map) = headless_map {
                if let Some(ref pid) = profile_id {
                    if let Some(info) = map.get(pid) {
                        println!("\nFika Status:");
                        let state_str = match info.state {
                            EHeadlessStatus::Ready => "Ready",
                            EHeadlessStatus::InRaid => "In Raid",
                            EHeadlessStatus::Unknown(_) => "Unknown",
                        };
                        println!("  State: {}", state_str);
                        println!("  Players: {}", info.players.join(", "));
                        println!("  Level: {}", info.level);
                    } else {
                        println!("\nFika Status: no data for this client");
                    }
                } else {
                    // PROFILE_ID is set on the container env during convergence.
                    // It will be None if no headless profile was available at creation
                    // time (the SPT server needs to run with Fika to generate them)
                    // or if the container is stopped.
                    println!(
                        "\nFika Status: awaiting profile assignment. \
                         Restart the SPT server to generate headless profiles, \
                         then run `quma client scale {}` to re-provision.",
                        clients_config.count
                    );
                }
            }
        }
    }

    Ok(())
}

async fn logs(ctx: &CliContext, client: u32, follow: bool) -> Result<()> {
    let container_mgr = require_container_manager(ctx)?;
    let name = client_container_name(client);

    // Verify container exists
    container_mgr
        .inspect(&name)
        .await
        .with_context(|| format!("container '{}' not found", name))?;

    println!("Streaming logs for {}...", name);

    let mut stream = container_mgr.log_stream(&name, 100, follow);
    while let Some(log) = stream.next().await {
        match log? {
            bollard::container::LogOutput::StdOut { message } => {
                print!("{}", String::from_utf8_lossy(&message));
            }
            bollard::container::LogOutput::StdErr { message } => {
                eprint!("{}", String::from_utf8_lossy(&message));
            }
            _ => {}
        }
    }

    Ok(())
}

async fn restart(ctx: &CliContext, client: u32) -> Result<()> {
    let container_mgr = require_container_manager(ctx)?;
    let name = client_container_name(client);

    // Verify container exists
    container_mgr
        .inspect(&name)
        .await
        .with_context(|| format!("container '{}' not found", name))?;

    println!("Restarting {}...", name);
    container_mgr.restart(&name).await?;
    println!("Client {} restarted successfully.", client);

    Ok(())
}

async fn scale(ctx: &CliContext, count: u32) -> Result<()> {
    let container_mgr = require_container_manager(ctx)?;

    // Load config
    let config_path = Config::resolve_path(None, Some(&ctx.spt_dir));
    let mut config = Config::load_with_env(&config_path)?;

    let old_count = config.clients.as_ref().map(|c| c.count).unwrap_or(0);

    if count == old_count {
        println!("Already at {} clients.", count);
        return Ok(());
    }

    // If scaling down, check for in-raid clients
    if count < old_count {
        let (server_host, server_port) =
            server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
        let spt_client = SptClient::new(&server_host, server_port)?;

        if let Ok(resp) = spt_client.headless_clients().await {
            for _index in (count + 1)..=old_count {
                // Check if any headless client is in raid with players
                for (session_id, info) in &resp.headlesses {
                    if info.state == EHeadlessStatus::InRaid && !info.players.is_empty() {
                        let player_count = info.players.len();
                        let prompt = format!(
                            "Client in session {} is currently in a raid with {} player(s). Scale down anyway?",
                            session_id, player_count
                        );
                        if !confirm(&prompt)? {
                            println!("Scale operation cancelled.");
                            return Ok(());
                        }
                        break;
                    }
                }
            }
        }
    }

    // Update config
    if let Some(ref mut clients) = config.clients {
        clients.count = count;
    } else {
        // Should not happen if require_clients_config passed, but handle it
        bail!("clients config not found");
    }

    config.save(&config_path)?;
    println!("Updated config: clients.count = {}", count);

    // Run convergence
    let (server_host, server_port) = server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&server_host, server_port)?;
    let converging = Arc::new(AtomicBool::new(false));

    println!("Converging to {} client(s)...", count);
    crate::client::converge::converge(
        container_mgr,
        config.clients.as_ref().unwrap(),
        &config,
        &ctx.spt_dir,
        &spt_client,
        converging,
    )
    .await?;

    println!("Successfully scaled to {} client(s).", count);
    Ok(())
}

fn require_clients_config(config: &Config) -> Result<&ClientsConfig> {
    config.clients.as_ref().ok_or_else(|| {
        anyhow!(
            "dedicated clients not configured.\n\
             Run `quma setup` to configure Fika dedicated clients."
        )
    })
}

fn require_container_manager(ctx: &CliContext) -> Result<&ContainerManager> {
    ctx.container_mgr.as_ref().ok_or_else(|| {
        anyhow!(
            "failed to connect to Podman socket.\n\
             Ensure podman.socket is enabled:\n  \
             systemctl --user enable --now podman.socket"
        )
    })
}
