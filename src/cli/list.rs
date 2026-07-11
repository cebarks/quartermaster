use anyhow::Result;
use serde::Serialize;

use super::common::{find_unmanaged_mod_dirs, truncate_str, CliContext};

use crate::config::{FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID};

const INFRASTRUCTURE_FORGE_IDS: &[i64] = &[FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID];

#[derive(Serialize)]
struct ModEntry {
    name: String,
    version: String,
    forge_mod_id: Option<i64>,
    slug: Option<String>,
    file_count: usize,
    installed_at: String,
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    addons: Vec<AddonEntry>,
}

#[derive(Serialize)]
struct AddonEntry {
    name: String,
    version: String,
    forge_addon_id: i64,
    file_count: usize,
}

#[derive(Serialize)]
struct UnmanagedEntry {
    directory: String,
    file_count: usize,
}

#[derive(Serialize)]
struct ListOutput {
    infrastructure: Vec<ModEntry>,
    mods: Vec<ModEntry>,
    unmanaged: Vec<UnmanagedEntry>,
}

pub fn run(json: bool, ctx: &CliContext) -> Result<()> {
    let installed_mods = ctx.db.list_mods()?;

    let all_tracked_files = ctx.db.get_all_tracked_files()?;
    let mut file_counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut addon_file_counts: std::collections::HashMap<i64, usize> =
        std::collections::HashMap::new();
    for f in &all_tracked_files {
        if let Some(mid) = f.mod_id {
            *file_counts.entry(mid).or_default() += 1;
        }
        if let Some(aid) = f.addon_id {
            *addon_file_counts.entry(aid).or_default() += 1;
        }
    }

    // Fetch all addons and group by parent mod
    let all_addons = ctx.db.list_addons()?;
    let addons_by_parent: std::collections::HashMap<i64, Vec<_>> =
        all_addons
            .into_iter()
            .fold(std::collections::HashMap::new(), |mut map, addon| {
                map.entry(addon.parent_mod_id).or_default().push(addon);
                map
            });

    let mut infra_entries = Vec::new();
    let mut mod_entries = Vec::new();
    for m in &installed_mods {
        let addon_entries: Vec<AddonEntry> = addons_by_parent
            .get(&m.id)
            .map(|addons| {
                addons
                    .iter()
                    .map(|a| AddonEntry {
                        name: a.name.clone(),
                        version: a.version.clone(),
                        forge_addon_id: a.forge_addon_id,
                        file_count: addon_file_counts.get(&a.id).copied().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let entry = ModEntry {
            name: m.name.clone(),
            version: m.version.clone(),
            forge_mod_id: m.forge_mod_id,
            slug: m.slug.clone(),
            file_count: file_counts.get(&m.id).copied().unwrap_or(0),
            installed_at: m.installed_at.clone(),
            updated_at: m.updated_at.clone(),
            addons: addon_entries,
        };
        if m.forge_mod_id
            .is_some_and(|id| INFRASTRUCTURE_FORGE_IDS.contains(&id))
        {
            infra_entries.push(entry);
        } else {
            mod_entries.push(entry);
        }
    }

    let (unmanaged_dirs, _) = find_unmanaged_mod_dirs(&ctx.dirs, &ctx.db)?;
    let unmanaged_entries: Vec<UnmanagedEntry> = unmanaged_dirs
        .into_iter()
        .map(|(dir, count)| UnmanagedEntry {
            directory: dir,
            file_count: count,
        })
        .collect();

    if json {
        let output = ListOutput {
            infrastructure: infra_entries,
            mods: mod_entries,
            unmanaged: unmanaged_entries,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Infrastructure section
    if !infra_entries.is_empty() {
        println!("Infrastructure:");
        for entry in &infra_entries {
            println!("  {} {}", entry.name, entry.version);
        }
        println!();
    }

    // Table output
    if mod_entries.is_empty() && unmanaged_entries.is_empty() && infra_entries.is_empty() {
        println!("No mods installed and no unmanaged mods found.");
        return Ok(());
    }

    if !mod_entries.is_empty() {
        println!(
            "{:<30} {:<12} {:<8} {:<20}",
            "Name", "Version", "Files", "Installed"
        );
        println!("{}", "-".repeat(72));

        for entry in &mod_entries {
            let date = &entry.installed_at[..10.min(entry.installed_at.len())];
            println!(
                "{:<30} {:<12} {:<8} {:<20}",
                truncate_str(&entry.name, 29),
                entry.version,
                entry.file_count,
                date,
            );

            // Display nested addons with └─ prefix
            for addon in &entry.addons {
                println!(
                    "  └─ {:<26} {:<12} {:<8}",
                    truncate_str(&addon.name, 25),
                    addon.version,
                    addon.file_count,
                );
            }
        }
    }

    if !unmanaged_entries.is_empty() {
        println!("\nUnmanaged mods:");
        for entry in &unmanaged_entries {
            println!("  {} ({} files)", entry.directory, entry.file_count);
        }
        println!("\nManage them through the web UI or reinstall via Forge.");
    }

    Ok(())
}
