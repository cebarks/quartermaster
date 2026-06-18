use anyhow::Result;

use super::common::CliContext;

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
        .map(|m| (m.forge_mod_id, m.version.clone()))
        .collect();

    let results = ctx
        .forge
        .check_updates(&check_list, &ctx.spt_info.spt_version)
        .await?;

    let mut has_updates = false;

    // Categorize results
    let mut up_to_date = Vec::new();
    let mut updatable = Vec::new();
    let mut blocked = Vec::new();
    let mut incompatible = Vec::new();

    // Process updates
    for update in &results.updates {
        has_updates = true;
        let name = installed
            .iter()
            .find(|m| m.forge_mod_id == update.current_version.mod_id)
            .map(|m| m.name.as_str())
            .unwrap_or("unknown");
        updatable.push((
            name,
            update.current_version.version.as_str(),
            update.recommended_version.version.as_str(),
        ));
    }

    // Process blocked updates
    for _blocked in &results.blocked_updates {
        // The blocked_updates field is a Vec<serde_json::Value> - we'd need to know the exact structure
        // For now, just count them
        blocked.push("(mod with blocked update)");
    }

    // Process incompatible mods
    for incompat in &results.incompatible_with_spt {
        let name = installed
            .iter()
            .find(|m| m.forge_mod_id == incompat.mod_id)
            .map(|m| m.name.as_str())
            .unwrap_or(&incompat.name);
        incompatible.push(name);
    }

    // Process up-to-date mods
    for _up in &results.up_to_date {
        // The up_to_date field is a Vec<serde_json::Value> - we'd need to know the exact structure
        // For now, just count them
        up_to_date.push("(up-to-date mod)");
    }

    if !updatable.is_empty() {
        println!("Updates available:");
        for (name, current, latest) in &updatable {
            println!("  {} — {} → {}", name, current, latest);
        }
    }

    if !blocked.is_empty() {
        println!("\nBlocked (dependency conflict):");
        for name in &blocked {
            println!("  {}", name);
        }
    }

    if !incompatible.is_empty() {
        println!("\nIncompatible with SPT {}:", ctx.spt_info.spt_version);
        for name in &incompatible {
            println!("  {}", name);
        }
    }

    if !up_to_date.is_empty() {
        println!("\nUp to date ({}):", up_to_date.len());
        for name in &up_to_date {
            println!("  ✓ {}", name);
        }
    }

    if has_updates {
        println!("\nRun `quma update` to apply updates.");
    }

    Ok(has_updates)
}
