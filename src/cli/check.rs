use anyhow::Result;

use crate::db::mods::InstalledMod;

use super::common::CliContext;

/// Extract a mod name from a `serde_json::Value` by looking for `mod_id` and
/// falling back to a `name` field or the raw JSON.
fn name_from_value(val: &serde_json::Value, installed: &[InstalledMod]) -> String {
    if let Some(mod_id) = val.get("mod_id").and_then(|v| v.as_i64()) {
        if let Some(m) = installed.iter().find(|m| m.forge_mod_id == Some(mod_id)) {
            return m.name.clone();
        }
    }
    if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
        return name.to_string();
    }
    val.to_string()
}

/// Returns Ok(true) if updates are available, Ok(false) if all up to date.
/// Caller (main.rs) maps true → exit code 1.
pub async fn run(ctx: &CliContext) -> Result<bool> {
    let installed = ctx.db.list_mods()?;

    if installed.is_empty() {
        println!("No mods installed.");
        return Ok(false);
    }

    let check_list: Vec<(i64, String)> = installed
        .iter()
        .filter_map(|m| m.forge_mod_id.map(|id| (id, m.version.clone())))
        .collect();

    let results = ctx
        .forge
        .check_updates(&check_list, &ctx.spt_info.spt_version)
        .await?;

    let has_updates = !results.updates.is_empty();

    if has_updates {
        println!("Updates available:");
        for update in &results.updates {
            let name = installed
                .iter()
                .find(|m| m.forge_mod_id == Some(update.current_version.mod_id))
                .map(|m| m.name.as_str())
                .unwrap_or("unknown");
            println!(
                "  {} — {} → {}",
                name, update.current_version.version, update.recommended_version.version
            );
        }
    }

    if !results.blocked_updates.is_empty() {
        println!("\nBlocked (dependency conflict):");
        for val in &results.blocked_updates {
            println!("  {}", name_from_value(val, &installed));
        }
    }

    if !results.incompatible_with_spt.is_empty() {
        println!("\nIncompatible with SPT {}:", ctx.spt_info.spt_version);
        for incompat in &results.incompatible_with_spt {
            let name = installed
                .iter()
                .find(|m| m.forge_mod_id == Some(incompat.mod_id))
                .map(|m| m.name.as_str())
                .unwrap_or(&incompat.name);
            println!("  {}", name);
        }
    }

    if !results.up_to_date.is_empty() {
        println!("\nUp to date:");
        for val in &results.up_to_date {
            println!("  {}", name_from_value(val, &installed));
        }
    }

    if has_updates {
        println!("\nRun `quma update` to apply updates.");
    }

    Ok(has_updates)
}
