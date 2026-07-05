use anyhow::Result;

use crate::db::mods::InstalledMod;

use super::common::{confirm, resolve_installed_mod, CliContext};

pub async fn run(mod_ref: Option<&str>, force: bool, addon: bool, ctx: &CliContext) -> Result<()> {
    // If --addon flag is set, use addon update flow
    if addon {
        let addon_ref = mod_ref.ok_or_else(|| {
            anyhow::anyhow!(
                "addon reference required for --addon flag (e.g., `quma update --addon <name>`)"
            )
        })?;
        return run_addon_update(addon_ref, force, ctx).await;
    }
    // Spec: `quma update` drains pending operations before checking for updates
    let pending = ctx.db.list_pending_ops()?;
    if !pending.is_empty() {
        let running = crate::server_detect::is_server_running(
            &ctx.config,
            &ctx.spt_dir,
            ctx.container_mgr.as_ref(),
        )
        .await?;
        if running && !force {
            anyhow::bail!(
                "{} pending operation(s) queued — stop the server first or use --force.\n\
                 Stop the server and retry, or use --force.",
                pending.len()
            );
        }
        println!(
            "Draining {} pending operation(s) before checking updates...",
            pending.len()
        );
        crate::cli::apply::drain_all(ctx).await?;
    }

    let mods_to_check: Vec<InstalledMod> = match mod_ref {
        Some(r) => vec![resolve_installed_mod(r, ctx)?],
        None => {
            let all = ctx.db.list_mods()?;
            if ctx.config.update_disabled_mods {
                all
            } else {
                all.into_iter().filter(|m| !m.disabled).collect()
            }
        }
    };

    if mods_to_check.is_empty() {
        println!("No mods installed. Use `quma install` to install mods.");
        return Ok(());
    }

    let check_list: Vec<(i64, String)> = mods_to_check
        .iter()
        .filter_map(|m| m.forge_mod_id.map(|id| (id, m.version.clone())))
        .collect();

    let results = ctx
        .forge
        .check_updates(&check_list, &ctx.spt_info.spt_version)
        .await?;

    if results.updates.is_empty() {
        println!("All mods are up to date.");
        report_non_updatable(&results, &mods_to_check, &ctx.spt_info.spt_version);
        return Ok(());
    }

    display_update_plan(&results.updates, &mods_to_check);

    if !confirm("Proceed with updates?")? {
        println!("Update cancelled.");
        return Ok(());
    }

    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir, ctx.container_mgr.as_ref())
        .await?
    {
        for update in &results.updates {
            let installed = mods_to_check
                .iter()
                .find(|m| m.forge_mod_id == Some(update.current_version.mod_id));
            if let Some(m) = installed {
                ctx.db.insert_pending_op(
                    crate::db::users::QueueAction::Update,
                    m.forge_mod_id.expect("forge mod in update path"),
                    Some(update.recommended_version.id),
                    &m.name,
                    None,
                    None,
                )?;
            }
        }
        println!(
            "Server is running — {} update(s) queued. They will be applied on next server restart.",
            results.updates.len()
        );
        return Ok(());
    }

    super::common::warn_if_forcing_while_running(force, ctx).await?;

    let mut updated_count = 0;
    for update in &results.updates {
        if apply_single_update(update, &mods_to_check, ctx).await? {
            updated_count += 1;
        }
    }

    println!("\n{} mod(s) updated.", updated_count);
    Ok(())
}

fn report_non_updatable(
    results: &crate::forge::models::UpdatesResponseData,
    mods: &[InstalledMod],
    spt_version: &str,
) {
    if !results.blocked_updates.is_empty() {
        println!(
            "  {} mod(s) blocked (dependency conflict)",
            results.blocked_updates.len()
        );
    }

    for incompat in &results.incompatible_with_spt {
        println!(
            "  {} — incompatible with SPT {}",
            mod_name_for_id(mods, incompat.mod_id),
            spt_version
        );
    }
}

fn display_update_plan(updates: &[crate::forge::models::UpdateEntry], mods: &[InstalledMod]) {
    println!("Updates available:");
    for update in updates {
        println!(
            "  {} — {} → {}",
            mod_name_for_id(mods, update.current_version.mod_id),
            update.current_version.version,
            update.recommended_version.version
        );
    }
}

/// Download, extract, and swap files for a specific version.
/// Used by both the interactive `update` command and `apply::drain_all`.
pub async fn apply_update_by_version(
    ctx: &CliContext,
    installed: &InstalledMod,
    target_version_id: i64,
) -> Result<bool> {
    let versions = ctx
        .forge
        .get_versions(
            installed.forge_mod_id.expect("forge mod in update path"),
            None,
        )
        .await?;
    let version_info = match versions.iter().find(|v| v.id == target_version_id) {
        Some(v) => v,
        None => {
            println!(
                "    Skipping {} — version {} not found",
                installed.name, target_version_id
            );
            return Ok(false);
        }
    };

    let download_url = match &version_info.link {
        Some(url) => url.clone(),
        None => {
            println!(
                "    Skipping {} — no download link for v{}",
                installed.name, version_info.version
            );
            return Ok(false);
        }
    };

    println!(
        "  Updating {} to v{}...",
        installed.name, version_info.version
    );

    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    ctx.forge
        .download_file(&download_url, &archive_path)
        .await?;

    crate::ops::update_mod_from_archive(
        &ctx.db,
        &ctx.spt_dir,
        &ctx.config,
        installed.id,
        target_version_id,
        &version_info.version,
        &archive_path,
    )?;

    let file_count = ctx.db.get_files_for_mod(installed.id)?.len();
    println!("    Updated {} files for {}", file_count, installed.name);

    // Re-fetch the mod to get the updated version for compatibility check
    let updated_mod = ctx
        .db
        .get_mod(installed.id)?
        .expect("mod must exist after update");

    // Check for incompatible addons after parent mod update
    check_addon_compatibility_after_update(ctx, &updated_mod).await?;

    Ok(true)
}

async fn check_addon_compatibility_after_update(
    ctx: &CliContext,
    parent_mod: &InstalledMod,
) -> Result<()> {
    let child_addons = ctx.db.list_addons_for_mod(parent_mod.id)?;
    if child_addons.is_empty() {
        return Ok(());
    }

    for addon in &child_addons {
        if let Some(constraint) = &addon.mod_version_constraint {
            if !super::common::version_satisfies_constraint(&parent_mod.version, constraint) {
                tracing::warn!(
                    "Addon {} (constraint '{}') does not match parent mod version '{}'",
                    addon.name,
                    constraint,
                    parent_mod.version
                );
            }
        }
    }

    Ok(())
}

async fn run_addon_update(addon_ref: &str, force: bool, ctx: &CliContext) -> Result<()> {
    use super::common::resolve_installed_addon;

    let installed = resolve_installed_addon(addon_ref, ctx)?;
    println!("Checking for updates to {}...", installed.name);

    let versions = ctx
        .forge
        .get_addon_versions(installed.forge_addon_id)
        .await?;

    if versions.is_empty() {
        println!("No versions available for {}", installed.name);
        return Ok(());
    }

    // Find the latest version
    let latest = versions
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no versions available"))?;

    // Check if update is available
    if latest.id == installed.forge_version_id {
        println!(
            "{} is already up to date (v{})",
            installed.name, installed.version
        );
        return Ok(());
    }

    println!(
        "Update available: {} → {}",
        installed.version, latest.version
    );

    if !confirm("Proceed with update?")? {
        println!("Update cancelled.");
        return Ok(());
    }

    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir, ctx.container_mgr.as_ref())
        .await?
    {
        ctx.db.insert_pending_addon_op(
            crate::db::users::QueueAction::Update,
            installed.forge_addon_id,
            Some(latest.id),
            &installed.name,
            None,
            None,
        )?;
        println!(
            "Server is running — operation queued. It will be applied on next server restart."
        );
        return Ok(());
    }

    super::common::warn_if_forcing_while_running(force, ctx).await?;

    // Download and update
    let download_url = latest
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no download URL for addon version"))?;

    println!("Downloading {} v{}...", installed.name, latest.version);
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("addon.zip");
    ctx.forge.download_file(download_url, &archive_path).await?;

    crate::ops::update_addon_from_archive(
        &ctx.db,
        &ctx.spt_dir,
        &ctx.config,
        installed.id,
        latest.id,
        &latest.version,
        latest.mod_version_constraint.as_deref(),
        &archive_path,
    )?;

    println!("\n{} updated to v{}", installed.name, latest.version);
    Ok(())
}

async fn apply_single_update(
    update: &crate::forge::models::UpdateEntry,
    mods: &[InstalledMod],
    ctx: &CliContext,
) -> Result<bool> {
    let installed = mods
        .iter()
        .find(|m| m.forge_mod_id == Some(update.current_version.mod_id))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "update result references unknown mod ID {}",
                update.current_version.mod_id
            )
        })?;

    apply_update_by_version(ctx, installed, update.recommended_version.id).await
}

fn mod_name_for_id(mods: &[InstalledMod], forge_mod_id: i64) -> &str {
    mods.iter()
        .find(|m| m.forge_mod_id == Some(forge_mod_id))
        .map(|m| m.name.as_str())
        .unwrap_or("unknown")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn mod_name_for_id_finds_match() {
        let mods = vec![InstalledMod {
            id: 1,
            forge_mod_id: Some(100),
            forge_version_id: Some(200),
            name: "TestMod".to_string(),
            slug: None,
            version: "1.0.0".to_string(),
            installed_at: "2025-01-01".to_string(),
            updated_at: None,
            disabled: false,
            source: "forge".to_string(),
            source_url: None,
            group_id: None,
        }];

        assert_eq!(mod_name_for_id(&mods, 100), "TestMod");
    }

    #[test]
    fn mod_name_for_id_returns_unknown_on_miss() {
        let mods: Vec<InstalledMod> = vec![];
        assert_eq!(mod_name_for_id(&mods, 999), "unknown");
    }
}
