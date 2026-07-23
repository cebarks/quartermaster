use anyhow::Result;

use super::common::CliContext;

/// Helper to extract a typed field from pending op metadata JSON.
fn metadata_i64(metadata: Option<&str>, key: &str) -> Option<i64> {
    metadata
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get(key)?.as_i64())
}

fn metadata_str(metadata: Option<&str>, key: &str) -> Option<String> {
    metadata
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get(key)?.as_str().map(String::from))
}

/// Apply all pending operations without prompting for confirmation.
/// Returns the number of operations successfully applied.
/// Used by `quma apply` (after confirmation) and auto-drain (server lifecycle).
///
/// All install/update ops are expected to have `archive_path` set (pre-staged by
/// the queue/staging functions). No Forge API calls are made at apply time.
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
                    let version_id = match op.forge_version_id {
                        Some(id) => id,
                        None => {
                            println!("    Skipped — no version ID for install operation");
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }
                    };

                    let archive_path = match op.archive_path.as_deref() {
                        Some(p) => p,
                        None => {
                            eprintln!(
                                "    Error: queued addon install for {} has no archive_path",
                                op.mod_name
                            );
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }
                    };
                    let archive = std::path::Path::new(archive_path);
                    if !archive.exists() {
                        eprintln!("    Error: queued archive not found at {archive_path}");
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }

                    // Skip if already installed
                    if ctx.db.get_addon_by_forge_id(forge_addon_id)?.is_some() {
                        println!("    Skipped — already installed");
                        ctx.db.delete_pending_op(op.id)?;
                        applied += 1;
                        continue;
                    }

                    // Extract parent mod ID and version from metadata
                    let parent_forge_mod_id =
                        match metadata_i64(op.metadata.as_deref(), "parent_forge_mod_id") {
                            Some(id) => id,
                            None => {
                                eprintln!(
                                    "    Error: addon op missing parent_forge_mod_id in metadata"
                                );
                                ctx.db.delete_pending_op(op.id)?;
                                continue;
                            }
                        };

                    let parent = match ctx.db.get_mod_by_forge_id(parent_forge_mod_id)? {
                        Some(p) => p,
                        None => {
                            eprintln!(
                                "    Error: parent mod not installed (forge_id {})",
                                parent_forge_mod_id
                            );
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }
                    };

                    let version_str =
                        crate::queue::extract_version_from_metadata(op.metadata.as_deref())
                            .unwrap_or_else(|| "unknown".to_string());
                    let mod_version_constraint =
                        metadata_str(op.metadata.as_deref(), "mod_version_constraint");

                    if let Err(e) =
                        crate::ops::install_addon_from_archive(&crate::ops::InstallAddonRequest {
                            db: &ctx.db,
                            dirs: &ctx.dirs,
                            config: &ctx.config,
                            forge_addon_id: Some(forge_addon_id),
                            parent_mod_id: parent.id,
                            version_id: Some(version_id),
                            name: &op.mod_name,
                            slug: None,
                            version: &version_str,
                            mod_version_constraint: mod_version_constraint.as_deref(),
                            archive_path: archive,
                            source: crate::ops::ModSource::Forge,
                            source_url: None,
                        })
                    {
                        let remaining = pending.len() - applied - 1;
                        eprintln!("\n  Error: {e:#}");
                        eprintln!(
                            "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                        );
                        return Err(e);
                    }
                }
                crate::db::users::QueueAction::Update => {
                    let version_id = match op.forge_version_id {
                        Some(id) => id,
                        None => {
                            println!("    Skipped — no version ID for update operation");
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }
                    };

                    let installed = match ctx.db.get_addon_by_forge_id(forge_addon_id)? {
                        Some(a) => a,
                        None => {
                            println!("    Skipped — addon not found in database");
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }
                    };

                    let archive_path = match op.archive_path.as_deref() {
                        Some(p) => p,
                        None => {
                            eprintln!(
                                "    Error: queued addon update for {} has no archive_path",
                                op.mod_name
                            );
                            ctx.db.delete_pending_op(op.id)?;
                            continue;
                        }
                    };
                    let archive = std::path::Path::new(archive_path);
                    if !archive.exists() {
                        eprintln!("    Error: queued archive not found at {archive_path}");
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }

                    let version_str =
                        crate::queue::extract_version_from_metadata(op.metadata.as_deref())
                            .unwrap_or_else(|| "unknown".to_string());
                    let mod_version_constraint =
                        metadata_str(op.metadata.as_deref(), "mod_version_constraint");

                    if let Err(e) = crate::ops::update_addon_from_archive(
                        &ctx.db,
                        &ctx.dirs,
                        &ctx.config,
                        installed.id,
                        version_id,
                        &version_str,
                        mod_version_constraint.as_deref(),
                        archive,
                    ) {
                        let remaining = pending.len() - applied - 1;
                        eprintln!("\n  Error: {e:#}");
                        eprintln!(
                            "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                        );
                        return Err(e);
                    }
                }
                crate::db::users::QueueAction::Remove => {
                    if let Some(installed) = ctx.db.get_addon_by_forge_id(forge_addon_id)? {
                        if let Err(e) = crate::ops::remove_addon_by_id(
                            &ctx.db,
                            &ctx.dirs,
                            &ctx.config,
                            installed.id,
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

            crate::queue::cleanup_queued_archive(op);
            ctx.db.delete_pending_op(op.id)?;
            applied += 1;
            continue;
        }

        match op.action {
            crate::db::users::QueueAction::Install => {
                let archive_path = match op.archive_path.as_deref() {
                    Some(p) => p,
                    None => {
                        eprintln!(
                            "    Error: queued install for {} has no archive_path",
                            op.mod_name
                        );
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }
                };
                let archive = std::path::Path::new(archive_path);
                if !archive.exists() {
                    eprintln!("    Error: queued archive not found at {archive_path}");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }

                let forge_mod_id = op.forge_mod_id;
                let version_id = op.forge_version_id;

                // Skip if already installed (dep may have been installed by a previous op)
                if let Some(fid) = forge_mod_id {
                    if ctx.db.get_mod_by_forge_id(fid)?.is_some() {
                        println!("    Skipped — already installed");
                        crate::queue::cleanup_queued_archive(op);
                        ctx.db.delete_pending_op(op.id)?;
                        applied += 1;
                        continue;
                    }
                }

                let version_str =
                    crate::queue::extract_version_from_metadata(op.metadata.as_deref())
                        .unwrap_or_else(|| "unknown".to_string());
                let source = crate::ops::ModSource::parse(&op.source)
                    .unwrap_or(crate::ops::ModSource::Forge);
                let queued_for = crate::queue::extract_queued_for(op.metadata.as_deref());

                let installed_db_id = match crate::ops::install_mod_from_archive(
                    &crate::ops::InstallRequest {
                        db: &ctx.db,
                        dirs: &ctx.dirs,
                        config: &ctx.config,
                        forge_mod_id,
                        version_id,
                        name: &op.mod_name,
                        slug: None,
                        version: &version_str,
                        archive_path: archive,
                        source,
                        source_url: op.source_url.as_deref(),
                    },
                ) {
                    Ok(db_id) => db_id,
                    Err(e) => {
                        let remaining = pending.len() - applied - 1;
                        eprintln!("\n  Error: {e:#}");
                        eprintln!(
                            "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                        );
                        return Err(e);
                    }
                };

                // Record dependency edges: if this op was queued as a dep
                // (has queued_for), each entry is a parent forge_mod_id.
                for parent_forge_mod_id in &queued_for {
                    if let Ok(Some(parent)) = ctx.db.get_mod_by_forge_id(*parent_forge_mod_id) {
                        match ctx.db.insert_dependency(
                            parent.id,
                            Some(installed_db_id),
                            op.forge_mod_id,
                            Some(&op.mod_name),
                            None,
                        ) {
                            Ok(_) => {}
                            Err(rusqlite::Error::SqliteFailure(err, _))
                                if err.code == rusqlite::ffi::ErrorCode::ConstraintViolation => {}
                            Err(e) => {
                                tracing::warn!(
                                    parent_id = parent.id,
                                    dep_id = installed_db_id,
                                    err = %e,
                                    "failed to record dependency edge from queue"
                                );
                            }
                        }
                    }
                }

                println!("    Installed {} from {}", op.mod_name, op.source);
            }
            crate::db::users::QueueAction::Remove => {
                let forge_mod_id = op.forge_mod_id.expect("mod remove must have forge_mod_id");

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
                let forge_mod_id = match op.forge_mod_id {
                    Some(id) => id,
                    None => {
                        eprintln!("    Error: update op missing forge_mod_id");
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }
                };
                let version_id = match op.forge_version_id {
                    Some(id) => id,
                    None => {
                        println!("    Skipped — mod not found or no version ID");
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }
                };

                let installed = match ctx.db.get_mod_by_forge_id(forge_mod_id)? {
                    Some(m) => m,
                    None => {
                        println!("    Skipped — mod not found in database");
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }
                };

                let archive_path = match op.archive_path.as_deref() {
                    Some(p) => p,
                    None => {
                        eprintln!(
                            "    Error: queued update for {} has no archive_path",
                            op.mod_name
                        );
                        ctx.db.delete_pending_op(op.id)?;
                        continue;
                    }
                };
                let archive = std::path::Path::new(archive_path);
                if !archive.exists() {
                    eprintln!("    Error: queued archive not found at {archive_path}");
                    ctx.db.delete_pending_op(op.id)?;
                    continue;
                }

                let version_str =
                    crate::queue::extract_version_from_metadata(op.metadata.as_deref())
                        .unwrap_or_else(|| "unknown".to_string());

                if let Err(e) = crate::ops::update_mod_from_archive(
                    &ctx.db,
                    &ctx.dirs,
                    &ctx.config,
                    installed.id,
                    version_id,
                    &version_str,
                    archive,
                ) {
                    let remaining = pending.len() - applied - 1;
                    eprintln!("\n  Error: {e:#}");
                    eprintln!(
                        "  {applied} operation(s) applied, 1 failed, {remaining} remaining in queue."
                    );
                    return Err(e);
                }
            }
        }

        crate::queue::cleanup_queued_archive(op);
        ctx.db.delete_pending_op(op.id)?;
        applied += 1;
    }

    Ok(applied)
}
