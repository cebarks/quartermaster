use std::path::Path;

use anyhow::Result;

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

#[allow(clippy::too_many_arguments)]
pub fn install_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    forge_mod_id: i64,
    version_id: i64,
    name: &str,
    slug: Option<&str>,
    version: &str,
    archive_path: &Path,
) -> Result<i64> {
    tracing::info!(name, forge_mod_id, version, "installing mod from archive");
    let extracted = crate::spt::mods::extract_mod(archive_path, spt_dir)?;
    let db_id = db.insert_mod(forge_mod_id, version_id, name, slug, version)?;
    record_extracted_files(db, db_id, &extracted)?;
    tracing::debug!(
        db_id,
        file_count = extracted.len(),
        "mod installed, files recorded"
    );
    Ok(db_id)
}

pub fn update_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
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
    Ok(())
}

pub fn remove_mod_by_id(db: &Database, spt_dir: &Path, mod_db_id: i64) -> Result<()> {
    tracing::info!(mod_db_id, "removing mod");
    let files = db.get_files_for_mod(mod_db_id)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    tracing::debug!(file_count = paths.len(), "deleting mod files");
    crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
    db.delete_mod(mod_db_id)?;
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
                let _ = db.insert_file_with_source(
                    mod_db_id,
                    &rel_str,
                    Some(&hash),
                    Some(size),
                    "runtime",
                );
            }
        }
    }
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

        let db_id = install_mod_from_archive(
            &db,
            spt_dir.path(),
            100,
            200,
            "TestMod",
            Some("test-mod"),
            "1.0.0",
            zip.path(),
        )
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
        let db_id = install_mod_from_archive(
            &db,
            spt_dir.path(),
            100,
            200,
            "TestMod",
            None,
            "1.0.0",
            zip_v1.path(),
        )
        .unwrap();

        // Update to v2
        let zip_v2 = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"v\":\"2\"}"),
            ("SPT/user/mods/TestMod/new_file.ts", b"new"),
        ]);
        update_mod_from_archive(&db, spt_dir.path(), db_id, 300, "2.0.0", zip_v2.path()).unwrap();

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
        let db_id = install_mod_from_archive(
            &db,
            spt_dir.path(),
            100,
            200,
            "TestMod",
            None,
            "1.0.0",
            zip.path(),
        )
        .unwrap();

        assert!(spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());

        remove_mod_by_id(&db, spt_dir.path(), db_id).unwrap();

        assert!(!spt_dir
            .path()
            .join("SPT/user/mods/TestMod/package.json")
            .exists());
        assert!(db.get_mod(db_id).unwrap().is_none());
    }
}
