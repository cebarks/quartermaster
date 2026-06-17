use anyhow::{bail, Result};

use crate::podman::PodmanClient;
use crate::spt::server::SptClient;

use super::common::CliContext;
use super::ServerAction;

pub async fn run(action: &ServerAction, ctx: &CliContext) -> Result<()> {
    match action {
        ServerAction::Start { timeout } => start(ctx, *timeout).await,
        ServerAction::Stop => stop(ctx).await,
        ServerAction::Restart { drain, skip_queue } => restart(ctx, *drain, *skip_queue).await,
        ServerAction::Logs { follow } => logs(ctx, *follow).await,
        ServerAction::Status { json } => crate::cli::status::run(*json, ctx).await,
    }
}

async fn start(ctx: &CliContext, timeout_secs: u64) -> Result<()> {
    let podman = require_container(ctx)?;

    if ctx.config.auto_drain_on_lifecycle {
        drain_if_pending(ctx).await?;
    }

    println!("Starting SPT server container...");
    podman.start().await?;

    wait_for_ping(ctx, timeout_secs).await
}

async fn stop(ctx: &CliContext) -> Result<()> {
    let podman = require_container(ctx)?;

    if ctx.config.auto_drain_on_lifecycle {
        drain_if_pending(ctx).await?;
    }

    println!("Stopping SPT server container...");
    podman.stop().await?;
    println!("Server stopped.");
    Ok(())
}

async fn restart(ctx: &CliContext, force_drain: bool, skip_queue: bool) -> Result<()> {
    let podman = require_container(ctx)?;

    println!("Stopping SPT server container...");
    podman.stop().await?;
    println!("Server stopped.");

    let should_drain = if skip_queue {
        false
    } else if force_drain {
        true
    } else {
        ctx.config.auto_drain_on_lifecycle
    };

    if should_drain {
        drain_if_pending(ctx).await?;
    }

    println!("Starting SPT server container...");
    podman.start().await?;

    wait_for_ping(ctx, 60).await
}

async fn logs(ctx: &CliContext, follow: bool) -> Result<()> {
    let podman = require_container(ctx)?;
    podman.logs(follow, 100).await
}

async fn wait_for_ping(ctx: &CliContext, timeout_secs: u64) -> Result<()> {
    let (host, port) = crate::server_detect::resolve_server_addr(&ctx.config, &ctx.spt_dir);
    let spt_client = SptClient::new(&host, port)?;

    println!("Waiting for server to respond (timeout: {timeout_secs}s)...");
    let start_time = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        if start_time.elapsed() > timeout {
            bail!(
                "Server did not respond within {timeout_secs}s — check `quma server logs` for errors"
            );
        }

        let ping = spt_client.ping().await?;
        if ping.ok {
            println!("Server is ready (responded in {}ms).", ping.latency_ms);
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

fn require_container(ctx: &CliContext) -> Result<PodmanClient> {
    match &ctx.config.server_container {
        Some(name) => Ok(PodmanClient::new(name)),
        None => bail!(
            "no server_container configured.\n\
             Set it with: quma config set server_container <name>\n\
             Or run `quma setup` to auto-detect."
        ),
    }
}

async fn drain_if_pending(ctx: &CliContext) -> Result<()> {
    let pending = ctx.db.list_pending_ops()?;
    if pending.is_empty() {
        return Ok(());
    }
    println!("\nDraining {} pending operation(s)...", pending.len());
    crate::cli::apply::drain_all(ctx).await?;
    Ok(())
}
