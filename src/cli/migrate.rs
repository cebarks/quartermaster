use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;

use crate::config::Config;
use crate::spt::detect::validate_spt_dir;

pub async fn run(dry_run: bool, cli: &crate::cli::Cli) -> Result<()> {
    let root = resolve_legacy_root(cli)?;

    // Verify this is actually a legacy layout
    validate_spt_dir(&root).context("This doesn't look like a legacy SPT directory")?;

    if root.join("spt-server").exists() && validate_spt_dir(&root.join("spt-server")).is_ok() {
        bail!("This directory already uses the new layout (spt-server/ exists and is valid)");
    }

    let moves = plan_moves(&root)?;

    println!("\nMigration plan:");
    println!("{:<50} → Destination", "Source");
    println!("{}", "─".repeat(90));
    for (src, dst) in &moves {
        let src_rel = src.strip_prefix(&root).unwrap_or(src);
        let dst_rel = dst.strip_prefix(&root).unwrap_or(dst);
        println!("{:<50} → {}", src_rel.display(), dst_rel.display());
    }

    if dry_run {
        println!("\nDry run — no changes made.");
        return Ok(());
    }

    print!("\nProceed with migration? [y/N]: ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Migration cancelled.");
        return Ok(());
    }

    // Write marker
    let marker = root.join(".migration-in-progress");
    std::fs::write(&marker, "").context("failed to write migration marker")?;

    // Create spt-server/ directory
    let spt_dest = root.join("spt-server");
    std::fs::create_dir_all(&spt_dest)?;

    // Move SPT directories into spt-server/
    move_dir(&root.join("SPT"), &spt_dest.join("SPT"))?;
    move_dir(&root.join("BepInEx"), &spt_dest.join("BepInEx"))?;

    // Flatten quma internal dirs
    flatten_quma_dirs(&root)?;

    // Create headless dirs
    std::fs::create_dir_all(root.join("headless"))?;
    std::fs::create_dir_all(root.join("headless-overlay"))?;

    // Migrate headless if configured
    migrate_headless(&root)?;

    // Update config file
    update_config(&root)?;

    // Remove marker
    let _ = std::fs::remove_file(&marker);

    println!("\nMigration complete.");
    println!("SPT server files are now at: {}", spt_dest.display());
    println!("\nNote: If you have a running SPT server container, recreate it with updated volume mounts.");

    Ok(())
}

fn resolve_legacy_root(cli: &crate::cli::Cli) -> Result<PathBuf> {
    if let Some(p) = cli.effective_quma_dir() {
        return Ok(p.to_path_buf());
    }
    if let Ok(val) = std::env::var("QUMA_DIR") {
        return Ok(PathBuf::from(val));
    }
    if let Ok(val) = std::env::var("QUMA_SPT_DIR") {
        return Ok(PathBuf::from(val));
    }
    let cwd = std::env::current_dir()?;
    if validate_spt_dir(&cwd).is_ok() {
        return Ok(cwd);
    }
    bail!("Could not find legacy SPT directory. Pass --quma-dir or set QUMA_DIR.")
}

fn plan_moves(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut moves = vec![
        (root.join("SPT"), root.join("spt-server/SPT")),
        (root.join("BepInEx"), root.join("spt-server/BepInEx")),
    ];

    // Flatten quartermaster/ subdirs
    let qm = root.join("quartermaster");
    if qm.exists() {
        for name in [".staging", "config-history", "disabled"] {
            let src = qm.join(name);
            if src.exists() {
                moves.push((src, root.join(name)));
            }
        }
        // backups special case
        let backups = qm.join("backups");
        if backups.exists() {
            moves.push((backups, root.join("backups")));
        }
    }

    // Flatten .quartermaster/queued
    let dotqm = root.join(".quartermaster/queued");
    if dotqm.exists() {
        moves.push((dotqm, root.join("queued")));
    }

    // Rename quartermaster-cache → cache
    let old_cache = root.join("quartermaster-cache");
    if old_cache.exists() {
        moves.push((old_cache, root.join("cache")));
    }

    Ok(moves)
}

fn move_dir(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::rename(src, dst)
        .with_context(|| format!("failed to move {} → {} (cross-filesystem moves not supported — both must be on the same filesystem)", src.display(), dst.display()))
}

fn flatten_quma_dirs(root: &Path) -> Result<()> {
    let qm = root.join("quartermaster");
    if qm.exists() {
        for name in [".staging", "config-history", "disabled", "backups"] {
            let src = qm.join(name);
            if src.exists() {
                move_dir(&src, &root.join(name))?;
            }
        }
        // Remove empty quartermaster/ dir
        let _ = std::fs::remove_dir(&qm);
    }

    let dotqm_queued = root.join(".quartermaster/queued");
    if dotqm_queued.exists() {
        move_dir(&dotqm_queued, &root.join("queued"))?;
        let _ = std::fs::remove_dir(root.join(".quartermaster"));
    }

    let old_cache = root.join("quartermaster-cache");
    if old_cache.exists() {
        move_dir(&old_cache, &root.join("cache"))?;
    }

    Ok(())
}

#[allow(deprecated)]
fn migrate_headless(root: &Path) -> Result<()> {
    let config_path = root.join("quartermaster.toml");
    if !config_path.exists() {
        return Ok(());
    }
    let config = Config::load(&config_path)?;
    let headless = match &config.headless {
        Some(h) if !h.install_dir.as_os_str().is_empty() => h,
        _ => return Ok(()),
    };

    let old_install_dir = &headless.install_dir;
    if !old_install_dir.exists() {
        tracing::warn!(
            "Headless install_dir {} does not exist, skipping headless migration",
            old_install_dir.display()
        );
        return Ok(());
    }

    let headless_dest = root.join("headless");
    let overlay_dest = root.join("headless-overlay");

    // Move .quma/clients/ to headless-overlay/
    let old_clients = old_install_dir.join(".quma/clients");
    if old_clients.exists() {
        for entry in std::fs::read_dir(&old_clients)?.flatten() {
            let name = entry.file_name();
            let index: u32 = match name.to_string_lossy().parse() {
                Ok(i) => i,
                Err(_) => {
                    tracing::warn!(
                        "skipping non-numeric overlay dir: {}",
                        name.to_string_lossy()
                    );
                    continue;
                }
            };
            let dest = overlay_dest.join(format!("client-{index}"));
            move_dir(&entry.path(), &dest)?;
        }
        let _ = std::fs::remove_dir_all(old_install_dir.join(".quma"));
    }

    // Move headless install dir contents
    for entry in std::fs::read_dir(old_install_dir)?.flatten() {
        let name = entry.file_name();
        move_dir(&entry.path(), &headless_dest.join(&name))?;
    }

    println!(
        "Migrated headless client from {}",
        old_install_dir.display()
    );

    Ok(())
}

fn update_config(root: &Path) -> Result<()> {
    let config_path = root.join("quartermaster.toml");
    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)?;

    // Replace only `spt_dir` as a TOML key (at start of line, before =)
    let key_regex = Regex::new(r"(?m)^(\s*)spt_dir(\s*=)").expect("hardcoded regex should compile");
    let updated = key_regex.replace_all(&content, "${1}quma_dir${2}");

    std::fs::write(&config_path, updated.as_ref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_moves_includes_spt_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("SPT/user/mods")).expect("mkdir");
        std::fs::create_dir_all(root.join("BepInEx/plugins")).expect("mkdir");

        let moves = plan_moves(root).expect("plan_moves");
        assert!(moves.iter().any(|(s, _)| s == &root.join("SPT")));
        assert!(moves.iter().any(|(s, _)| s == &root.join("BepInEx")));
    }

    #[test]
    fn plan_moves_flattens_quartermaster_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("SPT")).expect("mkdir");
        std::fs::create_dir_all(root.join("BepInEx")).expect("mkdir");
        std::fs::create_dir_all(root.join("quartermaster/disabled")).expect("mkdir");
        std::fs::create_dir_all(root.join("quartermaster/config-history")).expect("mkdir");
        std::fs::create_dir_all(root.join(".quartermaster/queued")).expect("mkdir");
        std::fs::create_dir_all(root.join("quartermaster-cache")).expect("mkdir");

        let moves = plan_moves(root).expect("plan_moves");
        assert!(
            moves
                .iter()
                .any(|(s, d)| s == &root.join("quartermaster/disabled")
                    && d == &root.join("disabled"))
        );
        assert!(moves
            .iter()
            .any(|(s, d)| s == &root.join("quartermaster/config-history")
                && d == &root.join("config-history")));
        assert!(moves
            .iter()
            .any(|(s, d)| s == &root.join(".quartermaster/queued") && d == &root.join("queued")));
        assert!(moves
            .iter()
            .any(|(s, d)| s == &root.join("quartermaster-cache") && d == &root.join("cache")));
    }
}
