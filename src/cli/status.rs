use anyhow::Result;

use crate::health;

use super::common::CliContext;

pub async fn run(json: bool, ctx: &CliContext) -> Result<()> {
    let report = health::run_checks(ctx).await?;
    let exit_code = report.exit_code();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        print_report(&report, &ctx.spt_info);
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

fn print_report(report: &health::HealthReport, spt_info: &crate::spt::detect::SptInfo) {
    println!("SPT Server");
    if report.server.reachable {
        println!(
            "  Status:     running (responded in {}ms)",
            report.server.latency_ms.unwrap_or(0)
        );
        if let Some(ref ver) = report.server.version {
            let match_status = match report.server.version_matches {
                Some(true) => " (matches core.json)",
                Some(false) => " (MISMATCH with core.json!)",
                None => "",
            };
            println!("  Version:    {}{}", ver, match_status);
        }
        println!("  EFT Build:  {}", spt_info.tarkov_version);
        println!("  Address:    {}", report.server.address);
    } else {
        let reason = report.server.error.as_deref().unwrap_or("unreachable");
        println!("  Status:     DOWN ({})", reason);
        println!("  Address:    {}", report.server.address);
    }

    println!();
    match report.mods.loaded_count {
        Some(loaded) => println!(
            "Mods ({} installed, {} loaded)",
            report.mods.installed_count, loaded
        ),
        None => println!("Mods ({} installed)", report.mods.installed_count),
    }

    if !report.mods.load_failures.is_empty() {
        for name in &report.mods.load_failures {
            println!("  FAILED TO LOAD: {}", name);
        }
    }

    if !report.mods.untracked_loaded.is_empty() {
        for name in &report.mods.untracked_loaded {
            println!("  UNTRACKED (loaded but not managed): {}", name);
        }
    }

    if !report.mods.incompatible_mods.is_empty() {
        for name in &report.mods.incompatible_mods {
            println!(
                "  WARNING: {} is incompatible with SPT {}",
                name, spt_info.spt_version
            );
        }
    }

    if report.mods.updates_available > 0 {
        println!(
            "  {} update(s) available (run `quma check` for details)",
            report.mods.updates_available
        );
    }

    if report.mods.incompatible_mods.is_empty() && report.mods.updates_available == 0 {
        println!("  All mods compatible and up to date.");
    }

    println!();
    println!(
        "Integrity ({} tracked files)",
        report.integrity.tracked_files
    );

    if report.integrity.missing_files.is_empty()
        && report.integrity.modified_files.is_empty()
        && report.integrity.untracked_dirs.is_empty()
    {
        println!("  All mod files present on disk, hashes match.");
    } else {
        if !report.integrity.missing_files.is_empty() {
            println!(
                "  {} file(s) MISSING from disk:",
                report.integrity.missing_files.len()
            );
            for f in &report.integrity.missing_files {
                println!("    - {f}");
            }
        }
        if !report.integrity.modified_files.is_empty() {
            println!(
                "  {} file(s) MODIFIED (hash mismatch):",
                report.integrity.modified_files.len()
            );
            for f in &report.integrity.modified_files {
                println!("    - {f}");
            }
        }
        for dir in &report.integrity.untracked_dirs {
            println!("  {} untracked file(s) in {}", dir.file_count, dir.path);
        }
    }
}
