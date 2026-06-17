use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::db::Database;
use crate::spt::detect::{detect_spt_dir, read_spt_version};

use super::Cli;

pub fn run(path: Option<PathBuf>, cli: &Cli) -> Result<()> {
    // 1. Resolve SPT directory
    let spt_dir = match path {
        Some(ref p) => {
            crate::spt::detect::validate_spt_dir(p)?;
            p.clone()
        }
        None => detect_spt_dir(cli.spt_dir.as_deref(), None)?,
    };

    let spt_info = read_spt_version(&spt_dir)?;
    println!(
        "Detected SPT {} (EFT {}) at {}",
        spt_info.spt_version,
        spt_info.tarkov_version,
        spt_dir.display()
    );

    // 2. Create config file
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = if config_path.exists() {
        println!("Config already exists at {}", config_path.display());
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.spt_dir = Some(spt_dir.clone());
    config.ensure_session_secret();
    config.save(&config_path)?;
    println!("Config saved to {}", config_path.display());

    // 3. Create database
    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    // 4. Scan for existing mods
    let (unmanaged_dirs, unmanaged_count) = super::common::find_unmanaged_mod_dirs(&spt_dir, &db)?;

    if unmanaged_dirs.is_empty() {
        println!("No existing mod files found.");
    } else {
        println!(
            "\nFound {} unmanaged mod director{} ({} files):",
            unmanaged_dirs.len(),
            if unmanaged_dirs.len() == 1 {
                "y"
            } else {
                "ies"
            },
            unmanaged_count
        );
        for dir in unmanaged_dirs.keys() {
            println!("  {}", dir);
        }
        println!("\nUse `quma track <path> <forge_mod_id>` to associate them with Forge entries.");
    }

    // 5. Check for admin user
    if !db.admin_exists()? {
        println!("\nNo admin user exists. Create one with the web UI (`quma serve`) or during `quma setup`.");
    }

    println!("\nQuartermaster initialized successfully.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_fake_spt_dir(base: &std::path::Path) -> PathBuf {
        let spt_root = base.to_path_buf();

        // SPT/SPT.Server.exe
        std::fs::create_dir_all(spt_root.join("SPT")).unwrap();
        std::fs::write(spt_root.join("SPT/SPT.Server.exe"), b"").unwrap();

        // SPT/SPT_Data/configs/core.json
        let configs_dir = spt_root.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("core.json"),
            r#"{"compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();

        // SPT/SPT.Server.deps.json
        std::fs::write(
            spt_root.join("SPT/SPT.Server.deps.json"),
            r#"{"libraries":{"SPT.Server/4.0.13-RELEASE+abc123.20260101":{}}}"#,
        )
        .unwrap();

        // SPT/user/mods/
        std::fs::create_dir_all(spt_root.join("SPT/user/mods")).unwrap();

        // BepInEx/plugins/
        std::fs::create_dir_all(spt_root.join("BepInEx/plugins")).unwrap();

        spt_root
    }

    #[test]
    fn init_creates_config_and_db() {
        // Clear QUMA_CONFIG env var to avoid interference from other tests
        unsafe {
            std::env::remove_var("QUMA_CONFIG");
        }

        let tmp = TempDir::new().unwrap();
        let spt_dir = create_fake_spt_dir(tmp.path());

        let cli = Cli {
            spt_dir: None,
            config: None,
            command: crate::cli::Command::Init { path: None },
        };

        run(Some(spt_dir.clone()), &cli).unwrap();

        // Config file should exist
        let config_path = spt_dir.join("quartermaster.toml");
        assert!(config_path.exists(), "config file should be created");

        // DB file should exist
        let db_path = spt_dir.join("quartermaster.db");
        assert!(db_path.exists(), "database should be created");

        // Config should have spt_dir set and a session secret
        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.spt_dir, Some(spt_dir));
        assert!(!config.session_secret.is_empty());
    }

    #[test]
    fn init_detects_unmanaged_mods() {
        // Clear QUMA_CONFIG env var to avoid interference from other tests
        unsafe {
            std::env::remove_var("QUMA_CONFIG");
        }

        let tmp = TempDir::new().unwrap();
        let spt_dir = create_fake_spt_dir(tmp.path());

        // Create some existing mod files
        let mod_dir = spt_dir.join("SPT/user/mods/SomeMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();

        let cli = Cli {
            spt_dir: None,
            config: None,
            command: crate::cli::Command::Init { path: None },
        };

        // Should succeed and report unmanaged files (output goes to stdout)
        run(Some(spt_dir), &cli).unwrap();
    }

    #[test]
    fn init_idempotent_with_existing_config() {
        // Clear QUMA_CONFIG env var to avoid interference from other tests
        unsafe {
            std::env::remove_var("QUMA_CONFIG");
        }

        let tmp = TempDir::new().unwrap();
        let spt_dir = create_fake_spt_dir(tmp.path());

        let cli = Cli {
            spt_dir: None,
            config: None,
            command: crate::cli::Command::Init { path: None },
        };

        // Run init twice
        run(Some(spt_dir.clone()), &cli).unwrap();
        run(Some(spt_dir), &cli).unwrap();
    }
}
