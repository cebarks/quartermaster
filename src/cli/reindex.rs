use std::path::Path;

use anyhow::{Context, Result};

use super::common::CliContext;
use crate::db::mods::InstalledMod;
use crate::spt::mods::{compute_file_hash, detect_strip_prefix, list_entry_names};

/// Rebuild the `installed_files` index by re-downloading archives from Forge
/// and matching their contents against files on disk.
pub async fn run(dry_run: bool, ctx: &CliContext) -> Result<()> {
    let mods = ctx.db.list_mods()?;
    let needs_reindex: Vec<&InstalledMod> = mods
        .iter()
        .filter(|m| {
            m.forge_mod_id.is_some()
                && m.forge_version_id.is_some()
                && ctx
                    .db
                    .get_files_for_mod(m.id)
                    .map(|f| f.is_empty())
                    .unwrap_or(true)
        })
        .collect();

    if needs_reindex.is_empty() {
        println!("All mods already have tracked files.");
        return Ok(());
    }

    println!(
        "{} mod(s) with no tracked files{}:",
        needs_reindex.len(),
        if dry_run { " (dry run)" } else { "" }
    );
    for m in &needs_reindex {
        println!("  {} (v{})", m.name, m.version);
    }
    println!();

    let mut indexed = 0;
    let mut skipped = Vec::new();

    for m in &needs_reindex {
        let (forge_mod_id, forge_version_id) = match (m.forge_mod_id, m.forge_version_id) {
            (Some(mid), Some(vid)) => (mid, vid),
            _ => continue,
        };

        print!("  {} ... ", m.name);

        // Fetch the specific version to get the download link
        let versions = match ctx.forge.get_versions(forge_mod_id, None).await {
            Ok(v) => v,
            Err(e) => {
                println!("skip (API error: {e})");
                skipped.push((m.name.as_str(), format!("API error: {e}")));
                continue;
            }
        };

        let version = match versions.iter().find(|v| v.id == forge_version_id) {
            Some(v) => v,
            None => {
                println!("skip (version {} not found on Forge)", forge_version_id);
                skipped.push((m.name.as_str(), "version not found on Forge".into()));
                continue;
            }
        };

        let link = match version.link.as_deref() {
            Some(l) => l,
            None => {
                println!("skip (no download link)");
                skipped.push((m.name.as_str(), "no download link".into()));
                continue;
            }
        };

        // Download to a tempdir
        let tmp_dir = tempfile::tempdir()?;
        let archive_path = tmp_dir.path().join("mod.archive");
        if let Err(e) = ctx.forge.download_file(link, &archive_path).await {
            println!("skip (download failed: {e})");
            skipped.push((m.name.as_str(), format!("download failed: {e}")));
            continue;
        }

        // List archive entries and apply prefix stripping
        match reindex_mod_from_archive(&ctx.db, &ctx.dirs.spt_server, m.id, &archive_path, dry_run)
        {
            Ok(count) => {
                println!("{count} file(s)");
                indexed += 1;
            }
            Err(e) => {
                println!("skip (index error: {e})");
                skipped.push((m.name.as_str(), format!("index error: {e}")));
            }
        }
    }

    println!();
    if dry_run {
        println!(
            "Dry run complete: {indexed}/{} mod(s) would be reindexed.",
            needs_reindex.len()
        );
        println!("Run with --apply to commit changes.");
    } else {
        println!("Reindexed {indexed}/{} mod(s).", needs_reindex.len());
    }

    if !skipped.is_empty() {
        println!("\nSkipped:");
        for (name, reason) in &skipped {
            println!("  {name} — {reason}");
        }
    }

    Ok(())
}

/// List files from an archive, match against disk, and insert into DB.
/// Returns the number of files indexed.
fn reindex_mod_from_archive(
    db: &crate::db::Database,
    spt_dir: &Path,
    mod_db_id: i64,
    archive_path: &Path,
    dry_run: bool,
) -> Result<usize> {
    let prefix = detect_strip_prefix(archive_path)?;
    let entries = list_entry_names(archive_path)?;

    let file_paths: Vec<String> = entries
        .iter()
        .filter_map(|name| {
            // Apply prefix stripping (same as extract_mod)
            let effective = if !prefix.is_empty() && name.starts_with(&prefix) {
                &name[prefix.len()..]
            } else {
                name
            };

            // Skip directories
            if effective.ends_with('/') || effective.is_empty() {
                return None;
            }

            // Only index files under known mod directories
            if effective.starts_with("SPT/user/mods/") || effective.starts_with("BepInEx/plugins/")
            {
                Some(effective.to_string())
            } else {
                None
            }
        })
        .collect();

    if file_paths.is_empty() {
        anyhow::bail!("archive contains no recognizable mod files");
    }

    let mut count = 0;
    let tx = if !dry_run {
        Some(db.begin_transaction()?)
    } else {
        None
    };

    for rel_path in &file_paths {
        let disk_path = spt_dir.join(rel_path);
        if !disk_path.exists() {
            continue;
        }

        if !dry_run {
            let hash = compute_file_hash(&disk_path)
                .with_context(|| format!("failed to hash {}", rel_path))?;
            let size = std::fs::metadata(&disk_path)
                .map(|m| m.len() as i64)
                .unwrap_or(0);
            db.insert_file(mod_db_id, rel_path, Some(&hash), Some(size))?;
        }
        count += 1;
    }

    if let Some(tx) = tx {
        tx.commit()?;
    }

    Ok(count)
}

pub async fn run_deps(apply: bool, ctx: &CliContext) -> Result<()> {
    let mods = ctx.db.list_mods()?;
    let forge_mods: Vec<&InstalledMod> = mods.iter().filter(|m| m.forge_mod_id.is_some()).collect();

    if forge_mods.is_empty() {
        println!("No Forge-installed mods found.");
        return Ok(());
    }

    println!(
        "Resolving dependencies for {} mod(s){}...",
        forge_mods.len(),
        if !apply { " (dry run)" } else { "" }
    );

    let mut total_edges = 0u64;

    for m in &forge_mods {
        let forge_mod_id = m.forge_mod_id.expect("filtered above");
        let version = &m.version;

        let dep_nodes = match ctx
            .forge
            .get_dependencies(&[(&forge_mod_id.to_string(), version)])
            .await
        {
            Ok(nodes) => nodes,
            Err(e) => {
                println!("  {} — failed to resolve: {e}", m.name);
                continue;
            }
        };

        let edge_count = count_dep_nodes(&dep_nodes);
        if edge_count > 0 {
            println!("  {} — {} dependency edge(s)", m.name, edge_count);
            total_edges += edge_count as u64;

            if apply {
                crate::cli::install::record_dependency_edges_from_tree(&ctx.db, m.id, &dep_nodes);
            }
        }
    }

    if total_edges == 0 {
        println!("\nNo dependency edges found.");
    } else if apply {
        println!("\nRecorded {total_edges} dependency edge(s).");
    } else {
        println!("\nWould record {total_edges} edge(s). Run with --apply to commit.");
    }

    Ok(())
}

fn count_dep_nodes(nodes: &[crate::forge::models::DependencyNode]) -> usize {
    nodes
        .iter()
        .filter(|n| !n.conflict)
        .map(|n| 1 + count_dep_nodes(&n.dependencies))
        .sum()
}
