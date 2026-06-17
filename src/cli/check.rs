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

    for r in &results {
        let name = installed
            .iter()
            .find(|m| m.forge_mod_id == r.mod_id)
            .map(|m| m.name.as_str())
            .unwrap_or("unknown");

        match r.status.as_str() {
            "up_to_date" => up_to_date.push(name),
            "updated" => {
                has_updates = true;
                updatable.push((
                    name,
                    r.current_version.as_str(),
                    r.latest_version.as_deref().unwrap_or("?"),
                ));
            }
            "blocked" => blocked.push(name),
            "incompatible" => incompatible.push(name),
            _ => {}
        }
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
