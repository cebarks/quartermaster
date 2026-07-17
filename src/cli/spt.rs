use anyhow::{Context, Result};
use clap::Subcommand;

use crate::spt::detect::read_spt_version;
use crate::spt::releases;

use super::common::CliContext;

#[derive(Subcommand)]
pub enum SptAction {
    /// Show installed SPT server version
    Version,
    /// Check for SPT server updates
    Check,
    /// Update the SPT server to the latest version
    Update {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
    },
}

pub async fn run(action: &SptAction, ctx: &CliContext) -> Result<()> {
    match action {
        SptAction::Version => version(ctx),
        SptAction::Check => check(ctx).await,
        SptAction::Update { yes } => update(ctx, *yes).await,
    }
}

fn version(ctx: &CliContext) -> Result<()> {
    println!(
        "SPT {} (EFT {})",
        ctx.spt_info.spt_version, ctx.spt_info.tarkov_version
    );
    Ok(())
}

async fn check(ctx: &CliContext) -> Result<()> {
    let installed = &ctx.spt_info.spt_version;
    println!("Installed: SPT {installed}");
    println!("Checking for updates...");

    let latest = releases::get_latest_release().await?;
    if latest.version == *installed {
        println!("Already up to date.");
    } else {
        println!(
            "Update available: SPT {} → {} (EFT {})",
            installed, latest.version, latest.eft_version
        );
        println!("Run `quma spt update` to install.");
    }
    Ok(())
}

async fn update(ctx: &CliContext, skip_confirm: bool) -> Result<()> {
    let installed = &ctx.spt_info.spt_version;
    let latest = releases::get_latest_release().await?;

    if latest.version == *installed {
        println!("SPT {} is already the latest version.", installed);
        return Ok(());
    }

    println!(
        "Update available: SPT {} → {} (EFT {})",
        installed, latest.version, latest.eft_version
    );

    if !skip_confirm {
        if !super::common::confirm("Proceed with update?")? {
            println!("Update cancelled.");
            return Ok(());
        }
    }

    // Stop server if running
    let was_running = if let Some(ref mgr) = ctx.container_mgr {
        if let Some(ref container) = ctx.config.server_container {
            let running = mgr
                .inspect(container)
                .await
                .ok()
                .and_then(|info| info.state)
                .and_then(|state| state.running)
                .unwrap_or(false);
            if running {
                println!("Stopping server...");
                mgr.stop(container).await?;
                println!("Server stopped.");
            }
            running
        } else {
            false
        }
    } else {
        false
    };

    // Auto-backup
    println!("Creating backup...");
    match crate::backup::backup_full(&ctx.db, &ctx.dirs, &ctx.config) {
        Ok(backup_id) => println!("Backup created (ID: {backup_id})."),
        Err(e) => {
            if ctx.config.backup.require_backup {
                return Err(e).context("backup failed and require_backup is enabled");
            }
            tracing::warn!(err = %e, "backup failed, continuing anyway");
        }
    }

    // Download and extract
    println!(
        "Downloading SPT {} (EFT {})...",
        latest.version, latest.eft_version
    );

    let last_log = std::sync::Mutex::new(std::time::Instant::now());
    releases::download_and_extract_release(&latest, &ctx.dirs.spt_server, |downloaded, total| {
        let mut last = last_log.lock().unwrap();
        if last.elapsed().as_secs() >= 5 {
            let dl_mb = downloaded as f64 / 1_048_576.0;
            if let Some(t) = total {
                let total_mb = t as f64 / 1_048_576.0;
                println!(
                    "  {dl_mb:.1} / {total_mb:.1} MB ({:.0}%)",
                    dl_mb / total_mb * 100.0
                );
            } else {
                println!("  {dl_mb:.1} MB downloaded");
            }
            *last = std::time::Instant::now();
        }
    })
    .await?;

    // Verify new version
    let new_info = read_spt_version(&ctx.dirs.spt_server)?;
    println!(
        "Updated: SPT {} → {} (EFT {})",
        installed, new_info.spt_version, new_info.tarkov_version
    );

    // Restart server if it was running
    if was_running {
        if let (Some(ref mgr), Some(ref container)) =
            (&ctx.container_mgr, &ctx.config.server_container)
        {
            println!("Restarting server...");
            mgr.start(container).await?;
            println!("Server started.");
        }
    }

    Ok(())
}
