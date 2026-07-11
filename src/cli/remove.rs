use std::io::{self, Write};

use anyhow::Result;

use crate::db::mods::InstalledMod;

use super::common::{confirm, resolve_installed_mod, CliContext};

pub async fn run(
    mod_ref: &str,
    force: bool,
    yes: bool,
    addon: bool,
    ctx: &CliContext,
) -> Result<()> {
    // If --addon flag is set, use addon remove flow
    if addon {
        return run_addon_remove(mod_ref, force, yes, ctx).await;
    }

    let installed = resolve_installed_mod(mod_ref, ctx)?;

    // Check if we should queue instead of applying
    if crate::queue::should_queue(&ctx.config, force, &ctx.dirs, ctx.container_mgr.as_ref()).await?
    {
        // URL/file mods can't be queued (no forge_mod_id), must use --force
        if installed.forge_mod_id.is_none() {
            anyhow::bail!(
                "Cannot queue removal of {} (installed from URL/file). Use --force to remove immediately.",
                installed.name
            );
        }

        if !yes {
            let file_count = ctx.db.get_files_for_mod(installed.id)?.len();
            if !confirm(&format_remove_prompt(&installed.name, file_count))? {
                println!("Removal cancelled.");
                return Ok(());
            }
        }

        ctx.db.insert_pending_op(
            crate::db::users::QueueAction::Remove,
            installed.forge_mod_id.expect("checked above"), // ponytail: safe, None bails earlier
            None,
            &installed.name,
            None,
            None,
        )?;
        println!(
            "Server is running — removal of {} queued. It will be applied on next server restart.",
            installed.name
        );
        return Ok(());
    }

    super::common::warn_if_forcing_while_running(force, ctx).await?;

    let all_dependents = collect_all_reverse_deps(installed.id, ctx)?;
    if !all_dependents.is_empty() {
        println!(
            "Warning: the following installed mods depend on {}:",
            installed.name
        );
        for dep in &all_dependents {
            println!("  - {} (v{})", dep.name, dep.version);
        }

        if yes {
            // --yes skips the interactive menu and removes the target only
            println!(
                "Removing {} only (--yes flag, skipping dependents).",
                installed.name
            );
            remove_single_mod(&installed, ctx)?;
        } else {
            println!("\nOptions:");
            println!(
                "  [1] Remove {} only (may break dependents)",
                installed.name
            );
            println!(
                "  [2] Remove {} and all {} dependents",
                installed.name,
                all_dependents.len()
            );
            println!("  [3] Cancel");

            print!("Select [1-3]: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim() {
                "1" => {
                    remove_single_mod(&installed, ctx)?;
                }
                "2" => {
                    // Remove in reverse order (leaves before roots)
                    for dep in all_dependents.iter().rev() {
                        remove_single_mod(dep, ctx)?;
                    }
                    remove_single_mod(&installed, ctx)?;
                }
                _ => {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        }
    } else {
        if !yes {
            let file_count = ctx.db.get_files_for_mod(installed.id)?.len();
            if !confirm(&format_remove_prompt(&installed.name, file_count))? {
                println!("Removal cancelled.");
                return Ok(());
            }
        }
        remove_single_mod(&installed, ctx)?;
    }

    println!("{} removed successfully.", installed.name);
    Ok(())
}

fn format_remove_prompt(mod_name: &str, file_count: usize) -> String {
    match file_count {
        0 => format!("Remove {}?", mod_name),
        1 => format!("Remove {} (1 file)?", mod_name),
        n => format!("Remove {} ({} files)?", mod_name, n),
    }
}

/// Recursively collect all transitive reverse dependencies of a mod.
/// Returns them in BFS order (direct dependents first, then their dependents, etc.).
pub fn collect_all_reverse_deps(mod_db_id: i64, ctx: &CliContext) -> Result<Vec<InstalledMod>> {
    crate::ops::collect_all_reverse_deps(&ctx.db, mod_db_id)
}

pub fn remove_single_mod(installed: &InstalledMod, ctx: &CliContext) -> Result<()> {
    let file_count = ctx.db.get_files_for_mod(installed.id)?.len();

    crate::ops::remove_mod_by_id(&ctx.db, &ctx.dirs, &ctx.config, installed.id)?;

    if file_count > 0 {
        println!("  Deleted {} files for {}", file_count, installed.name);
    }

    Ok(())
}

async fn run_addon_remove(addon_ref: &str, force: bool, yes: bool, ctx: &CliContext) -> Result<()> {
    use super::common::resolve_installed_addon;

    let installed = resolve_installed_addon(addon_ref, ctx)?;

    // Check if we should queue instead of applying
    if crate::queue::should_queue(&ctx.config, force, &ctx.dirs, ctx.container_mgr.as_ref()).await?
    {
        if !yes {
            let file_count = ctx.db.get_files_for_addon(installed.id)?.len();
            if !confirm(&format_addon_remove_prompt(&installed.name, file_count))? {
                println!("Removal cancelled.");
                return Ok(());
            }
        }

        ctx.db.insert_pending_addon_op(
            crate::db::users::QueueAction::Remove,
            installed.forge_addon_id,
            None,
            &installed.name,
            None,
            None,
        )?;
        println!(
            "Server is running — removal of {} queued. It will be applied on next server restart.",
            installed.name
        );
        return Ok(());
    }

    super::common::warn_if_forcing_while_running(force, ctx).await?;

    if !yes {
        let file_count = ctx.db.get_files_for_addon(installed.id)?.len();
        if !confirm(&format_addon_remove_prompt(&installed.name, file_count))? {
            println!("Removal cancelled.");
            return Ok(());
        }
    }

    remove_single_addon(&installed, ctx)?;
    println!("{} removed successfully.", installed.name);
    Ok(())
}

fn format_addon_remove_prompt(addon_name: &str, file_count: usize) -> String {
    match file_count {
        0 => format!("Remove addon {}?", addon_name),
        1 => format!("Remove addon {} (1 file)?", addon_name),
        n => format!("Remove addon {} ({} files)?", addon_name, n),
    }
}

fn remove_single_addon(
    installed: &crate::db::addons::InstalledAddon,
    ctx: &CliContext,
) -> Result<()> {
    let file_count = ctx.db.get_files_for_addon(installed.id)?.len();

    crate::ops::remove_addon_by_id(&ctx.db, &ctx.dirs, &ctx.config, installed.id)?;

    if file_count > 0 {
        println!("  Deleted {} files for {}", file_count, installed.name);
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::cli::common::resolve_installed_mod;
    use crate::config::Config;
    use crate::db::Database;
    use crate::forge::client::ForgeClient;
    use crate::spt::detect::SptInfo;
    use tempfile::TempDir;

    fn make_test_ctx(tmp: &TempDir) -> CliContext {
        let root = tmp.path().to_path_buf();
        let dirs = crate::dirs::QumaDirs::from_root(root.clone());
        std::fs::create_dir_all(dirs.server_mods_dir()).unwrap();
        std::fs::create_dir_all(dirs.client_mods_dir()).unwrap();

        CliContext {
            dirs,
            spt_info: SptInfo {
                root: root.clone(),
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            db: Database::open_in_memory().unwrap(),
            forge: ForgeClient::new().unwrap(),
            container_mgr: None,
        }
    }

    #[test]
    fn resolve_installed_by_forge_id() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                Some("test-mod"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();

        let m = resolve_installed_mod("100", &ctx).unwrap();
        assert_eq!(m.name, "TestMod");
    }

    #[test]
    fn resolve_installed_by_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                Some("test-mod"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();

        let m = resolve_installed_mod("TestMod", &ctx).unwrap();
        assert_eq!(m.forge_mod_id, Some(100));
    }

    #[test]
    fn resolve_installed_by_slug_distinct_from_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db
            .insert_mod(
                Some(100),
                Some(200),
                "S.A.I.N.",
                Some("sain"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();

        let m = resolve_installed_mod("sain", &ctx).unwrap();
        assert_eq!(m.forge_mod_id, Some(100));
        assert_eq!(m.name, "S.A.I.N.");
    }

    #[test]
    fn resolve_installed_not_found() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        let result = resolve_installed_mod("nonexistent", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn remove_single_mod_deletes_files_and_db() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);

        // Create mod files on disk
        let mod_dir = ctx.dirs.spt_server.join("SPT/user/mods/TestMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();

        // Insert into DB
        let db_id = ctx
            .db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        ctx.db
            .insert_file(
                db_id,
                "SPT/user/mods/TestMod/package.json",
                Some("abc"),
                Some(2),
            )
            .unwrap();

        let installed = ctx.db.get_mod_by_forge_id(100).unwrap().unwrap();
        remove_single_mod(&installed, &ctx).unwrap();

        // File should be gone
        assert!(!mod_dir.join("package.json").exists());

        // DB record should be gone
        assert!(ctx.db.get_mod_by_forge_id(100).unwrap().is_none());
    }

    #[test]
    fn remove_mod_with_dependent_succeeds_via_cascade_fk() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);

        let mod_c = ctx
            .db
            .insert_mod(Some(100), Some(200), "ModC", None, "1.0.0", "forge", None)
            .unwrap();
        let mod_b = ctx
            .db
            .insert_mod(Some(101), Some(201), "ModB", None, "1.0.0", "forge", None)
            .unwrap();
        // B depends on C
        ctx.db.insert_dependency(mod_b, mod_c, None).unwrap();

        // Removing C directly should succeed (CASCADE on depends_on_mod_id cleans up the dep row)
        remove_single_mod(&ctx.db.get_mod(mod_c).unwrap().unwrap(), &ctx).unwrap();

        assert!(ctx.db.get_mod(mod_c).unwrap().is_none());
        // B still exists but its dependency row on C is gone
        assert!(ctx.db.get_mod(mod_b).unwrap().is_some());
        assert!(ctx.db.get_dependencies(mod_b).unwrap().is_empty());
    }

    #[test]
    fn collect_all_reverse_deps_transitive() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);

        let mod_c = ctx
            .db
            .insert_mod(Some(100), Some(200), "ModC", None, "1.0.0", "forge", None)
            .unwrap();
        let mod_b = ctx
            .db
            .insert_mod(Some(101), Some(201), "ModB", None, "1.0.0", "forge", None)
            .unwrap();
        let mod_a = ctx
            .db
            .insert_mod(Some(102), Some(202), "ModA", None, "1.0.0", "forge", None)
            .unwrap();
        // A depends on B, B depends on C
        ctx.db.insert_dependency(mod_a, mod_b, None).unwrap();
        ctx.db.insert_dependency(mod_b, mod_c, None).unwrap();

        let deps = collect_all_reverse_deps(mod_c, &ctx).unwrap();
        assert_eq!(deps.len(), 2);
        // BFS order: B first (direct), then A (transitive)
        assert_eq!(deps[0].name, "ModB");
        assert_eq!(deps[1].name, "ModA");
    }
}
