use std::io::{self, Write};

use anyhow::Result;

use crate::db::mods::InstalledMod;

use super::common::{resolve_installed_mod, CliContext};

pub async fn run(mod_ref: &str, force: bool, ctx: &CliContext) -> Result<()> {
    let installed = resolve_installed_mod(mod_ref, ctx)?;

    // Check if we should queue instead of applying
    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir).await? {
        ctx.db.insert_pending_op(
            "remove",
            installed.forge_mod_id,
            None,
            &installed.name,
            None,
            None,
        )?;
        println!(
            "Server is running — removal of {} queued. Run `quma apply` when the server is stopped.",
            installed.name
        );
        return Ok(());
    }

    if force {
        let running = crate::server_detect::is_server_running(&ctx.config, &ctx.spt_dir).await?;
        if running {
            println!(
                "Warning: applying changes while the server is running may cause instability."
            );
        }
    }

    let all_dependents = collect_all_reverse_deps(installed.id, ctx)?;
    if !all_dependents.is_empty() {
        println!(
            "Warning: the following installed mods depend on {}:",
            installed.name
        );
        for dep in &all_dependents {
            println!("  - {} (v{})", dep.name, dep.version);
        }

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
    } else {
        remove_single_mod(&installed, ctx)?;
    }

    println!("{} removed successfully.", installed.name);
    Ok(())
}

/// Recursively collect all transitive reverse dependencies of a mod.
/// Returns them in BFS order (direct dependents first, then their dependents, etc.).
pub fn collect_all_reverse_deps(mod_db_id: i64, ctx: &CliContext) -> Result<Vec<InstalledMod>> {
    crate::ops::collect_all_reverse_deps(&ctx.db, mod_db_id)
}

pub fn remove_single_mod(installed: &InstalledMod, ctx: &CliContext) -> Result<()> {
    let file_count = ctx.db.get_files_for_mod(installed.id)?.len();

    crate::ops::remove_mod_by_id(&ctx.db, &ctx.spt_dir, installed.id)?;

    if file_count > 0 {
        println!("  Deleted {} files for {}", file_count, installed.name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::common::resolve_installed_mod;
    use crate::config::Config;
    use crate::db::Database;
    use crate::forge::client::ForgeClient;
    use crate::spt::detect::SptInfo;
    use tempfile::TempDir;

    fn make_test_ctx(tmp: &TempDir) -> CliContext {
        let spt_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            db: Database::open_in_memory().unwrap(),
            forge: ForgeClient::new(None).unwrap(),
        }
    }

    #[test]
    fn resolve_installed_by_forge_id() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();

        let m = resolve_installed_mod("100", &ctx).unwrap();
        assert_eq!(m.name, "TestMod");
    }

    #[test]
    fn resolve_installed_by_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();

        let m = resolve_installed_mod("TestMod", &ctx).unwrap();
        assert_eq!(m.forge_mod_id, 100);
    }

    #[test]
    fn resolve_installed_by_slug_distinct_from_name() {
        let tmp = TempDir::new().unwrap();
        let ctx = make_test_ctx(&tmp);
        ctx.db
            .insert_mod(100, 200, "S.A.I.N.", Some("sain"), "1.0.0")
            .unwrap();

        let m = resolve_installed_mod("sain", &ctx).unwrap();
        assert_eq!(m.forge_mod_id, 100);
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
        let mod_dir = ctx.spt_dir.join("SPT/user/mods/TestMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();

        // Insert into DB
        let db_id = ctx
            .db
            .insert_mod(100, 200, "TestMod", None, "1.0.0")
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

        let mod_c = ctx.db.insert_mod(100, 200, "ModC", None, "1.0.0").unwrap();
        let mod_b = ctx.db.insert_mod(101, 201, "ModB", None, "1.0.0").unwrap();
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

        let mod_c = ctx.db.insert_mod(100, 200, "ModC", None, "1.0.0").unwrap();
        let mod_b = ctx.db.insert_mod(101, 201, "ModB", None, "1.0.0").unwrap();
        let mod_a = ctx.db.insert_mod(102, 202, "ModA", None, "1.0.0").unwrap();
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
