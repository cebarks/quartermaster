use anyhow::{Context, Result};

use super::common::CliContext;

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
                    if let Err(e) =
                        crate::cli::install::install_with_deps(ctx, op.forge_mod_id, version_id)
                            .await
                            .with_context(|| {
                                format!("failed to apply queued install of {}", op.mod_name)
                            })
                    {
                        let remaining = pending.len() - applied - 1;
                        eprintln!("\n  Error: {e:#}");
                        eprintln!(
                            "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                        );
                        return Err(e);
                    }
                } else {
                    println!("    Skipped — no version ID for install operation");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }
            }
            "remove" => {
                if let Some(installed) = ctx.db.get_mod_by_forge_id(op.forge_mod_id)? {
                    // Check reverse dependencies like interactive remove does
                    let reverse_deps =
                        crate::cli::remove::collect_all_reverse_deps(installed.id, ctx)?;
                    if !reverse_deps.is_empty() {
                        let names: Vec<&str> =
                            reverse_deps.iter().map(|m| m.name.as_str()).collect();
                        println!("    Also removing dependents: {}", names.join(", "));
                        for dep in reverse_deps.iter().rev() {
                            if let Err(e) = crate::cli::remove::remove_single_mod(dep, ctx) {
                                let remaining = pending.len() - applied - 1;
                                eprintln!("\n  Error: {e:#}");
                                eprintln!(
                                    "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                                );
                                return Err(e);
                            }
                        }
                    }
                    if let Err(e) = crate::cli::remove::remove_single_mod(&installed, ctx) {
                        let remaining = pending.len() - applied - 1;
                        eprintln!("\n  Error: {e:#}");
                        eprintln!(
                            "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                        );
                        return Err(e);
                    }
                    println!("    Removed {}", op.mod_name);
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
                    if let Err(e) =
                        crate::cli::update::apply_update_by_version(ctx, &installed, version_id)
                            .await
                            .with_context(|| {
                                format!("failed to apply queued update of {}", op.mod_name)
                            })
                    {
                        let remaining = pending.len() - applied - 1;
                        eprintln!("\n  Error: {e:#}");
                        eprintln!(
                            "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                        );
                        return Err(e);
                    }
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
