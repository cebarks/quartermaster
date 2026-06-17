use anyhow::Result;
use serde::Serialize;

use super::common::{find_unmanaged_mod_dirs, truncate_str, CliContext};

#[derive(Serialize)]
struct ModEntry {
    name: String,
    version: String,
    forge_mod_id: i64,
    slug: Option<String>,
    file_count: usize,
    installed_at: String,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct UnmanagedEntry {
    directory: String,
    file_count: usize,
}

#[derive(Serialize)]
struct ListOutput {
    mods: Vec<ModEntry>,
    unmanaged: Vec<UnmanagedEntry>,
}

pub fn run(json: bool, ctx: &CliContext) -> Result<()> {
    let installed_mods = ctx.db.list_mods()?;

    // Count files per mod from the tracked files list (avoids N+1 DB queries)
    let all_tracked_files = ctx.db.get_all_tracked_files()?;
    let mut file_counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for f in &all_tracked_files {
        *file_counts.entry(f.mod_id).or_default() += 1;
    }

    let mut mod_entries = Vec::new();
    for m in &installed_mods {
        mod_entries.push(ModEntry {
            name: m.name.clone(),
            version: m.version.clone(),
            forge_mod_id: m.forge_mod_id,
            slug: m.slug.clone(),
            file_count: file_counts.get(&m.id).copied().unwrap_or(0),
            installed_at: m.installed_at.clone(),
            updated_at: m.updated_at.clone(),
        });
    }

    let (unmanaged_dirs, _) = find_unmanaged_mod_dirs(&ctx.spt_dir, &ctx.db)?;
    let unmanaged_entries: Vec<UnmanagedEntry> = unmanaged_dirs
        .into_iter()
        .map(|(dir, count)| UnmanagedEntry {
            directory: dir,
            file_count: count,
        })
        .collect();

    if json {
        let output = ListOutput {
            mods: mod_entries,
            unmanaged: unmanaged_entries,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Table output
    if mod_entries.is_empty() && unmanaged_entries.is_empty() {
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
        }
    }

    if !unmanaged_entries.is_empty() {
        println!("\nUnmanaged mods:");
        for entry in &unmanaged_entries {
            println!("  {} ({} files)", entry.directory, entry.file_count);
        }
        println!("\nUse `quma track <path> <forge_mod_id>` to manage them.");
    }

    Ok(())
}
