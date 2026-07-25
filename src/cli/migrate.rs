use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;

use crate::config::Config;
use crate::db::Database;
use crate::dirs::QumaDirs;
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

    // Move all non-quma entries into spt-server/
    for entry in std::fs::read_dir(&root)?.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !is_quma_owned(&name_str) {
            move_entry(&entry.path(), &spt_dest.join(&name))?;
        }
    }

    // Flatten quma internal dirs
    flatten_quma_dirs(&root)?;

    // Create headless dirs
    std::fs::create_dir_all(root.join("headless"))?;
    std::fs::create_dir_all(root.join("headless-overlay"))?;

    // Migrate headless if configured
    migrate_headless(&root)?;

    // Update config file
    update_config(&root)?;

    // Migrate to overlay layout if not in legacy mode
    let dirs = QumaDirs::from_root(root.clone());
    let db_path = dirs.db_path();
    if db_path.exists() {
        let db = Database::open(&db_path).context("failed to open database")?;
        migrate_to_overlay_layout(&db, &dirs).context("failed to migrate to overlay layout")?;
    }

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

// Files/dirs that stay at root (quma-owned). Everything else is SPT server
// runtime and moves into spt-server/.
const QUMA_OWNED: &[&str] = &[
    "quartermaster.db",
    "quartermaster.db-shm",
    "quartermaster.db-wal",
    "quartermaster.toml",
    "quma-cert.pem",
    "quma-key.pem",
    "quartermaster",
    ".quartermaster",
    "quartermaster-cache",
    "backups",
    "logs",
    "spt-server",
    ".migration-in-progress",
    // dev/editor artifacts
    ".claude",
    ".mcp.json",
    "CLAUDE.md",
    "docs",
];

fn is_quma_owned(name: &str) -> bool {
    QUMA_OWNED.contains(&name) || name.starts_with("quartermaster.db.bak") || name.starts_with('.')
}

fn plan_moves(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut moves = Vec::new();

    // Move all non-quma entries into spt-server/
    for entry in std::fs::read_dir(root)?.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !is_quma_owned(&name_str) {
            moves.push((entry.path(), root.join("spt-server").join(&name)));
        }
    }

    // Flatten quartermaster/ subdirs
    let qm = root.join("quartermaster");
    if qm.exists() {
        for name in [".staging", "config-history", "disabled"] {
            let src = qm.join(name);
            if src.exists() {
                moves.push((src, root.join(name)));
            }
        }
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

fn move_entry(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(17) && src.is_dir() && dst.is_dir() => {
            // EEXIST — destination dir already exists, merge contents
            for child in std::fs::read_dir(src)?.flatten() {
                let child_dst = dst.join(child.file_name());
                move_entry(&child.path(), &child_dst)?;
            }
            let _ = std::fs::remove_dir(src);
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!(
            "failed to move {} → {} (cross-filesystem moves not supported — both must be on the same filesystem)",
            src.display(), dst.display()
        )),
    }
}

fn flatten_quma_dirs(root: &Path) -> Result<()> {
    let qm = root.join("quartermaster");
    if qm.exists() {
        for name in [".staging", "config-history", "disabled", "backups"] {
            let src = qm.join(name);
            if src.exists() {
                move_entry(&src, &root.join(name))?;
            }
        }
        // Remove empty quartermaster/ dir
        let _ = std::fs::remove_dir(&qm);
    }

    let dotqm_queued = root.join(".quartermaster/queued");
    if dotqm_queued.exists() {
        move_entry(&dotqm_queued, &root.join("queued"))?;
        let _ = std::fs::remove_dir(root.join(".quartermaster"));
    }

    let old_cache = root.join("quartermaster-cache");
    if old_cache.exists() {
        move_entry(&old_cache, &root.join("cache"))?;
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
            move_entry(&entry.path(), &dest)?;
        }
        let _ = std::fs::remove_dir_all(old_install_dir.join(".quma"));
    }

    // Move headless install dir contents
    for entry in std::fs::read_dir(old_install_dir)?.flatten() {
        let name = entry.file_name();
        move_entry(&entry.path(), &headless_dest.join(&name))?;
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

pub fn migrate_to_overlay_layout(db: &Database, dirs: &QumaDirs) -> Result<()> {
    if dirs.is_legacy() {
        bail!("Cannot migrate to overlay layout from legacy layout. Run the standard migration first.");
    }

    let mod_overlay = dirs.mod_overlay();
    if mod_overlay.exists() && std::fs::read_dir(&mod_overlay)?.next().is_some() {
        tracing::info!("overlays/mod/ already populated, skipping overlay migration");
        return Ok(());
    }

    tracing::info!("Migrating mod files from spt-server/ to overlays/mod/");

    // Move all DB-tracked mod files
    let mods = db.list_mods()?;
    for m in &mods {
        let files = db.get_files_for_mod(m.id)?;
        for f in &files {
            let src = dirs.spt_server.join(&f.file_path);
            let dst = mod_overlay.join(&f.file_path);
            if src.exists() {
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                move_file_or_fallback(&src, &dst)?;
            }
        }

        // Also move addon files
        let addons = db.list_addons_for_mod(m.id)?;
        for addon in &addons {
            let addon_files = db.get_files_for_addon(addon.id)?;
            for f in &addon_files {
                let src = dirs.spt_server.join(&f.file_path);
                let dst = mod_overlay.join(&f.file_path);
                if src.exists() {
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    move_file_or_fallback(&src, &dst)?;
                }
            }
        }
    }

    // Move any remaining mod directories not covered by DB tracking.
    // The DB may have stale file lists (e.g., mod was updated outside quma)
    // or mods installed manually without quma tracking.
    let server_mods_dir = dirs.spt_server.join("SPT/user/mods");
    if server_mods_dir.is_dir() {
        let mod_overlay_mods = mod_overlay.join("SPT/user/mods");
        std::fs::create_dir_all(&mod_overlay_mods)?;
        for entry in std::fs::read_dir(&server_mods_dir)?.flatten() {
            if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            let dst = mod_overlay_mods.join(&name);
            if !dst.exists() {
                // Directory move — use rename or copy+remove
                match std::fs::rename(entry.path(), &dst) {
                    Ok(()) => {}
                    Err(_) => {
                        copy_dir_all(&entry.path(), &dst)?;
                        std::fs::remove_dir_all(entry.path())?;
                    }
                }
            }
        }
    }

    // Same for BepInEx/plugins/ — move client-side mod files
    let server_plugins_dir = dirs.spt_server.join("BepInEx/plugins");
    if server_plugins_dir.is_dir() {
        let mod_overlay_plugins = mod_overlay.join("BepInEx/plugins");
        std::fs::create_dir_all(&mod_overlay_plugins)?;
        for entry in std::fs::read_dir(&server_plugins_dir)?.flatten() {
            let name = entry.file_name();
            let dst = mod_overlay_plugins.join(&name);
            if !dst.exists() {
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    match std::fs::rename(entry.path(), &dst) {
                        Ok(()) => {}
                        Err(_) => {
                            copy_dir_all(&entry.path(), &dst)?;
                            std::fs::remove_dir_all(entry.path())?;
                        }
                    }
                } else {
                    move_file_or_fallback(&entry.path(), &dst)?;
                }
            }
        }
    }

    // Move runtime state to runtime overlay
    let runtime_upper = dirs.runtime_upper();
    let runtime_dirs = [
        "SPT/user/profiles",
        "SPT/user/cache",
        "BepInEx/config",
        "BepInEx/cache",
    ];
    for dir in &runtime_dirs {
        let src = dirs.spt_server.join(dir);
        let dst = runtime_upper.join(dir);
        if src.exists() && src.is_dir() {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match std::fs::rename(&src, &dst) {
                Ok(()) => {}
                Err(_) => {
                    // Cross-device: copy then remove
                    copy_dir_all(&src, &dst)?;
                    std::fs::remove_dir_all(&src)?;
                }
            }
        }
    }

    // Create runtimes/ merge point directories
    std::fs::create_dir_all(dirs.spt_runtime())?;

    tracing::info!("Overlay migration complete");
    Ok(())
}

fn move_file_or_fallback(src: &Path, dst: &Path) -> Result<()> {
    std::fs::rename(src, dst).or_else(|_| {
        std::fs::copy(src, dst)?;
        std::fs::remove_file(src)?;
        Ok(())
    })
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_moves_all_non_quma_files_to_spt_server() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("SPT/user/mods")).expect("mkdir");
        std::fs::create_dir_all(root.join("BepInEx/plugins")).expect("mkdir");
        std::fs::create_dir_all(root.join("EscapeFromTarkov_Data")).expect("mkdir");
        std::fs::write(root.join("Greed.exe"), "").expect("write");
        std::fs::write(root.join("doorstop_config.ini"), "").expect("write");
        std::fs::write(root.join("winhttp.dll"), "").expect("write");
        // quma-owned files that should NOT move
        std::fs::write(root.join("quartermaster.db"), "").expect("write");
        std::fs::write(root.join("quartermaster.toml"), "").expect("write");

        let moves = plan_moves(root).expect("plan_moves");

        // SPT runtime files move into spt-server/
        for name in [
            "SPT",
            "BepInEx",
            "EscapeFromTarkov_Data",
            "Greed.exe",
            "doorstop_config.ini",
            "winhttp.dll",
        ] {
            assert!(
                moves.iter().any(|(s, d)| s == &root.join(name) && d == &root.join("spt-server").join(name)),
                "{name} should be moved to spt-server/"
            );
        }

        // quma-owned files stay put
        for name in ["quartermaster.db", "quartermaster.toml"] {
            assert!(
                !moves.iter().any(|(s, _)| s == &root.join(name)),
                "{name} should NOT be moved"
            );
        }
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

    #[test]
    fn dotfiles_are_quma_owned() {
        assert!(is_quma_owned(".claude"));
        assert!(is_quma_owned(".mcp.json"));
        assert!(is_quma_owned(".quartermaster"));
        assert!(is_quma_owned("quartermaster.db.bak-20260628-135941"));
    }

    #[test]
    fn migrate_to_overlay_moves_mod_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Set up new layout structure
        let dirs = QumaDirs::from_root(root.to_path_buf());
        std::fs::create_dir_all(&dirs.spt_server).expect("mkdir spt_server");
        std::fs::create_dir_all(&dirs.overlays).expect("mkdir overlays");

        // Create DB with a mod that has files
        let db = Database::open(&dirs.db_path()).expect("open db");
        let mod_id = db
            .insert_mod(
                Some(123),
                Some(456),
                "TestMod",
                Some("test-mod"),
                "1.0.0",
                "forge",
                None,
            )
            .expect("insert mod");
        db.insert_file(
            mod_id,
            "SPT/user/mods/test-mod/package.json",
            Some("abc123"),
            Some(100),
        )
        .expect("insert file");
        db.insert_file(
            mod_id,
            "BepInEx/plugins/test-mod.dll",
            Some("def456"),
            Some(200),
        )
        .expect("insert file");

        // Create the actual files in spt_server/
        std::fs::create_dir_all(dirs.spt_server.join("SPT/user/mods/test-mod")).expect("mkdir");
        std::fs::create_dir_all(dirs.spt_server.join("BepInEx/plugins")).expect("mkdir");
        std::fs::write(
            dirs.spt_server.join("SPT/user/mods/test-mod/package.json"),
            "{}",
        )
        .expect("write package.json");
        std::fs::write(dirs.spt_server.join("BepInEx/plugins/test-mod.dll"), "dll")
            .expect("write dll");

        // Create runtime state
        std::fs::create_dir_all(dirs.spt_server.join("SPT/user/profiles")).expect("mkdir");
        std::fs::create_dir_all(dirs.spt_server.join("BepInEx/config")).expect("mkdir");
        std::fs::write(
            dirs.spt_server.join("SPT/user/profiles/profile1.json"),
            "{}",
        )
        .expect("write profile");
        std::fs::write(dirs.spt_server.join("BepInEx/config/test.cfg"), "config")
            .expect("write config");

        // Run migration
        super::migrate_to_overlay_layout(&db, &dirs).expect("migrate");

        // Assert mod files moved to mod_overlay()
        assert!(
            dirs.mod_overlay()
                .join("SPT/user/mods/test-mod/package.json")
                .exists(),
            "package.json should be in mod_overlay"
        );
        assert!(
            dirs.mod_overlay()
                .join("BepInEx/plugins/test-mod.dll")
                .exists(),
            "test-mod.dll should be in mod_overlay"
        );

        // Assert runtime state moved to runtime_upper()
        assert!(
            dirs.runtime_upper()
                .join("SPT/user/profiles/profile1.json")
                .exists(),
            "profile should be in runtime_upper"
        );
        assert!(
            dirs.runtime_upper()
                .join("BepInEx/config/test.cfg")
                .exists(),
            "config should be in runtime_upper"
        );

        // Assert spt_runtime() directory was created
        assert!(
            dirs.spt_runtime().exists(),
            "spt_runtime directory should exist"
        );

        // Assert original files are gone
        assert!(
            !dirs
                .spt_server
                .join("SPT/user/mods/test-mod/package.json")
                .exists(),
            "package.json should be removed from spt_server"
        );
        assert!(
            !dirs
                .spt_server
                .join("BepInEx/plugins/test-mod.dll")
                .exists(),
            "test-mod.dll should be removed from spt_server"
        );
    }

    #[test]
    fn migrate_to_overlay_moves_untracked_mod_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let dirs = QumaDirs::from_root(root.to_path_buf());
        std::fs::create_dir_all(&dirs.spt_server).expect("mkdir spt_server");
        std::fs::create_dir_all(&dirs.overlays).expect("mkdir overlays");

        // Create DB with NO mods tracked
        let db = Database::open(&dirs.db_path()).expect("open db");

        // Create untracked mod directories in spt-server/
        let fika_dir = dirs
            .spt_server
            .join("SPT/user/mods/fika-server/assets/configs");
        std::fs::create_dir_all(&fika_dir).expect("mkdir fika");
        std::fs::write(fika_dir.join("fika.jsonc"), "{}").expect("write fika.jsonc");

        let other_mod = dirs.spt_server.join("SPT/user/mods/some-other-mod");
        std::fs::create_dir_all(&other_mod).expect("mkdir other");
        std::fs::write(other_mod.join("package.json"), "{}").expect("write package.json");

        // Create untracked BepInEx/plugins entries (file and directory)
        let bepinex_plugins = dirs.spt_server.join("BepInEx/plugins");
        std::fs::create_dir_all(&bepinex_plugins).expect("mkdir BepInEx/plugins");
        std::fs::write(bepinex_plugins.join("untracked.dll"), "dll content")
            .expect("write untracked.dll");

        let plugin_subdir = bepinex_plugins.join("UnTrackedPlugin");
        std::fs::create_dir_all(&plugin_subdir).expect("mkdir UnTrackedPlugin");
        std::fs::write(plugin_subdir.join("plugin.dll"), "plugin dll").expect("write plugin.dll");

        // Run migration
        migrate_to_overlay_layout(&db, &dirs).expect("migrate");

        // Untracked mods should be moved to mod overlay
        assert!(
            dirs.mod_overlay()
                .join("SPT/user/mods/fika-server/assets/configs/fika.jsonc")
                .exists(),
            "fika-server should be in mod_overlay"
        );
        assert!(
            dirs.mod_overlay()
                .join("SPT/user/mods/some-other-mod/package.json")
                .exists(),
            "some-other-mod should be in mod_overlay"
        );

        // BepInEx entries should be moved to mod overlay
        assert!(
            dirs.mod_overlay()
                .join("BepInEx/plugins/untracked.dll")
                .exists(),
            "untracked.dll should be in mod_overlay"
        );
        assert!(
            dirs.mod_overlay()
                .join("BepInEx/plugins/UnTrackedPlugin/plugin.dll")
                .exists(),
            "UnTrackedPlugin/plugin.dll should be in mod_overlay"
        );

        // Original locations should be empty
        assert!(
            !dirs.spt_server.join("SPT/user/mods/fika-server").exists(),
            "fika-server should be removed from spt_server"
        );
        assert!(
            !dirs
                .spt_server
                .join("SPT/user/mods/some-other-mod")
                .exists(),
            "some-other-mod should be removed from spt_server"
        );
        assert!(
            !dirs
                .spt_server
                .join("BepInEx/plugins/untracked.dll")
                .exists(),
            "untracked.dll should be removed from spt_server"
        );
        assert!(
            !dirs
                .spt_server
                .join("BepInEx/plugins/UnTrackedPlugin")
                .exists(),
            "UnTrackedPlugin should be removed from spt_server"
        );
    }
}
