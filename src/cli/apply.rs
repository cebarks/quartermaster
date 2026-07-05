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

        if op.item_type == "addon" {
            let forge_addon_id = op
                .forge_addon_id
                .expect("addon operation must have forge_addon_id");

            match op.action {
                crate::db::users::QueueAction::Install => {
                    if let Some(version_id) = op.forge_version_id {
                        // Resolve addon from Forge API
                        let addon = match ctx.forge.get_addon(forge_addon_id, true).await {
                            Ok(a) => a,
                            Err(e) => {
                                eprintln!("    Error: failed to fetch addon from Forge: {e:#}");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let parent_forge_mod_id = match addon.mod_id {
                            Some(id) => id,
                            None => {
                                eprintln!("    Error: detached addons are not supported");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let parent = match ctx.db.get_mod_by_forge_id(parent_forge_mod_id)? {
                            Some(p) => p,
                            None => {
                                eprintln!(
                                    "    Error: parent mod not installed for addon {}",
                                    addon.name
                                );
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let version = match addon
                            .versions
                            .as_ref()
                            .and_then(|vs| vs.iter().find(|v| v.id == version_id))
                        {
                            Some(v) => v,
                            None => {
                                eprintln!("    Error: addon version not found");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let download_url = match version.link.as_deref() {
                            Some(url) => url,
                            None => {
                                eprintln!("    Error: no download URL for addon version");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        // Download and install
                        let tmp_dir = match tempfile::tempdir() {
                            Ok(d) => d,
                            Err(e) => {
                                eprintln!("    Error: failed to create temp dir: {e:#}");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };
                        let archive_path = tmp_dir.path().join("addon.zip");
                        if let Err(e) = ctx.forge.download_file(download_url, &archive_path).await {
                            eprintln!("    Error: failed to download addon: {e:#}");
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }

                        if let Err(e) = crate::ops::install_addon_from_archive(
                            &crate::ops::InstallAddonRequest {
                                db: &ctx.db,
                                spt_dir: &ctx.spt_dir,
                                config: &ctx.config,
                                forge_addon_id: Some(forge_addon_id),
                                parent_mod_id: parent.id,
                                version_id: Some(version_id),
                                name: &addon.name,
                                slug: addon.slug.as_deref(),
                                version: &version.version,
                                mod_version_constraint: version.mod_version_constraint.as_deref(),
                                archive_path: &archive_path,
                                source: crate::ops::ModSource::Forge,
                                source_url: None,
                            },
                        ) {
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
                crate::db::users::QueueAction::Update => {
                    if let (Some(installed), Some(version_id)) = (
                        ctx.db.get_addon_by_forge_id(forge_addon_id)?,
                        op.forge_version_id,
                    ) {
                        let versions = match ctx.forge.get_addon_versions(forge_addon_id).await {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("    Error: failed to fetch addon versions: {e:#}");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let version = match versions.iter().find(|v| v.id == version_id) {
                            Some(v) => v,
                            None => {
                                eprintln!("    Error: addon version not found");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let download_url = match version.link.as_deref() {
                            Some(url) => url,
                            None => {
                                eprintln!("    Error: no download URL for addon version");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                        let tmp_dir = match tempfile::tempdir() {
                            Ok(d) => d,
                            Err(e) => {
                                eprintln!("    Error: failed to create temp dir: {e:#}");
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };
                        let archive_path = tmp_dir.path().join("addon.zip");
                        if let Err(e) = ctx.forge.download_file(download_url, &archive_path).await {
                            eprintln!("    Error: failed to download addon: {e:#}");
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }

                        if let Err(e) = crate::ops::update_addon_from_archive(
                            &ctx.db,
                            &ctx.spt_dir,
                            &ctx.config,
                            installed.id,
                            version_id,
                            &version.version,
                            version.mod_version_constraint.as_deref(),
                            &archive_path,
                        ) {
                            let remaining = pending.len() - applied - 1;
                            eprintln!("\n  Error: {e:#}");
                            eprintln!(
                                "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                            );
                            return Err(e);
                        }
                    } else {
                        println!("    Skipped — addon not found or no version ID");
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }
                }
                crate::db::users::QueueAction::Remove => {
                    if let Some(installed) = ctx.db.get_addon_by_forge_id(forge_addon_id)? {
                        if let Err(e) = crate::ops::remove_addon_by_id(
                            &ctx.db,
                            &ctx.spt_dir,
                            &ctx.config,
                            installed.id,
                            false,
                        ) {
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
            }

            ctx.db.delete_pending_op(op.id)?;
            applied += 1;
            continue;
        }

        let forge_mod_id = op
            .forge_mod_id
            .expect("mod operation must have forge_mod_id");

        match op.action {
            crate::db::users::QueueAction::Install => {
                if let Some(version_id) = op.forge_version_id {
                    if let Err(e) =
                        crate::cli::install::install_with_deps(ctx, forge_mod_id, version_id)
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
            crate::db::users::QueueAction::Remove => {
                if let Some(installed) = ctx.db.get_mod_by_forge_id(forge_mod_id)? {
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
            crate::db::users::QueueAction::Update => {
                if let (Some(installed), Some(version_id)) = (
                    ctx.db.get_mod_by_forge_id(forge_mod_id)?,
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
        }

        ctx.db.delete_pending_op(op.id)?;
        applied += 1;
    }

    Ok(applied)
}
