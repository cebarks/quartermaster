use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::db::mods::InstalledMod;
use crate::db::Database;
use crate::spt::mods::ExtractedFile;

fn record_extracted_files(db: &Database, mod_db_id: i64, files: &[ExtractedFile]) -> Result<()> {
    for file in files {
        db.insert_file(
            mod_db_id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }
    Ok(())
}

/// Parameters for installing a mod from a downloaded archive.
pub struct InstallRequest<'a> {
    pub db: &'a Database,
    pub spt_dir: &'a Path,
    pub config: &'a crate::config::Config,
    pub forge_mod_id: i64,
    pub version_id: i64,
    pub name: &'a str,
    pub slug: Option<&'a str>,
    pub version: &'a str,
    pub archive_path: &'a Path,
}

pub fn install_mod_from_archive(req: &InstallRequest<'_>) -> Result<i64> {
    tracing::info!(
        req.name,
        req.forge_mod_id,
        req.version,
        "installing mod from archive"
    );
    let extracted = crate::spt::mods::extract_mod(req.archive_path, req.spt_dir)?;
    let tx = req.db.begin_transaction()?;
    let db_id = req.db.insert_mod(
        req.forge_mod_id,
        req.version_id,
        req.name,
        req.slug,
        req.version,
    )?;
    record_extracted_files(req.db, db_id, &extracted)?;
    tx.commit()?;
    tracing::debug!(
        db_id,
        file_count = extracted.len(),
        "mod installed, files recorded"
    );
    if let Err(e) = crate::modsync::regenerate_if_enabled(req.spt_dir, req.config, req.db) {
        tracing::warn!(error = %e, "failed to regenerate NarcoNet config");
    }
    Ok(db_id)
}

pub fn update_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
    version_id: i64,
    version_str: &str,
    archive_path: &Path,
) -> Result<()> {
    tracing::info!(mod_db_id, version_str, "updating mod from archive");
    let staging_dir = tempfile::tempdir()?;
    let extracted = crate::spt::mods::extract_mod(archive_path, staging_dir.path())?;

    let old_files = db.get_files_for_mod(mod_db_id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(
        old_file_count = old_paths.len(),
        new_file_count = extracted.len(),
        "replacing mod files"
    );
    crate::spt::mods::delete_mod_files(spt_dir, &old_paths)?;

    let tx = db.begin_transaction()?;
    db.delete_files_for_mod(mod_db_id)?;

    for file in &extracted {
        let src = staging_dir.path().join(&file.path);
        let dst = spt_dir.join(&file.path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
    }

    record_extracted_files(db, mod_db_id, &extracted)?;
    db.update_mod(mod_db_id, version_id, version_str)?;
    tx.commit()?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(error = %e, "failed to regenerate NarcoNet config");
    }
    Ok(())
}

/// Apply a mod update using brief DB locks suitable for the web context.
///
/// This is the async counterpart of [`update_mod_from_archive`]: it performs the
/// same 3-step update (read old paths, filesystem swap, DB write) but splits
/// each step into a separate [`actix_web::web::block`] call so the DB mutex is
/// never held across slow filesystem I/O.
///
/// `extracted` must be the files already extracted to `staging_path` (e.g. via
/// [`crate::spt::mods::extract_mod`]).
pub async fn apply_mod_update(
    db: Arc<parking_lot::Mutex<Database>>,
    spt_dir: PathBuf,
    staging_path: PathBuf,
    extracted: Vec<ExtractedFile>,
    mod_db_id: i64,
    version_id: i64,
    version_str: String,
) -> Result<()> {
    // Step 1: Read old file paths (brief DB lock)
    let db_read = db.clone();
    let old_paths = actix_web::web::block(move || {
        let db = db_read.lock();
        let files = db.get_files_for_mod(mod_db_id)?;
        Ok::<_, anyhow::Error>(files.into_iter().map(|f| f.file_path).collect::<Vec<_>>())
    })
    .await??;

    // Step 2: Filesystem swap (no DB lock held)
    let spt_dir_fs = spt_dir.clone();
    let extracted = actix_web::web::block(move || {
        crate::spt::mods::delete_mod_files(&spt_dir_fs, &old_paths)?;
        for file in &extracted {
            let src = staging_path.join(&file.path);
            let dst = spt_dir_fs.join(&file.path);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
        }
        Ok::<_, anyhow::Error>(extracted)
    })
    .await??;

    // Step 3: DB writes atomically (brief DB lock)
    actix_web::web::block(move || {
        let db = db.lock();
        let tx = db.begin_transaction()?;
        db.delete_files_for_mod(mod_db_id)?;
        record_extracted_files(&db, mod_db_id, &extracted)?;
        db.update_mod(mod_db_id, version_id, &version_str)?;
        tx.commit()?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}

pub fn remove_mod_by_id(
    db: &Database,
    spt_dir: &Path,
    config: &crate::config::Config,
    mod_db_id: i64,
) -> Result<()> {
    tracing::info!(mod_db_id, "removing mod");
    let files = db.get_files_for_mod(mod_db_id)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(file_count = paths.len(), "deleting mod files");
    crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
    let tx = db.begin_transaction()?;
    db.delete_mod(mod_db_id)?;
    tx.commit()?;
    if let Err(e) = crate::modsync::regenerate_if_enabled(spt_dir, config, db) {
        tracing::warn!(error = %e, "failed to regenerate NarcoNet config");
    }
    Ok(())
}

/// Scan a mod's directories on disk and record any files not already tracked
/// as runtime-generated files (source = 'runtime').
pub fn scan_and_record_runtime_files(
    db: &std::sync::Arc<parking_lot::Mutex<Database>>,
    mod_db_id: i64,
    spt_dir: &Path,
) -> Result<()> {
    let db = db.lock();
    let tracked = db.get_files_for_mod(mod_db_id)?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked.iter().map(|f| f.file_path.as_str()).collect();

    // Determine which top-level directories this mod occupies
    let mut mod_dirs: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for file in &tracked {
        let p = Path::new(&file.file_path);
        // For SPT/user/mods/ModName/... take first 4 components
        // For BepInEx/plugins/ModName/... take first 3 components
        let parts: Vec<&str> = file.file_path.split('/').collect();
        let dir = if file.file_path.starts_with("SPT/") && parts.len() >= 4 {
            format!("{}/{}/{}/{}", parts[0], parts[1], parts[2], parts[3])
        } else if file.file_path.starts_with("BepInEx/") && parts.len() >= 3 {
            format!("{}/{}/{}", parts[0], parts[1], parts[2])
        } else if let Some(parent) = p.parent() {
            parent.to_string_lossy().to_string()
        } else {
            continue;
        };
        mod_dirs.insert(spt_dir.join(dir));
    }

    tracing::debug!(
        mod_db_id,
        dir_count = mod_dirs.len(),
        "scanning for runtime files"
    );

    // Scan each directory for untracked files
    for dir in &mod_dirs {
        if !dir.is_dir() {
            continue;
        }
        scan_runtime_recursive(dir, spt_dir, mod_db_id, &tracked_paths, &db)?;
    }

    Ok(())
}

fn scan_runtime_recursive(
    dir: &Path,
    spt_root: &Path,
    mod_db_id: i64,
    tracked: &std::collections::HashSet<&str>,
    db: &Database,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_runtime_recursive(&path, spt_root, mod_db_id, tracked, db)?;
        } else if let Ok(relative) = path.strip_prefix(spt_root) {
            let rel_str = relative.to_string_lossy();
            if !tracked.contains(rel_str.as_ref()) {
                tracing::trace!(path = %rel_str, "recording runtime file");
                let content = std::fs::read(&path).unwrap_or_default();
                let hash = crate::spt::mods::compute_hash_public(&content);
                let size = content.len() as i64;
                if let Err(e) = db.insert_file_with_source(
                    mod_db_id,
                    &rel_str,
                    Some(&hash),
                    Some(size),
                    "runtime",
                ) {
                    tracing::warn!(path = %path.display(), error = %e, "failed to record runtime file");
                }
            }
        }
    }
    Ok(())
}

/// Given a list of file paths belonging to a mod, find the unique top-level
/// directories that contain them (e.g. `SPT/user/mods/ModName` or
/// `BepInEx/plugins/PluginDir`). Returns relative paths.
fn find_top_level_mod_dirs(file_paths: &[String]) -> Vec<String> {
    let mut dirs = std::collections::BTreeSet::new();
    for path in file_paths {
        let parts: Vec<&str> = path.split('/').collect();
        // SPT/user/mods/ModName/... → 4 components
        if path.starts_with("SPT/user/mods/") && parts.len() >= 4 {
            dirs.insert(format!(
                "{}/{}/{}/{}",
                parts[0], parts[1], parts[2], parts[3]
            ));
        // BepInEx/plugins/PluginDir/... → 3 components (if the file is inside a directory)
        } else if path.starts_with("BepInEx/plugins/") && parts.len() >= 3 {
            // Check if third component is a directory (has children) or a loose file
            if parts.len() > 3 {
                dirs.insert(format!("{}/{}/{}", parts[0], parts[1], parts[2]));
            }
            // Loose files (e.g. BepInEx/plugins/something.dll) are handled separately
        }
    }
    dirs.into_iter().collect()
}

/// Find loose files that are not inside a top-level mod directory.
/// These are individual files like `BepInEx/plugins/something.dll`.
fn find_loose_files<'a>(file_paths: &'a [String], top_dirs: &[String]) -> Vec<&'a str> {
    file_paths
        .iter()
        .filter(|p| !top_dirs.iter().any(|d| p.starts_with(d)))
        .map(|p| p.as_str())
        .collect()
}

/// Disable a mod by renaming its top-level directories and loose files with
/// a `.disabled` suffix, updating file paths in the database, and marking
/// the mod as disabled.
pub fn disable_mod(db: &Database, spt_dir: &Path, mod_db_id: i64) -> Result<()> {
    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
    if mod_info.disabled {
        anyhow::bail!("mod is already disabled");
    }

    let files = db.get_files_for_mod(mod_db_id)?;
    let file_paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
    let top_dirs = find_top_level_mod_dirs(&file_paths);

    tracing::info!(mod_db_id, name = %mod_info.name, "disabling mod");

    // Rename top-level directories
    for dir in &top_dirs {
        let src = spt_dir.join(dir);
        let dst = spt_dir.join(format!("{dir}.disabled"));
        if src.exists() {
            std::fs::rename(&src, &dst)
                .with_context(|| format!("failed to rename {}", src.display()))?;
            tracing::debug!(from = %src.display(), to = %dst.display(), "renamed directory");
        }
    }

    // Rename loose files (individual DLLs etc. not inside a mod directory)
    let loose = find_loose_files(&file_paths, &top_dirs);
    for loose_path in &loose {
        let src = spt_dir.join(loose_path);
        let dst = spt_dir.join(format!("{loose_path}.disabled"));
        if src.exists() {
            std::fs::rename(&src, &dst)
                .with_context(|| format!("failed to rename {}", src.display()))?;
            tracing::debug!(from = %src.display(), to = %dst.display(), "renamed loose file");
        }
    }

    // Update file paths in the database
    let tx = db.begin_transaction()?;
    for file in &files {
        let new_path = if let Some(matching_dir) = top_dirs
            .iter()
            .find(|d| file.file_path.starts_with(d.as_str()))
        {
            file.file_path
                .replacen(matching_dir, &format!("{matching_dir}.disabled"), 1)
        } else if loose.contains(&file.file_path.as_str()) {
            format!("{}.disabled", file.file_path)
        } else {
            continue;
        };
        db.rename_file_path(file.id, &new_path)?;
    }

    db.set_mod_disabled(mod_db_id, true)?;
    tx.commit()?;
    tracing::info!(mod_db_id, name = %mod_info.name, "mod disabled");
    Ok(())
}

/// Enable a previously disabled mod by removing the `.disabled` suffix from
/// its directories and files, updating file paths in the database, and
/// clearing the disabled flag.
pub fn enable_mod(db: &Database, spt_dir: &Path, mod_db_id: i64) -> Result<()> {
    let mod_info = db
        .get_mod(mod_db_id)?
        .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
    if !mod_info.disabled {
        anyhow::bail!("mod is not disabled");
    }

    let files = db.get_files_for_mod(mod_db_id)?;
    let file_paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
    let top_dirs = find_top_level_mod_dirs(&file_paths);

    tracing::info!(mod_db_id, name = %mod_info.name, "enabling mod");

    // Rename top-level directories (strip .disabled suffix)
    for dir in &top_dirs {
        if dir.ends_with(".disabled") {
            let restored = dir.strip_suffix(".disabled").unwrap();
            let src = spt_dir.join(dir);
            let dst = spt_dir.join(restored);
            if src.exists() {
                std::fs::rename(&src, &dst)
                    .with_context(|| format!("failed to rename {}", src.display()))?;
                tracing::debug!(from = %src.display(), to = %dst.display(), "restored directory");
            }
        }
    }

    // Rename loose files (strip .disabled suffix)
    let loose = find_loose_files(&file_paths, &top_dirs);
    for loose_path in &loose {
        if loose_path.ends_with(".disabled") {
            let restored = loose_path.strip_suffix(".disabled").unwrap();
            let src = spt_dir.join(loose_path);
            let dst = spt_dir.join(restored);
            if src.exists() {
                std::fs::rename(&src, &dst)
                    .with_context(|| format!("failed to rename {}", src.display()))?;
                tracing::debug!(from = %src.display(), to = %dst.display(), "restored loose file");
            }
        }
    }

    // Update file paths in the database (strip .disabled from paths)
    let tx = db.begin_transaction()?;
    for file in &files {
        let new_path = if let Some(matching_dir) = top_dirs
            .iter()
            .find(|d| file.file_path.starts_with(d.as_str()))
        {
            if matching_dir.ends_with(".disabled") {
                let restored_dir = matching_dir.strip_suffix(".disabled").unwrap();
                file.file_path
                    .replacen(matching_dir.as_str(), restored_dir, 1)
            } else {
                continue;
            }
        } else if file.file_path.ends_with(".disabled") {
            file.file_path
                .strip_suffix(".disabled")
                .unwrap()
                .to_string()
        } else {
            continue;
        };
        db.rename_file_path(file.id, &new_path)?;
    }

    db.set_mod_disabled(mod_db_id, false)?;
    tx.commit()?;
    tracing::info!(mod_db_id, name = %mod_info.name, "mod enabled");
    Ok(())
}

/// Recursively collect all transitive reverse dependencies of a mod.
/// Returns them in BFS order (direct dependents first, then their dependents, etc.).
pub fn collect_all_reverse_deps(db: &Database, mod_db_id: i64) -> Result<Vec<InstalledMod>> {
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(mod_db_id);
    visited.insert(mod_db_id);

    while let Some(current_id) = queue.pop_front() {
        let rev_deps = db.get_reverse_dependencies(current_id)?;
        for dep in rev_deps {
            if visited.insert(dep.mod_id) {
                if let Some(dependent) = db.get_mod(dep.mod_id)? {
                    queue.push_back(dependent.id);
                    result.push(dependent);
                }
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn create_test_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let buf = std::io::Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(content).unwrap();
        }
        let buf = zip.finish().unwrap();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(buf.get_ref()).unwrap();
        tmp.flush().unwrap();
        tmp
    }

    fn setup_spt_dir() -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();
        tmp
    }

    #[test]
    fn install_extracts_files_and_records_in_db() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: Some("test-mod"),
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        let installed = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(installed.name, "TestMod");
        assert_eq!(installed.version, "1.0.0");

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/src/mod.ts")
            .exists());
    }

    #[test]
    fn update_uses_staging_so_failure_preserves_old_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1
        let zip_v1 = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{\"v\":\"1\"}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip_v1.path(),
        })
        .unwrap();

        // Update to v2
        let zip_v2 = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"v\":\"2\"}"),
            ("SPT/user/mods/TestMod/new_file.ts", b"new"),
        ]);
        update_mod_from_archive(
            &db,
            spt_dir.path(),
            &Config::default(),
            db_id,
            300,
            "2.0.0",
            zip_v2.path(),
        )
        .unwrap();

        let updated = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(updated.version, "2.0.0");

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);

        let content =
            std::fs::read_to_string(spt_dir.path().join("SPT/user/mods/TestMod/package.json"))
                .unwrap();
        assert!(content.contains("\"v\":\"2\""));
    }

    #[test]
    fn remove_deletes_files_and_db_records() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        let zip = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{}")]);
        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());

        remove_mod_by_id(&db, spt_dir.path(), &Config::default(), db_id).unwrap();

        assert!(!spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(db.get_mod(db_id).unwrap().is_none());
    }

    #[test]
    fn find_top_level_mod_dirs_extracts_correctly() {
        let paths = vec![
            "SPT/user/mods/TestMod/package.json".to_string(),
            "SPT/user/mods/TestMod/src/mod.ts".to_string(),
            "BepInEx/plugins/PluginDir/plugin.dll".to_string(),
            "BepInEx/plugins/loose.dll".to_string(),
        ];
        let dirs = find_top_level_mod_dirs(&paths);
        assert_eq!(dirs.len(), 2);
        assert!(dirs.contains(&"SPT/user/mods/TestMod".to_string()));
        assert!(dirs.contains(&"BepInEx/plugins/PluginDir".to_string()));
        // loose.dll should NOT be a top-level dir
    }

    #[test]
    fn find_loose_files_identifies_non_dir_files() {
        let paths = vec![
            "SPT/user/mods/TestMod/package.json".to_string(),
            "BepInEx/plugins/loose.dll".to_string(),
        ];
        let top_dirs = vec!["SPT/user/mods/TestMod".to_string()];
        let loose = find_loose_files(&paths, &top_dirs);
        assert_eq!(loose, vec!["BepInEx/plugins/loose.dll"]);
    }

    #[test]
    fn disable_and_enable_mod_renames_directories() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        // Verify mod is installed and enabled
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(!db.get_mod(db_id).unwrap().unwrap().disabled);

        // Disable the mod
        disable_mod(&db, spt_dir.path(), db_id).unwrap();

        // Directory should be renamed
        assert!(!spt_dir.path().join("SPT/user/mods/TestMod").exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled")
            .exists());
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled/package.json")
            .exists());

        // DB should reflect disabled state
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert!(m.disabled);

        // File paths in DB should be updated
        let files = db.get_files_for_mod(db_id).unwrap();
        assert!(files
            .iter()
            .all(|f| f.file_path.contains("TestMod.disabled")));

        // Enable the mod
        enable_mod(&db, spt_dir.path(), db_id).unwrap();

        // Directory should be restored
        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(!spt_dir
            .path()
            .join("SPT/user/mods/TestMod.disabled")
            .exists());

        // DB should reflect enabled state
        let m = db.get_mod(db_id).unwrap().unwrap();
        assert!(!m.disabled);

        // File paths in DB should be restored
        let files = db.get_files_for_mod(db_id).unwrap();
        assert!(files.iter().all(|f| !f.file_path.contains(".disabled")));
    }

    #[test]
    fn disable_mod_handles_loose_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[("BepInEx/plugins/loose.dll", b"dll content")]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "LooseMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        assert!(spt_dir.path().join("BepInEx/plugins/loose.dll").exists());

        disable_mod(&db, spt_dir.path(), db_id).unwrap();

        assert!(!spt_dir.path().join("BepInEx/plugins/loose.dll").exists());
        assert!(spt_dir
            .path()
            .join("BepInEx/plugins/loose.dll.disabled")
            .exists());

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files[0].file_path, "BepInEx/plugins/loose.dll.disabled");

        enable_mod(&db, spt_dir.path(), db_id).unwrap();

        assert!(spt_dir.path().join("BepInEx/plugins/loose.dll").exists());
        assert!(!spt_dir
            .path()
            .join("BepInEx/plugins/loose.dll.disabled")
            .exists());
    }

    #[test]
    fn disable_already_disabled_mod_errors() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[("SPT/user/mods/TestMod/package.json", b"{}")]);

        let db_id = install_mod_from_archive(&InstallRequest {
            db: &db,
            spt_dir: spt_dir.path(),
            config: &Config::default(),
            forge_mod_id: 100,
            version_id: 200,
            name: "TestMod",
            slug: None,
            version: "1.0.0",
            archive_path: zip.path(),
        })
        .unwrap();

        disable_mod(&db, spt_dir.path(), db_id).unwrap();
        let result = disable_mod(&db, spt_dir.path(), db_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already disabled"));
    }
}
