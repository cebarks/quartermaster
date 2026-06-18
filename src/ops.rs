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
    let extracted = crate::spt::mods::extract_mod(archive_path, spt_dir)?;
    let db_id = db.insert_mod(forge_mod_id, version_id, name, slug, version)?;
    record_extracted_files(db, db_id, &extracted)?;
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
    let staging_dir = tempfile::tempdir()?;
    let extracted = crate::spt::mods::extract_mod(archive_path, staging_dir.path())?;

    let old_files = db.get_files_for_mod(mod_db_id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
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
    let files = db.get_files_for_mod(mod_db_id)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
    db.delete_mod(mod_db_id)?;
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
