use anyhow::{bail, Result};

use crate::container::ContainerManager;
use crate::spt::server::SptClient;

use super::common::CliContext;
use super::ServerAction;

pub async fn run(action: &ServerAction, ctx: &CliContext) -> Result<()> {
    match action {
        ServerAction::Start { timeout } => start(ctx, *timeout).await,
        ServerAction::Stop => stop(ctx).await,
        ServerAction::Restart { drain, skip_queue } => restart(ctx, *drain, *skip_queue).await,
        ServerAction::Logs { follow } => logs(ctx, *follow).await,
    }
}

async fn start(ctx: &CliContext, timeout_secs: u64) -> Result<()> {
    let (mgr, container) = require_container(ctx)?;

    if ctx.config.auto_drain_on_lifecycle {
        drain_if_pending(ctx).await?;
    }

    println!("Starting SPT server container...");
    mgr.start(container).await?;

    wait_for_ping(ctx, timeout_secs).await
}

async fn stop(ctx: &CliContext) -> Result<()> {
    let (mgr, container) = require_container(ctx)?;

    println!("Stopping SPT server container...");
    mgr.stop(container).await?;
    println!("Server stopped.");

    if ctx.config.auto_drain_on_lifecycle {
        drain_if_pending(ctx).await?;
    }

    Ok(())
}

async fn restart(ctx: &CliContext, force_drain: bool, skip_queue: bool) -> Result<()> {
    let (mgr, container) = require_container(ctx)?;

    println!("Stopping SPT server container...");
    mgr.stop(container).await?;
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
    mgr.start(container).await?;

    wait_for_ping(ctx, 60).await
}

async fn logs(ctx: &CliContext, follow: bool) -> Result<()> {
    let (mgr, container) = require_container(ctx)?;
    use futures_util::StreamExt;
    let mut stream = mgr.log_stream(container, 100, follow);
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

fn require_container(ctx: &CliContext) -> Result<(&ContainerManager, &str)> {
    match (&ctx.config.server_container, &ctx.container_mgr) {
        (None, _) => bail!(
            "no server_container configured.\n\
             Set server_container in quartermaster.toml or run `quma setup`"
        ),
        (Some(_), None) => bail!(
            "failed to connect to Podman socket.\n\
             Ensure podman.socket is enabled:\n  \
             systemctl --user enable --now podman.socket"
        ),
        (Some(name), Some(mgr)) => Ok((mgr, name.as_str())),
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
