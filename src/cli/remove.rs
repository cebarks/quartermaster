use std::io::{self, Write};

use anyhow::Result;

use crate::db::mods::InstalledMod;
use crate::spt::mods::delete_mod_files;

use super::common::{resolve_installed_mod, CliContext};

pub fn run(mod_ref: &str, _force: bool, ctx: &CliContext) -> Result<()> {
    // TODO(debt): _force is accepted but unused until Phase 3 wires server-running detection
    let installed = resolve_installed_mod(mod_ref, ctx)?;

    let rev_deps = ctx.db.get_reverse_dependencies(installed.id)?;
    if !rev_deps.is_empty() {
        println!(
            "Warning: the following installed mods depend on {}:",
            installed.name
        );
        for dep in &rev_deps {
            if let Some(dependent) = ctx.db.get_mod(dep.mod_id)? {
                println!("  - {} (v{})", dependent.name, dependent.version);
            }
        }

        println!("\nOptions:");
        println!(
            "  [1] Remove {} only (may break dependents)",
            installed.name
        );
        println!("  [2] Remove {} and all dependents", installed.name);
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
                for dep in &rev_deps {
                    if let Some(dependent) = ctx.db.get_mod(dep.mod_id)? {
                        remove_single_mod(&dependent, ctx)?;
                    }
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

fn remove_single_mod(installed: &InstalledMod, ctx: &CliContext) -> Result<()> {
    // Get tracked files
    let files = ctx.db.get_files_for_mod(installed.id)?;
    let file_paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();

    // Delete files from disk
    if !file_paths.is_empty() {
        delete_mod_files(&ctx.spt_dir, &file_paths)?;
        println!(
            "  Deleted {} files for {}",
            file_paths.len(),
            installed.name
        );
    }

    // Remove from database (cascades to files and dependencies)
    ctx.db.delete_mod(installed.id)?;

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
        std::fs::create_dir_all(spt_dir.join("user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        CliContext {
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir,
                spt_version: "4.0.13".to_string(),
                tarkov_version: "0.16.9-40087".to_string(),
            },
            config: Config::default(),
            config_path: tmp.path().join("quartermaster.toml"),
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
        let mod_dir = ctx.spt_dir.join("user/mods/TestMod");
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
                "user/mods/TestMod/package.json",
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
}
