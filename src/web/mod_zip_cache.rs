use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use globset::{Glob, GlobSetBuilder};
use parking_lot::Mutex;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::config::{Config, SetupZipConfig};
use crate::db::mods::InstalledFile;
use crate::db::Database;

const NON_ESSENTIAL_NAMES: &[&str] = &[
    "readme",
    "readme.md",
    "readme.txt",
    "license",
    "license.txt",
    "license.md",
    "changelog",
    "changelog.md",
    "changelog.txt",
];

const NON_ESSENTIAL_EXTENSIONS: &[&str] = &["url", "html", "htm"];

fn is_non_essential(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let lower = filename.to_ascii_lowercase();
    if NON_ESSENTIAL_NAMES.contains(&lower.as_str()) {
        return true;
    }
    if let Some(ext) = lower.rsplit('.').next() {
        if ext != lower && NON_ESSENTIAL_EXTENSIONS.contains(&ext) {
            return true;
        }
    }
    false
}

pub fn filter_setup_zip_files(
    files: Vec<InstalledFile>,
    config: &SetupZipConfig,
) -> Vec<InstalledFile> {
    let include_set = build_globset(&config.include_patterns);
    let exclude_set = build_globset(&config.exclude_patterns);

    files
        .into_iter()
        .filter(|f| {
            let path = &f.file_path;

            // 1. Force-include overrides everything
            if let Some(ref set) = include_set {
                if set.is_match(path) {
                    return true;
                }
            }

            // 2. User exclude patterns
            if let Some(ref set) = exclude_set {
                if set.is_match(path) {
                    return false;
                }
            }

            // 3. Server-only files
            if config.exclude_server_files && path.starts_with("user/mods/") {
                return false;
            }

            // 4. Non-essential files
            if config.exclude_non_essential && is_non_essential(path) {
                return false;
            }

            true
        })
        .collect()
}

fn build_globset(patterns: &[String]) -> Option<globset::GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(e) => {
                tracing::warn!(pattern = %pattern, err = %e, "invalid glob pattern in setup_zip config, skipping");
            }
        }
    }
    builder.build().ok()
}

/// Write a ZIP archive of mod files directly to disk (no in-memory buffer).
pub fn build_mod_zip_to_file(
    spt_dir: &Path,
    files: &[InstalledFile],
    dest: &Path,
) -> anyhow::Result<()> {
    let file = BufWriter::new(File::create(dest)?);
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for f in files {
        let full_path = spt_dir.join(&f.file_path);
        match std::fs::read(&full_path) {
            Ok(data) => {
                zip.start_file(&f.file_path, options)?;
                std::io::Write::write_all(&mut zip, &data)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %f.file_path, "skipping missing file in mod zip cache");
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    zip.finish()?;
    Ok(())
}

struct Inner {
    spt_dir: PathBuf,
    db: Arc<Mutex<Database>>,
    config: Arc<parking_lot::RwLock<Config>>,
    cache_path: PathBuf,
    tmp_path: PathBuf,
    rebuilding: AtomicBool,
    dirty: AtomicBool,
}

#[derive(Clone)]
pub struct ModZipCache {
    inner: Arc<Inner>,
}

impl ModZipCache {
    pub fn new(
        spt_dir: PathBuf,
        db: Arc<Mutex<Database>>,
        config: Arc<parking_lot::RwLock<Config>>,
    ) -> Self {
        let cache_dir = spt_dir.join("quartermaster-cache");
        let _ = std::fs::create_dir_all(&cache_dir);

        Self {
            inner: Arc::new(Inner {
                cache_path: cache_dir.join("all-mods.zip"),
                tmp_path: cache_dir.join("all-mods.zip.tmp"),
                spt_dir,
                db,
                config,
                rebuilding: AtomicBool::new(false),
                dirty: AtomicBool::new(false),
            }),
        }
    }

    /// Returns the cached ZIP path if it exists on disk.
    pub fn get(&self) -> Option<PathBuf> {
        if self.inner.cache_path.exists() {
            Some(self.inner.cache_path.clone())
        } else {
            None
        }
    }

    /// Trigger a background rebuild. If a rebuild is already in progress,
    /// sets a dirty flag so a follow-up rebuild runs when the current one finishes.
    pub fn invalidate(&self) {
        if self
            .inner
            .rebuilding
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // Rebuild in progress — mark dirty so it re-triggers
            self.inner.dirty.store(true, Ordering::Release);
            return;
        }

        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.do_rebuild();
        });
    }

    /// Synchronous rebuild for use in tests. Bails out if a rebuild is
    /// already in progress (the async path will handle it).
    #[cfg(test)]
    pub fn rebuild_sync(&self) {
        if self
            .inner
            .rebuilding
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        self.do_rebuild();
    }

    fn do_rebuild(&self) {
        // Clear dirty before we start so we detect changes during the rebuild
        self.inner.dirty.store(false, Ordering::Release);

        // Scoped guard ensures rebuilding flag is cleared even on panic
        {
            struct ResetOnDrop<'a>(&'a AtomicBool);
            impl Drop for ResetOnDrop<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _guard = ResetOnDrop(&self.inner.rebuilding);

            if let Err(e) = self.build_cache() {
                tracing::warn!(err = %e, "failed to rebuild mod zip cache");
            }
        }
        // rebuilding is now false (guard dropped)

        // If someone called invalidate() while we were rebuilding, go again
        if self.inner.dirty.swap(false, Ordering::AcqRel) {
            self.invalidate();
        }
    }

    fn build_cache(&self) -> anyhow::Result<()> {
        let files = {
            let db = self.inner.db.lock();
            db.get_all_enabled_mod_files()?
        };
        let setup_zip_config = {
            let config = self.inner.config.read();
            config.setup_zip.clone()
        };

        let files = filter_setup_zip_files(files, &setup_zip_config);

        if files.is_empty() {
            let _ = std::fs::remove_file(&self.inner.cache_path);
            return Ok(());
        }

        build_mod_zip_to_file(&self.inner.spt_dir, &files, &self.inner.tmp_path)?;
        std::fs::rename(&self.inner.tmp_path, &self.inner.cache_path)?;

        tracing::debug!(
            path = %self.inner.cache_path.display(),
            files = files.len(),
            "mod zip cache rebuilt"
        );

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::SetupZipConfig;
    use crate::db::mods::InstalledFile;
    use std::io::Write;

    fn test_config() -> Arc<parking_lot::RwLock<crate::config::Config>> {
        let mut c = crate::config::Config::default();
        c.setup_zip.exclude_server_files = false;
        c.setup_zip.exclude_non_essential = false;
        Arc::new(parking_lot::RwLock::new(c))
    }

    fn make_test_file(dir: &std::path::Path, rel_path: &str, content: &[u8]) {
        let full = dir.join(rel_path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(full).unwrap();
        f.write_all(content).unwrap();
    }

    fn test_file(mod_id: i64, path: &str) -> InstalledFile {
        InstalledFile {
            id: 0,
            mod_id: Some(mod_id),
            addon_id: None,
            file_path: path.to_string(),
            file_hash: None,
            file_size: None,
            source: "archive".to_string(),
        }
    }

    #[test]
    fn build_zip_to_file_creates_valid_archive() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(
            spt_dir.path(),
            "user/mods/test/package.json",
            b"{\"name\":\"test\"}",
        );

        let files = vec![test_file(1, "user/mods/test/package.json")];

        let out = spt_dir.path().join("out.zip");
        build_mod_zip_to_file(spt_dir.path(), &files, &out).unwrap();

        assert!(out.exists());
        let data = std::fs::read(&out).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        assert_eq!(reader.len(), 1);
    }

    #[test]
    fn build_zip_to_file_skips_missing_files() {
        let spt_dir = tempfile::tempdir().unwrap();
        let files = vec![test_file(1, "user/mods/ghost/package.json")];

        let out = spt_dir.path().join("out.zip");
        build_mod_zip_to_file(spt_dir.path(), &files, &out).unwrap();

        let data = std::fs::read(&out).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn cache_get_returns_none_when_no_file() {
        let spt_dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc, test_config());
        assert!(cache.get().is_none());
    }

    #[test]
    fn cache_get_returns_path_after_rebuild() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "user/mods/test/package.json", b"{}");

        let db = Database::open_in_memory().unwrap();
        // Insert a mod + file so the query returns something
        db.insert_mod(1, 1, "test-mod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(1, "user/mods/test/package.json", Some("abc123"), Some(2))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc, test_config());

        // Directly call rebuild (synchronous, for testing)
        cache.rebuild_sync();

        let path = cache.get();
        assert!(path.is_some());
        assert!(path.unwrap().exists());
    }

    #[test]
    fn rebuild_replaces_cache_with_updated_content() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "user/mods/a/package.json", b"{\"a\":true}");

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "mod-a", Some("mod-a"), "1.0.0")
            .unwrap();
        db.insert_file(1, "user/mods/a/package.json", Some("h1"), Some(10))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc.clone(), test_config());
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let size_before = std::fs::metadata(&path).unwrap().len();

        // Add a second mod file
        make_test_file(spt_dir.path(), "user/mods/b/package.json", b"{\"b\":true}");
        {
            let db = db_arc.lock();
            db.insert_mod(2, 2, "mod-b", Some("mod-b"), "1.0.0")
                .unwrap();
            db.insert_file(2, "user/mods/b/package.json", Some("h2"), Some(10))
                .unwrap();
        }

        // Rebuild and verify the zip grew
        cache.rebuild_sync();
        let size_after = std::fs::metadata(&path).unwrap().len();
        assert!(
            size_after > size_before,
            "zip should be larger after adding a mod"
        );
    }

    #[test]
    fn empty_mods_removes_cache_file() {
        let spt_dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc, test_config());

        // Create a dummy cache file
        std::fs::write(&cache.inner.cache_path, b"dummy").unwrap();
        assert!(cache.get().is_some());

        // Rebuild with empty DB should remove it
        cache.rebuild_sync();
        assert!(cache.get().is_none());
    }

    #[test]
    fn filter_excludes_server_files_by_default() {
        let config = SetupZipConfig::default();
        let files = vec![
            test_file(1, "user/mods/test-mod/package.json"),
            test_file(1, "BepInEx/plugins/test-mod/test.dll"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, "BepInEx/plugins/test-mod/test.dll");
    }

    #[test]
    fn filter_keeps_server_files_when_disabled() {
        let config = SetupZipConfig {
            exclude_server_files: false,
            ..SetupZipConfig::default()
        };
        let files = vec![
            test_file(1, "user/mods/test-mod/package.json"),
            test_file(1, "BepInEx/plugins/test-mod/test.dll"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_excludes_non_essential_by_default() {
        let config = SetupZipConfig::default();
        let files = vec![
            test_file(1, "BepInEx/plugins/mod/mod.dll"),
            test_file(1, "BepInEx/plugins/mod/README.md"),
            test_file(1, "BepInEx/plugins/mod/LICENSE"),
            test_file(1, "BepInEx/plugins/mod/CHANGELOG.txt"),
            test_file(1, "BepInEx/plugins/mod/info.url"),
            test_file(1, "BepInEx/plugins/mod/docs.html"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, "BepInEx/plugins/mod/mod.dll");
    }

    #[test]
    fn filter_non_essential_case_insensitive() {
        let config = SetupZipConfig::default();
        let files = vec![
            test_file(1, "BepInEx/plugins/mod/readme.MD"),
            test_file(1, "BepInEx/plugins/mod/license.TXT"),
            test_file(1, "BepInEx/plugins/mod/Changelog.md"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_user_exclude_patterns() {
        let config = SetupZipConfig {
            exclude_patterns: vec!["**/*.pdb".to_string()],
            ..SetupZipConfig::default()
        };
        let files = vec![
            test_file(1, "BepInEx/plugins/mod/mod.dll"),
            test_file(1, "BepInEx/plugins/mod/mod.pdb"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, "BepInEx/plugins/mod/mod.dll");
    }

    #[test]
    fn filter_include_overrides_all_excludes() {
        let config = SetupZipConfig {
            exclude_server_files: true,
            exclude_non_essential: true,
            exclude_patterns: vec!["**/*.json".to_string()],
            include_patterns: vec!["user/mods/special/**".to_string()],
        };
        let files = vec![
            test_file(1, "user/mods/special/package.json"),
            test_file(1, "user/mods/other/package.json"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, "user/mods/special/package.json");
    }

    #[test]
    fn filter_all_disabled_passes_everything() {
        let config = SetupZipConfig {
            exclude_server_files: false,
            exclude_non_essential: false,
            exclude_patterns: vec![],
            include_patterns: vec![],
        };
        let files = vec![
            test_file(1, "user/mods/mod/package.json"),
            test_file(1, "BepInEx/plugins/mod/README.md"),
            test_file(1, "BepInEx/plugins/mod/mod.dll"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn filter_invalid_glob_is_skipped_gracefully() {
        let config = SetupZipConfig {
            exclude_patterns: vec!["[invalid".to_string()],
            ..SetupZipConfig::default()
        };
        let files = vec![test_file(1, "BepInEx/plugins/mod/mod.dll")];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn cache_rebuild_excludes_server_files() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(
            spt_dir.path(),
            "BepInEx/plugins/mod/mod.dll",
            b"dll-content",
        );
        make_test_file(spt_dir.path(), "user/mods/server-mod/package.json", b"{}");

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "hybrid-mod", Some("hybrid-mod"), "1.0.0")
            .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/mod.dll", Some("aaa"), Some(11))
            .unwrap();
        db.insert_file(1, "user/mods/server-mod/package.json", Some("bbb"), Some(2))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        // Default config has exclude_server_files=true
        let config = Arc::new(parking_lot::RwLock::new(crate::config::Config::default()));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc, config);
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let data = std::fs::read(&path).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();

        // Default config excludes server files
        assert_eq!(reader.len(), 1);
        let names: Vec<_> = reader.file_names().collect();
        assert!(names.contains(&"BepInEx/plugins/mod/mod.dll"));
        assert!(!names.contains(&"user/mods/server-mod/package.json"));
    }

    #[test]
    fn cache_rebuild_excludes_non_essential() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "BepInEx/plugins/mod/mod.dll", b"content");
        make_test_file(spt_dir.path(), "BepInEx/plugins/mod/README.md", b"# Readme");

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "test-mod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/mod.dll", Some("aaa"), Some(7))
            .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/README.md", Some("bbb"), Some(8))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        // Default config has exclude_non_essential=true
        let config = Arc::new(parking_lot::RwLock::new(crate::config::Config::default()));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc, config);
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let data = std::fs::read(&path).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();

        assert_eq!(reader.len(), 1);
        assert!(reader
            .file_names()
            .any(|n| n == "BepInEx/plugins/mod/mod.dll"));
    }

    #[test]
    fn cache_rebuild_with_all_filters_disabled() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "user/mods/mod/package.json", b"{}");
        make_test_file(spt_dir.path(), "BepInEx/plugins/mod/README.md", b"# Hi");

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "test-mod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(1, "user/mods/mod/package.json", Some("aaa"), Some(2))
            .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/README.md", Some("bbb"), Some(4))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        let config = Arc::new(parking_lot::RwLock::new({
            let mut c = crate::config::Config::default();
            c.setup_zip.exclude_server_files = false;
            c.setup_zip.exclude_non_essential = false;
            c
        }));
        let cache = ModZipCache::new(spt_dir.path().to_path_buf(), db_arc, config);
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let data = std::fs::read(&path).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();

        assert_eq!(reader.len(), 2);
    }
}
