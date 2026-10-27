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
use crate::dirs::QumaDirs;
use crate::spt::scoped::ScopedDir;

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
        .filter_map(
            |mut f| match crate::spt::scoped::relative_path(&f.file_path) {
                Ok(path) => {
                    f.file_path = path.to_string_lossy().into_owned();
                    Some(f)
                }
                Err(e) => {
                    tracing::warn!(path = %f.file_path, err = %e, "skipping unsafe setup ZIP path");
                    None
                }
            },
        )
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
            if config.exclude_server_files && crate::spt::layout::is_server_path(path) {
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

/// Write a ZIP archive directly to disk, reading runtime, mod, then base layers.
/// Works without a mounted overlay. When `client_only` is true, both complete
/// server runtime trees (including aliases to them) are excluded.
pub fn build_mod_zip_to_file(
    dirs: &QumaDirs,
    files: &[InstalledFile],
    dest: &Path,
    client_only: bool,
) -> anyhow::Result<()> {
    let roots = if dirs.is_legacy() {
        vec![dirs.spt_server.clone()]
    } else {
        vec![
            dirs.runtime_upper(),
            dirs.mod_overlay(),
            dirs.spt_server.clone(),
        ]
    };
    let mut scopes = Vec::new();
    for root in roots {
        match ScopedDir::open(&root) {
            Ok(scope) => scopes.push(scope),
            Err(e) if is_not_found(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    let file = BufWriter::new(File::create(dest)?);
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for f in files {
        let relative = match crate::spt::scoped::relative_path(&f.file_path) {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!(path = %f.file_path, err = %e, "skipping unsafe mod ZIP path");
                continue;
            }
        };
        let archive_name = relative.to_string_lossy();
        if client_only && crate::spt::layout::is_server_path(&archive_name) {
            continue;
        }
        let source = read_zip_source(&scopes, &relative, client_only);
        match source {
            Ok(mut source) => {
                zip.start_file(archive_name.as_ref(), options)?;
                std::io::copy(&mut source, &mut zip)?;
            }
            Err(e) => {
                tracing::warn!(path = %f.file_path, err = %e, "unsafe or missing mod ZIP source, skipping");
            }
        }
    }

    zip.finish()?;
    Ok(())
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

fn read_zip_source(
    scopes: &[ScopedDir],
    relative: &Path,
    client_only: bool,
) -> anyhow::Result<File> {
    for scope in scopes {
        let result = if client_only {
            scope.read_client_file(relative)
        } else {
            scope.read_file(relative)
        };
        match result {
            Err(e) if is_not_found(&e) && !scope.blocks_lower_layer(relative)? => continue,
            result => return result,
        }
    }
    anyhow::bail!("mod ZIP source does not exist in any installation layer")
}

struct Inner {
    dirs: QumaDirs,
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
        dirs: QumaDirs,
        db: Arc<Mutex<Database>>,
        config: Arc<parking_lot::RwLock<Config>>,
    ) -> Self {
        let cache_dir = dirs.spt_server.join("quartermaster-cache");
        let _ = std::fs::create_dir_all(&cache_dir);

        Self {
            inner: Arc::new(Inner {
                cache_path: cache_dir.join("all-mods.zip"),
                tmp_path: cache_dir.join("all-mods.zip.tmp"),
                dirs,
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

        build_mod_zip_to_file(&self.inner.dirs, &files, &self.inner.tmp_path, false)?;
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
    fn current_runtime_server_files_are_excluded_from_client_exports() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = [
            "SPT_Runtime/user/mods/Test/package.json",
            "SPT/user/mods/Old/package.json",
            "SPT_Runtime/SPT_Data/configs/core.json",
            "SPT/user/profiles/player.json",
            "./SPT_Runtime/user/profiles/alias.json",
            "./SPT/user/profiles/alias.json",
            "C:/payload.dll",
            "\\rooted.dll",
            "BepInEx/plugins/Client.dll",
        ];
        for path in paths {
            make_test_file(tmp.path(), path, b"test");
        }
        let files: Vec<_> = paths.iter().map(|p| test_file(1, p)).collect();
        let filtered = filter_setup_zip_files(files.clone(), &SetupZipConfig::default());
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, "BepInEx/plugins/Client.dll");
        let out = tmp.path().join("client.zip");
        build_mod_zip_to_file(
            &QumaDirs::from_legacy(tmp.path().to_path_buf()),
            &files,
            &out,
            true,
        )
        .unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(out).unwrap()).unwrap();
        assert_eq!(zip.len(), 1);
        assert_eq!(
            zip.file_names().next().unwrap().unwrap(),
            "BepInEx/plugins/Client.dll"
        );
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_mod(None, None, "Hybrid", None, "1", "local", None, None)
            .unwrap();
        for path in paths {
            db.insert_file(id, path, None, None).unwrap();
        }
        let out = crate::convoy::download::get_or_build_convoy_zip(&db, tmp.path(), &[id], false)
            .unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(out).unwrap()).unwrap();
        assert_eq!(
            zip.file_names()
                .collect::<zip::result::ZipResult<Vec<_>>>()
                .unwrap(),
            vec!["BepInEx/plugins/Client.dll"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn client_exports_reject_server_target_aliases_but_keep_client_links() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        make_test_file(
            tmp.path(),
            "SPT_Runtime/user/profiles/player.json",
            b"server-secret",
        );
        make_test_file(
            tmp.path(),
            "LegacyStorage/user/config.json",
            b"legacy-secret",
        );
        make_test_file(tmp.path(), "Assets/client.dat", b"client-data");
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();
        symlink("LegacyStorage", tmp.path().join("SPT")).unwrap();
        let paths = [
            (
                "BepInEx/plugins/Current.dll",
                "../../SPT_Runtime/user/profiles/player.json",
            ),
            ("BepInEx/plugins/Legacy.dll", "../../SPT/user/config.json"),
            ("BepInEx/plugins/Client.dll", "../../Assets/client.dat"),
        ];
        for (name, target) in paths {
            symlink(target, tmp.path().join(name)).unwrap();
        }
        symlink(
            tmp.path().join("SPT_Runtime/user/profiles/player.json"),
            tmp.path().join("BepInEx/plugins/Absolute.dll"),
        )
        .unwrap();
        let names = paths
            .iter()
            .map(|(name, _)| *name)
            .chain(["BepInEx/plugins/Absolute.dll"]);
        let files: Vec<_> = names.map(|name| test_file(1, name)).collect();
        let out = tmp.path().join("client.zip");
        build_mod_zip_to_file(
            &QumaDirs::from_legacy(tmp.path().to_path_buf()),
            &files,
            &out,
            true,
        )
        .unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(out).unwrap()).unwrap();
        assert_eq!(
            zip.file_names()
                .collect::<zip::result::ZipResult<Vec<_>>>()
                .unwrap(),
            vec!["BepInEx/plugins/Client.dll"]
        );
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_mod(None, None, "Hybrid", None, "1", "local", None, None)
            .unwrap();
        for file in files {
            db.insert_file(id, &file.file_path, None, None).unwrap();
        }
        let out = crate::convoy::download::get_or_build_convoy_zip(&db, tmp.path(), &[id], false)
            .unwrap();
        let zip = zip::ZipArchive::new(std::fs::File::open(out).unwrap()).unwrap();
        assert_eq!(
            zip.file_names()
                .collect::<zip::result::ZipResult<Vec<_>>>()
                .unwrap(),
            vec!["BepInEx/plugins/Client.dll"]
        );
    }

    #[test]
    fn build_zip_to_file_creates_valid_archive() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(
            spt_dir.path(),
            "SPT/user/mods/test/package.json",
            b"{\"name\":\"test\"}",
        );

        let files = vec![test_file(1, "SPT/user/mods/test/package.json")];

        let out = spt_dir.path().join("out.zip");
        build_mod_zip_to_file(
            &QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            &files,
            &out,
            false,
        )
        .unwrap();

        assert!(out.exists());
        let data = std::fs::read(&out).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        assert_eq!(reader.len(), 1);
    }

    #[test]
    fn build_zip_to_file_skips_missing_files() {
        let spt_dir = tempfile::tempdir().unwrap();
        let files = vec![test_file(1, "SPT/user/mods/ghost/package.json")];

        let out = spt_dir.path().join("out.zip");
        build_mod_zip_to_file(
            &QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            &files,
            &out,
            false,
        )
        .unwrap();

        let data = std::fs::read(&out).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn cache_reads_overlay_layers_without_a_mounted_server() {
        use std::io::Read;
        let root = tempfile::tempdir().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(root.path().to_path_buf());
        make_test_file(&dirs.spt_server, "BepInEx/plugins/base.dll", b"base");
        make_test_file(&dirs.spt_server, "BepInEx/plugins/override.dll", b"old");
        make_test_file(&dirs.mod_overlay(), "BepInEx/plugins/override.dll", b"mod");
        make_test_file(&dirs.mod_overlay(), "BepInEx/plugins/Fika.dll", b"fika");
        make_test_file(
            &dirs.runtime_upper(),
            "BepInEx/plugins/override.dll",
            b"runtime",
        );
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(Some(1), Some(1), "Fika", None, "2.4.6", "forge", None, None)
            .unwrap();
        for name in ["base.dll", "override.dll", "Fika.dll"] {
            db.insert_file(1, &format!("BepInEx/plugins/{name}"), None, None)
                .unwrap();
        }
        let cache = ModZipCache::new(dirs.clone(), Arc::new(Mutex::new(db)), test_config());
        cache.rebuild_sync();
        let mut zip = zip::ZipArchive::new(File::open(cache.get().unwrap()).unwrap()).unwrap();
        assert_eq!(
            zip.len(),
            3,
            "the client download must include installed overlay files"
        );
        for (name, expected) in [
            ("base.dll", "base"),
            ("override.dll", "runtime"),
            ("Fika.dll", "fika"),
        ] {
            let mut content = String::new();
            zip.by_name(&format!("BepInEx/plugins/{name}"))
                .unwrap()
                .read_to_string(&mut content)
                .unwrap();
            assert_eq!(content, expected);
        }
        assert!(
            !dirs.spt_runtime().exists(),
            "export must not require a FUSE mount"
        );
    }

    #[cfg(unix)]
    #[test]
    fn layered_zip_rejects_unsafe_shadow_without_falling_back() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"secret").unwrap();
        let dirs = QumaDirs::from_root(root.path().to_path_buf());
        let name = "BepInEx/plugins/shadow.dll";
        make_test_file(&dirs.spt_server, name, b"base");
        let shadow = dirs.mod_overlay().join(name);
        std::fs::create_dir_all(shadow.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(outside.path(), &shadow).unwrap();
        let dest = root.path().join("out.zip");
        build_mod_zip_to_file(&dirs, &[test_file(1, name)], &dest, false).unwrap();
        let zip = zip::ZipArchive::new(File::open(dest).unwrap()).unwrap();
        assert!(
            zip.is_empty(),
            "unsafe higher layers must not expose outside or stale base files"
        );
    }

    #[cfg(unix)]
    #[test]
    fn layered_zip_preserves_missing_shadow_entries() {
        for shadow_kind in ["dangling-file", "dangling-directory", "whiteout", "opaque"] {
            let root = tempfile::tempdir().unwrap();
            let dirs = QumaDirs::from_root(root.path().to_path_buf());
            let name = "BepInEx/plugins/Test.dll";
            make_test_file(&dirs.spt_server, name, b"stale base");
            let upper = dirs.runtime_upper();
            std::fs::create_dir_all(upper.join("BepInEx")).unwrap();
            if shadow_kind == "dangling-directory" {
                std::os::unix::fs::symlink("missing-directory", upper.join("BepInEx/plugins"))
                    .unwrap();
            } else {
                std::fs::create_dir_all(upper.join("BepInEx/plugins")).unwrap();
                match shadow_kind {
                    "dangling-file" => {
                        std::os::unix::fs::symlink("missing.dll", upper.join(name)).unwrap()
                    }
                    "whiteout" => make_test_file(&upper, "BepInEx/plugins/.wh.Test.dll", b""),
                    "opaque" => make_test_file(&upper, "BepInEx/.wh..wh..opq", b""),
                    _ => unreachable!(),
                }
            }
            let dest = root.path().join("out.zip");
            build_mod_zip_to_file(&dirs, &[test_file(1, name)], &dest, false).unwrap();
            let zip = zip::ZipArchive::new(File::open(dest).unwrap()).unwrap();
            assert!(
                zip.is_empty(),
                "{shadow_kind} must hide the stale base file"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn layered_zip_preserves_opaque_xattr_but_keeps_upper_files() {
        use std::os::unix::ffi::OsStrExt;
        for attr in [c"user.overlay.opaque", c"user.fuseoverlayfs.opaque"] {
            let root = tempfile::tempdir().unwrap();
            let dirs = QumaDirs::from_root(root.path().to_path_buf());
            let name = "BepInEx/plugins/Test.dll";
            make_test_file(&dirs.spt_server, name, b"stale base");
            let upper = dirs.runtime_upper();
            std::fs::create_dir_all(upper.join("BepInEx")).unwrap();
            let directory =
                std::ffi::CString::new(upper.join("BepInEx").as_os_str().as_bytes()).unwrap();
            // SAFETY: paths are NUL-terminated and the value points to one readable byte.
            let result = unsafe {
                libc::setxattr(
                    directory.as_ptr(),
                    attr.as_ptr(),
                    b"y".as_ptr().cast(),
                    1,
                    0,
                )
            };
            assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
            let dest = root.path().join("out.zip");
            let files = [test_file(1, name)];
            build_mod_zip_to_file(&dirs, &files, &dest, false).unwrap();
            assert!(zip::ZipArchive::new(File::open(&dest).unwrap())
                .unwrap()
                .is_empty());
            make_test_file(&upper, name, b"current upper");
            build_mod_zip_to_file(&dirs, &files, &dest, false).unwrap();
            assert_eq!(
                zip::ZipArchive::new(File::open(dest).unwrap())
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[test]
    fn cache_get_returns_none_when_no_file() {
        let spt_dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc,
            test_config(),
        );
        assert!(cache.get().is_none());
    }

    #[test]
    fn cache_get_returns_path_after_rebuild() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "SPT/user/mods/test/package.json", b"{}");

        let db = Database::open_in_memory().unwrap();
        // Insert a mod + file so the query returns something
        db.insert_mod(
            Some(1),
            Some(1),
            "test-mod",
            Some("test-mod"),
            "1.0.0",
            "forge",
            None,
            None,
        )
        .unwrap();
        db.insert_file(
            1,
            "SPT/user/mods/test/package.json",
            Some("abc123"),
            Some(2),
        )
        .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc,
            test_config(),
        );

        // Directly call rebuild (synchronous, for testing)
        cache.rebuild_sync();

        let path = cache.get();
        assert!(path.is_some());
        assert!(path.unwrap().exists());
    }

    #[test]
    fn rebuild_replaces_cache_with_updated_content() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(
            spt_dir.path(),
            "SPT/user/mods/a/package.json",
            b"{\"a\":true}",
        );

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(
            Some(1),
            Some(1),
            "mod-a",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
            None,
        )
        .unwrap();
        db.insert_file(1, "SPT/user/mods/a/package.json", Some("h1"), Some(10))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc.clone(),
            test_config(),
        );
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let size_before = std::fs::metadata(&path).unwrap().len();

        // Add a second mod file
        make_test_file(
            spt_dir.path(),
            "SPT/user/mods/b/package.json",
            b"{\"b\":true}",
        );
        {
            let db = db_arc.lock();
            db.insert_mod(
                Some(2),
                Some(2),
                "mod-b",
                Some("mod-b"),
                "1.0.0",
                "forge",
                None,
                None,
            )
            .unwrap();
            db.insert_file(2, "SPT/user/mods/b/package.json", Some("h2"), Some(10))
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
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc,
            test_config(),
        );

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
            test_file(1, "SPT/user/mods/test-mod/package.json"),
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
            test_file(1, "SPT/user/mods/test-mod/package.json"),
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
            include_patterns: vec!["SPT/user/mods/special/**".to_string()],
        };
        let files = vec![
            test_file(1, "SPT/user/mods/special/package.json"),
            test_file(1, "SPT/user/mods/other/package.json"),
        ];
        let filtered = filter_setup_zip_files(files, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, "SPT/user/mods/special/package.json");
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
            test_file(1, "SPT/user/mods/mod/package.json"),
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
        make_test_file(
            spt_dir.path(),
            "SPT/user/mods/server-mod/package.json",
            b"{}",
        );

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(
            Some(1),
            Some(1),
            "hybrid-mod",
            Some("hybrid-mod"),
            "1.0.0",
            "forge",
            None,
            None,
        )
        .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/mod.dll", Some("aaa"), Some(11))
            .unwrap();
        db.insert_file(
            1,
            "SPT/user/mods/server-mod/package.json",
            Some("bbb"),
            Some(2),
        )
        .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        // Default config has exclude_server_files=true
        let config = Arc::new(parking_lot::RwLock::new(crate::config::Config::default()));
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc,
            config,
        );
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let data = std::fs::read(&path).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();

        // Default config excludes server files
        assert_eq!(reader.len(), 1);
        let names = reader
            .file_names()
            .collect::<zip::result::ZipResult<Vec<_>>>()
            .unwrap();
        assert_eq!(names, vec!["BepInEx/plugins/mod/mod.dll"]);
    }

    #[test]
    fn cache_rebuild_excludes_non_essential() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "BepInEx/plugins/mod/mod.dll", b"content");
        make_test_file(spt_dir.path(), "BepInEx/plugins/mod/README.md", b"# Readme");

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(
            Some(1),
            Some(1),
            "test-mod",
            Some("test-mod"),
            "1.0.0",
            "forge",
            None,
            None,
        )
        .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/mod.dll", Some("aaa"), Some(7))
            .unwrap();
        db.insert_file(1, "BepInEx/plugins/mod/README.md", Some("bbb"), Some(8))
            .unwrap();

        let db_arc = Arc::new(Mutex::new(db));
        // Default config has exclude_non_essential=true
        let config = Arc::new(parking_lot::RwLock::new(crate::config::Config::default()));
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc,
            config,
        );
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let data = std::fs::read(&path).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();

        assert_eq!(reader.len(), 1);
        assert!(reader
            .file_names()
            .any(|n| n.unwrap() == "BepInEx/plugins/mod/mod.dll"));
    }

    #[test]
    fn cache_rebuild_with_all_filters_disabled() {
        let spt_dir = tempfile::tempdir().unwrap();
        make_test_file(spt_dir.path(), "SPT/user/mods/mod/package.json", b"{}");
        make_test_file(spt_dir.path(), "BepInEx/plugins/mod/README.md", b"# Hi");

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(
            Some(1),
            Some(1),
            "test-mod",
            Some("test-mod"),
            "1.0.0",
            "forge",
            None,
            None,
        )
        .unwrap();
        db.insert_file(1, "SPT/user/mods/mod/package.json", Some("aaa"), Some(2))
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
        let cache = ModZipCache::new(
            QumaDirs::from_legacy(spt_dir.path().to_path_buf()),
            db_arc,
            config,
        );
        cache.rebuild_sync();

        let path = cache.get().unwrap();
        let data = std::fs::read(&path).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();

        assert_eq!(reader.len(), 2);
    }
}
