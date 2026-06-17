use anyhow::Result;

use crate::db::mods::InstalledMod;
use crate::spt::mods::{delete_mod_files, extract_mod};

use super::common::{confirm, resolve_installed_mod, CliContext};

pub async fn run(mod_ref: Option<&str>, _force: bool, ctx: &CliContext) -> Result<()> {
    // TODO(debt): _force is accepted but unused until Phase 3 wires server-running detection
    let mods_to_check: Vec<InstalledMod> = match mod_ref {
        Some(r) => vec![resolve_installed_mod(r, ctx)?],
        None => ctx.db.list_mods()?,
    };

    if mods_to_check.is_empty() {
        println!("No mods installed. Use `quma install` to install mods.");
        return Ok(());
    }

    let check_list: Vec<(i64, String)> = mods_to_check
        .iter()
        .map(|m| (m.forge_mod_id, m.version.clone()))
        .collect();

    let results = ctx
        .forge
        .check_updates(&check_list, &ctx.spt_info.spt_version)
        .await?;

    let updatable: Vec<_> = results.iter().filter(|r| r.status == "updated").collect();

    if updatable.is_empty() {
        println!("All mods are up to date.");
        report_non_updatable(&results, &mods_to_check, &ctx.spt_info.spt_version);
        return Ok(());
    }

    display_update_plan(&updatable, &mods_to_check);

    if !confirm("Proceed with updates?")? {
        println!("Update cancelled.");
        return Ok(());
    }

    let mut updated_count = 0;
    for update_result in &updatable {
        if apply_single_update(update_result, &mods_to_check, ctx).await? {
            updated_count += 1;
        }
    }

    println!("\n{} mod(s) updated.", updated_count);
    Ok(())
}

fn report_non_updatable(
    results: &[crate::forge::models::UpdateCheckResult],
    mods: &[InstalledMod],
    spt_version: &str,
) {
    for r in results {
        match r.status.as_str() {
            "blocked" => println!(
                "  {} — blocked (dependency conflict)",
                mod_name_for_id(mods, r.mod_id)
            ),
            "incompatible" => println!(
                "  {} — incompatible with SPT {}",
                mod_name_for_id(mods, r.mod_id),
                spt_version
            ),
            _ => {}
        }
    }
}

fn display_update_plan(
    updatable: &[&crate::forge::models::UpdateCheckResult],
    mods: &[InstalledMod],
) {
    println!("Updates available:");
    for r in updatable {
        println!(
            "  {} — {} → {}",
            mod_name_for_id(mods, r.mod_id),
            r.current_version,
            r.latest_version.as_deref().unwrap_or("?")
        );
    }
}

async fn apply_single_update(
    update_result: &crate::forge::models::UpdateCheckResult,
    mods: &[InstalledMod],
    ctx: &CliContext,
) -> Result<bool> {
    let installed = mods
        .iter()
        .find(|m| m.forge_mod_id == update_result.mod_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "update result references unknown mod ID {}",
                update_result.mod_id
            )
        })?;

    let latest_version_id = match update_result.latest_version_id {
        Some(id) => id,
        None => {
            println!(
                "  Skipping {} — no version ID in update response",
                installed.name
            );
            return Ok(false);
        }
    };

    let latest_version_str = update_result.latest_version.as_deref().unwrap_or("unknown");

    let versions = ctx.forge.get_versions(installed.forge_mod_id, None).await?;
    let version_info = match versions.iter().find(|v| v.id == latest_version_id) {
        Some(v) => v,
        None => {
            println!(
                "  Skipping {} — version {} not found",
                installed.name, latest_version_id
            );
            return Ok(false);
        }
    };

    let download_url = match &version_info.link {
        Some(url) => url.clone(),
        None => {
            println!(
                "  Skipping {} — no download link for v{}",
                installed.name, latest_version_str
            );
            return Ok(false);
        }
    };

    println!(
        "\nUpdating {} to v{}...",
        installed.name, latest_version_str
    );

    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    ctx.forge
        .download_file(&download_url, &archive_path)
        .await?;

    // Extract to staging dir first — if this fails, old files are untouched
    let staging_dir = tempfile::tempdir()?;
    let new_files = extract_mod(&archive_path, staging_dir.path())?;

    // Extraction succeeded — now safe to remove old files
    let old_files = ctx.db.get_files_for_mod(installed.id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    delete_mod_files(&ctx.spt_dir, &old_paths)?;
    ctx.db.delete_files_for_mod(installed.id)?;

    // Move staged files to SPT dir
    for file in &new_files {
        let src = staging_dir.path().join(&file.path);
        let dest = ctx.spt_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dest).or_else(|_| {
            // rename fails across mount points; fall back to copy+delete
            std::fs::copy(&src, &dest).map(|_| ())
        })?;
    }

    for file in &new_files {
        ctx.db.insert_file(
            installed.id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }

    ctx.db
        .update_mod(installed.id, latest_version_id, latest_version_str)?;
    println!("  Updated {} files for {}", new_files.len(), installed.name);
    Ok(true)
}

fn mod_name_for_id(mods: &[InstalledMod], forge_mod_id: i64) -> &str {
    mods.iter()
        .find(|m| m.forge_mod_id == forge_mod_id)
        .map(|m| m.name.as_str())
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_name_for_id_finds_match() {
        let mods = vec![InstalledMod {
            id: 1,
            forge_mod_id: 100,
            forge_version_id: 200,
            name: "TestMod".to_string(),
            slug: None,
            version: "1.0.0".to_string(),
            installed_at: "2025-01-01".to_string(),
            updated_at: None,
        }];

        assert_eq!(mod_name_for_id(&mods, 100), "TestMod");
    }

    #[test]
    fn mod_name_for_id_returns_unknown_on_miss() {
        let mods: Vec<InstalledMod> = vec![];
        assert_eq!(mod_name_for_id(&mods, 999), "unknown");
    }
}
