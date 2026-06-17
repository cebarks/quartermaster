use anyhow::{bail, Result};

use super::common::CliContext;

/// Interactive apply — prompts for confirmation, then drains the queue.
pub async fn run(force: bool, ctx: &CliContext) -> Result<()> {
    if !force {
        let running = crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running {
            bail!(
                "SPT server is running — stop it first or use --force.\n\
                 Applying changes while the server is running may cause instability."
            );
        }
    }

    let pending = ctx.db.list_pending_ops()?;
    if pending.is_empty() {
        println!("No pending operations to apply.");
        return Ok(());
    }

    println!("Pending operations ({}):", pending.len());
    for op in &pending {
        println!(
            "  {} {} (Forge ID: {}){}",
            op.action,
            op.mod_name,
            op.forge_mod_id,
            op.forge_version_id
                .map(|v| format!(", version ID: {v}"))
                .unwrap_or_default()
        );
    }

    if !super::common::confirm("Apply all pending operations?")? {
        println!("Cancelled.");
        return Ok(());
    }

    let applied = drain_all(ctx).await?;
    println!("\n{applied} operation(s) applied.");
    Ok(())
}

/// Apply all pending operations without prompting for confirmation.
/// Returns the number of operations successfully applied.
/// Used by `quma apply` (after confirmation) and auto-drain (server lifecycle).
pub async fn drain_all(ctx: &CliContext) -> Result<usize> {
    let pending = ctx.db.list_pending_ops()?;
    let mut applied = 0;

    for op in &pending {
        println!("  Applying: {} {}...", op.action, op.mod_name);
        match op.action.as_str() {
            "install" => {
                if let Some(version_id) = op.forge_version_id {
                    crate::cli::install::install_with_deps(ctx, op.forge_mod_id, version_id)
                        .await?;
                } else {
                    println!("    Skipped — no version ID for install operation");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            "remove" => {
                if let Some(installed) = ctx.db.get_mod_by_forge_id(op.forge_mod_id)? {
                    let files = ctx.db.get_files_for_mod(installed.id)?;
                    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
                    crate::spt::mods::delete_mod_files(&ctx.spt_dir, &paths)?;
                    ctx.db.delete_mod(installed.id)?;
                    println!("    Removed {} ({} files)", op.mod_name, paths.len());
                } else {
                    println!("    Skipped — {} not found in database", op.mod_name);
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            "update" => {
                if let (Some(installed), Some(version_id)) = (
                    ctx.db.get_mod_by_forge_id(op.forge_mod_id)?,
                    op.forge_version_id,
                ) {
                    crate::cli::update::apply_update_by_version(ctx, &installed, version_id)
                        .await?;
                } else {
                    println!("    Skipped — mod not found or no version ID");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            other => {
                println!("    Skipped — unknown action: {other}");
                ctx.db.delete_pending_op(op.id)?;
                continue;
            }
        }

        ctx.db.delete_pending_op(op.id)?;
        applied += 1;
    }

    Ok(applied)
}
