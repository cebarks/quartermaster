use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use futures_util::StreamExt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::client::converge::client_container_name;
use crate::config::{is_fika_installed, Config, HeadlessClientDef, HeadlessConfig};
use crate::container::ContainerManager;
use crate::server_detect;
use crate::spt::headless::EHeadlessStatus;
use crate::spt::server::SptClient;

use super::common::{confirm, CliContext};

#[derive(Subcommand)]
pub enum HeadlessAction {
    /// Show headless client status
    Status {
        /// Client number for detailed view
        client: Option<u32>,
    },
    /// Create a new headless client
    Create {
        /// Extra isolated paths for this client (additive to global)
        #[arg(long)]
        extra_isolated_paths: Vec<String>,
    },
    /// Delete a specific headless client
    Delete {
        /// Client number
        client: u32,
        /// Force delete even if client is in a raid
        #[arg(long)]
        force: bool,
    },
    /// Stream container logs for a client
    Logs {
        /// Client number
        client: u32,
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
    /// Stop a headless client
    Stop {
        /// Client number
        client: u32,
    },
    /// Start a headless client
    Start {
        /// Client number
        client: u32,
    },
    /// Restart a headless client
    Restart {
        /// Client number
        client: u32,
    },
    /// Set the desired number of headless clients
    Scale {
        /// Desired number of clients
        count: u32,
    },
}

pub async fn run(action: &HeadlessAction, ctx: &CliContext) -> Result<()> {
    if !is_fika_installed(&ctx.spt_dir) {
        bail!(
            "Fika server mod is not installed.\n\
             Install Fika with: quma install fika-server\n\
             Or run setup: quma setup"
        );
    }

    match action {
        HeadlessAction::Status { client } => status(ctx, *client).await,
        HeadlessAction::Create {
            extra_isolated_paths,
        } => create(ctx, extra_isolated_paths).await,
        HeadlessAction::Delete { client, force } => delete(ctx, *client, *force).await,
        HeadlessAction::Logs { client, follow } => logs(ctx, *client, *follow).await,
        HeadlessAction::Stop { client } => stop(ctx, *client).await,
        HeadlessAction::Start { client } => start(ctx, *client).await,
        HeadlessAction::Restart { client } => restart(ctx, *client).await,
        HeadlessAction::Scale { count } => scale(ctx, *count).await,
    }
}

async fn status(ctx: &CliContext, client: Option<u32>) -> Result<()> {
    let headless_config = require_headless_config(&ctx.config)?;
    let container_mgr = require_container_manager(ctx)?;

    // Get live data from containers (including PROFILE_ID for Fika correlation)
    let mut states: Vec<(u32, String, &str, Option<String>)> = Vec::new();
    for index in 1..=headless_config.client_count() {
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
                    let client_count = headless_config.client_count();
                    println!(
                        "\nFika Status: awaiting profile assignment. \
                         Restart the SPT server to generate headless profiles, \
                         then run `quma headless scale {}` to re-provision.",
                        client_count
                    );
                }
            }
        }
    }

    Ok(())
}

async fn create(ctx: &CliContext, extra_isolated_paths: &[String]) -> Result<()> {
    let _headless_config = require_headless_config(&ctx.config)?;
    let container_mgr = require_container_manager(ctx)?;

    let config_path = Config::resolve_path(None, Some(&ctx.spt_dir));
    let mut config = Config::load_with_env(&config_path)?;

    let new_def = HeadlessClientDef {
        extra_isolated_paths: extra_isolated_paths.to_vec(),
    };

    let headless = config.headless.get_or_insert_with(HeadlessConfig::default);
    headless.clients.push(new_def);
    let index = headless.client_count();
    let total = index; // capture before dropping borrow

    config.save(&config_path)?;
    println!("Created headless client {} (total: {})", index, total);

    let (server_host, server_port) =
        crate::server_detect::resolve_server_addr(&config, &ctx.spt_dir);
    let spt_client = SptClient::new(&server_host, server_port)?;
    let converging = Arc::new(AtomicBool::new(false));

    crate::client::converge::converge(
        container_mgr,
        config.headless.as_ref().unwrap(),
        &config,
        &ctx.spt_dir,
        &spt_client,
        converging,
    )
    .await?;

    println!("Client {} created and started.", index);
    Ok(())
}

async fn delete(ctx: &CliContext, client: u32, force: bool) -> Result<()> {
    let headless_config = require_headless_config(&ctx.config)?;
    let container_mgr = require_container_manager(ctx)?;

    let index = client as usize;
    if index == 0 || index > headless_config.clients.len() {
        bail!(
            "Client {} does not exist (valid range: 1-{})",
            client,
            headless_config.client_count()
        );
    }

    if !force {
        let (server_host, server_port) =
            crate::server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
        let spt_client = SptClient::new(&server_host, server_port)?;
        if let Ok(resp) = spt_client.headless_clients().await {
            let container_name = crate::client::converge::client_container_name(client);
            if let Ok(info) = container_mgr.inspect(&container_name).await {
                if let Some(pid) = info.config.and_then(|c| c.env).and_then(|env| {
                    env.iter()
                        .find(|e| e.starts_with("PROFILE_ID="))
                        .and_then(|e| e.strip_prefix("PROFILE_ID="))
                        .map(String::from)
                }) {
                    if let Some(client_info) = resp.headlesses.get(&pid) {
                        if client_info.state == EHeadlessStatus::InRaid
                            && !client_info.players.is_empty()
                        {
                            bail!(
                                "Client {} is in a raid with {} player(s). Use --force to override.",
                                client,
                                client_info.players.len()
                            );
                        }
                    }
                }
            }
        }
    }

    // Stop and remove container
    let container_name = crate::client::converge::client_container_name(client);
    if container_mgr
        .is_running(&container_name)
        .await
        .unwrap_or(false)
    {
        println!("Stopping {}...", container_name);
        container_mgr.stop(&container_name).await?;
    }
    if container_mgr.inspect(&container_name).await.is_ok() {
        container_mgr.remove_container(&container_name).await?;
    }

    // Remove from config
    let config_path = Config::resolve_path(None, Some(&ctx.spt_dir));
    let mut config = Config::load_with_env(&config_path)?;
    if let Some(ref mut headless) = config.headless {
        headless.clients.remove(index - 1);
    }
    config.save(&config_path)?;

    // Update fika.jsonc
    let fika_config_path = ctx
        .spt_dir
        .join("SPT/user/mods/fika-server/assets/configs/fika.jsonc");
    let new_count = config
        .headless
        .as_ref()
        .map(|h| h.client_count())
        .unwrap_or(0);
    crate::client::converge::edit_headless_amount(&fika_config_path, new_count)?;

    // Clean up overlay directory
    let overlay_dir = ctx.spt_dir.join("clients").join(client.to_string());
    if overlay_dir.exists() {
        std::fs::remove_dir_all(&overlay_dir).ok();
    }

    println!(
        "Deleted client {}. {} client(s) remaining.",
        client, new_count
    );
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

async fn stop(ctx: &CliContext, client: u32) -> Result<()> {
    let container_mgr = require_container_manager(ctx)?;
    let name = client_container_name(client);

    // Verify container exists
    container_mgr
        .inspect(&name)
        .await
        .with_context(|| format!("container '{}' not found", name))?;

    println!("Stopping {}...", name);
    container_mgr.stop(&name).await?;
    println!("Client {} stopped successfully.", client);

    Ok(())
}

async fn start(ctx: &CliContext, client: u32) -> Result<()> {
    let container_mgr = require_container_manager(ctx)?;
    let name = client_container_name(client);

    // Verify container exists
    container_mgr
        .inspect(&name)
        .await
        .with_context(|| format!("container '{}' not found", name))?;

    println!("Starting {}...", name);
    container_mgr.start(&name).await?;
    println!("Client {} started successfully.", client);

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

    let old_count = config
        .headless
        .as_ref()
        .map(|h| h.client_count())
        .unwrap_or(0);

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
            for index in (count + 1)..=old_count {
                let container_name = crate::client::converge::client_container_name(index);
                if let Ok(info) = container_mgr.inspect(&container_name).await {
                    if let Some(pid) = info.config.and_then(|c| c.env).and_then(|env| {
                        env.iter()
                            .find(|e| e.starts_with("PROFILE_ID="))
                            .and_then(|e| e.strip_prefix("PROFILE_ID="))
                            .map(String::from)
                    }) {
                        if let Some(client_info) = resp.headlesses.get(&pid) {
                            if client_info.state == EHeadlessStatus::InRaid
                                && !client_info.players.is_empty()
                            {
                                let prompt = format!(
                                    "Client {} is in a raid with {} player(s). Scale down anyway?",
                                    index,
                                    client_info.players.len()
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
        }
    }

    // Update config: add or remove HeadlessClientDef entries
    let headless = config.headless.get_or_insert_with(HeadlessConfig::default);
    if count > old_count {
        // Scale up: push new default entries
        for _ in old_count..count {
            headless.clients.push(HeadlessClientDef::default());
        }
    } else {
        // Scale down: truncate the vec
        headless.clients.truncate(count as usize);
    }

    config.save(&config_path)?;
    println!("Updated config: {} headless client(s)", count);

    // Run convergence
    let (server_host, server_port) = server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&server_host, server_port)?;
    let converging = Arc::new(AtomicBool::new(false));

    println!("Converging to {} client(s)...", count);
    crate::client::converge::converge(
        container_mgr,
        config.headless.as_ref().unwrap(),
        &config,
        &ctx.spt_dir,
        &spt_client,
        converging,
    )
    .await?;

    println!("Successfully scaled to {} client(s).", count);
    Ok(())
}

fn require_headless_config(config: &Config) -> Result<&HeadlessConfig> {
    config.headless.as_ref().ok_or_else(|| {
        anyhow!(
            "headless clients not configured.\n\
             Add a [headless] section to quartermaster.toml or run `quma setup`."
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
