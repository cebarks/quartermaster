use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::spt::scoped::{relative_path, ScopedDir};

/// Trust current bytes for queued legacy hashes; unavailable files remain pending for retry.
pub(crate) fn rebaseline_pending_hashes(db: &Database, dirs: &QumaDirs) -> Result<usize> {
    let mut rebaselined = 0;
    for (file, disabled) in db.pending_file_rehashes()? {
        let recovered = (|| -> Result<_> {
            let relative = relative_path(&file.file_path)?;
            let source = open_visible_file(dirs, &relative, disabled)?;
            let size = i64::try_from(source.metadata()?.len())
                .context("file size exceeds SQLite range")?;
            let hash = crate::spt::mods::compute_reader_hash(source)?;
            Ok((hash, size))
        })();
        match recovered {
            Ok((hash, size)) => {
                rebaselined += db
                    .complete_file_rehash(file.id, &hash, Some(size))
                    .context("failed to complete pending file rehash")?;
            }
            Err(error) => {
                tracing::warn!(
                    file_id = file.id,
                    path = %file.file_path,
                    err = %error,
                    "legacy file hash remains pending"
                );
            }
        }
    }
    if rebaselined > 0 {
        tracing::info!(
            files = rebaselined,
            "rebaselined legacy file hashes from current files"
        );
    }
    Ok(rebaselined)
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn open_visible_file(dirs: &QumaDirs, relative: &Path, disabled: bool) -> Result<File> {
    let roots = if disabled {
        vec![dirs.disabled_dir()]
    } else if dirs.is_legacy() {
        vec![dirs.spt_server.clone()]
    } else {
        vec![
            dirs.runtime_upper(),
            dirs.mod_overlay(),
            dirs.spt_server.clone(),
        ]
    };
    for root in roots {
        let layer = match ScopedDir::open(&root) {
            Ok(layer) => layer,
            Err(error) if is_not_found(&error) => continue,
            Err(error) => return Err(error),
        };
        match layer.read_overlay_file(relative) {
            Ok(Some(file)) => return Ok(file),
            Ok(None) => bail!("tracked file is hidden by an overlay whiteout"),
            Err(error) if is_not_found(&error) => {
                if layer.blocks_lower_layer(relative)? {
                    bail!("tracked file is hidden by an upper layer");
                }
            }
            Err(error) => return Err(error),
        }
    }
    bail!("tracked file is unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::mods::InstalledFile;
    use crate::db::Database;
    use crate::dirs::QumaDirs;
    use std::path::{Path, PathBuf};

    struct Fixture {
        _temp: tempfile::TempDir,
        dirs: QumaDirs,
        db: Database,
        mod_id: i64,
        addon_id: i64,
    }

    impl Fixture {
        fn new(legacy: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let dirs = if legacy {
                QumaDirs::from_legacy(temp.path().to_path_buf())
            } else {
                QumaDirs::from_root(temp.path().to_path_buf())
            };
            std::fs::create_dir_all(&dirs.spt_server).unwrap();
            let db = Database::open(&dirs.db_path()).unwrap();
            let mod_id = db
                .insert_mod(
                    Some(7001),
                    Some(1),
                    "Recovery",
                    None,
                    "1.0",
                    "forge",
                    None,
                    None,
                )
                .unwrap();
            let addon_id = db
                .insert_addon(8001, mod_id, 1, "Addon", None, "1.0", None)
                .unwrap();
            Self {
                _temp: temp,
                dirs,
                db,
                mod_id,
                addon_id,
            }
        }

        fn pending(&self, path: &str, addon: bool) -> i64 {
            let id = if addon {
                self.db
                    .insert_addon_file(self.addon_id, path, None, None)
                    .unwrap()
            } else {
                self.db.insert_file(self.mod_id, path, None, None).unwrap()
            };
            self.db
                .conn()
                .execute(
                    "INSERT INTO pending_file_rehashes (file_id) VALUES (?1)",
                    [id],
                )
                .unwrap();
            id
        }

        fn file(&self, id: i64) -> InstalledFile {
            self.db
                .get_all_tracked_files()
                .unwrap()
                .into_iter()
                .find(|file| file.id == id)
                .unwrap()
        }

        fn remaining(&self) -> i64 {
            self.db
                .conn()
                .query_row("SELECT count(*) FROM pending_file_rehashes", [], |row| {
                    row.get(0)
                })
                .unwrap()
        }
    }

    fn write(root: &Path, relative: &str, content: &[u8]) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn rebaselines_old_mod_and_addon_files_once_without_touching_other_hashes() {
        for legacy in [false, true] {
            let fixture = Fixture::new(legacy);
            let server = if legacy {
                "SPT/user/mods/Test/config.json"
            } else {
                "SPT_Runtime/user/mods/Test/config.json"
            };
            let mod_file = fixture.pending(server, false);
            let addon_file = fixture.pending("BepInEx/plugins/addon.dll", true);
            let path = write(&fixture.dirs.spt_server, server, b"hello");
            let addon_path = write(
                &fixture.dirs.spt_server,
                "BepInEx/plugins/addon.dll",
                b"legacy addon",
            );
            let times = std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1));
            std::fs::File::open(path).unwrap().set_times(times).unwrap();
            std::fs::File::open(addon_path)
                .unwrap()
                .set_times(times)
                .unwrap();
            let current = fixture
                .db
                .insert_file(
                    fixture.mod_id,
                    "BepInEx/plugins/current.dll",
                    Some("1111111111111111"),
                    Some(99),
                )
                .unwrap();
            let unknown = fixture
                .db
                .insert_addon_file(fixture.addon_id, "BepInEx/plugins/unknown.dll", None, None)
                .unwrap();
            write(
                &fixture.dirs.spt_server,
                "BepInEx/plugins/current.dll",
                b"changed",
            );
            write(
                &fixture.dirs.spt_server,
                "BepInEx/plugins/unknown.dll",
                b"hello",
            );

            assert_eq!(
                rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
                2
            );
            assert_eq!(
                fixture.file(mod_file).file_hash.as_deref(),
                Some("9555e8555c62dcfd")
            );
            assert_eq!(fixture.file(mod_file).file_size, Some(5));
            assert_eq!(
                fixture.file(addon_file).file_hash.as_deref(),
                Some("94c7978acb932996")
            );
            assert_eq!(fixture.file(addon_file).file_size, Some(12));
            assert_eq!(
                fixture.file(current).file_hash.as_deref(),
                Some("1111111111111111")
            );
            assert_eq!(fixture.file(current).file_size, Some(99));
            assert!(fixture.file(unknown).file_hash.is_none());
            assert_eq!(fixture.remaining(), 0);
            write(
                &fixture.dirs.spt_server,
                server,
                b"changed after rebaseline",
            );
            assert_eq!(
                rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
                0
            );
            assert_eq!(
                fixture.file(mod_file).file_hash.as_deref(),
                Some("9555e8555c62dcfd")
            );
        }
    }

    #[test]
    fn honors_runtime_then_mod_then_base_visibility() {
        let fixture = Fixture::new(false);
        for (path, layer) in [
            ("BepInEx/plugins/runtime.dll", fixture.dirs.runtime_upper()),
            ("BepInEx/plugins/mod.dll", fixture.dirs.mod_overlay()),
            ("BepInEx/plugins/base.dll", fixture.dirs.spt_server.clone()),
        ] {
            let id = fixture.pending(path, false);
            write(&fixture.dirs.spt_server, path, b"lower base");
            if layer != fixture.dirs.spt_server {
                write(&fixture.dirs.mod_overlay(), path, b"lower mod");
            }
            write(&layer, path, b"hello");
            assert_eq!(
                rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
                1
            );
            assert_eq!(
                fixture.file(id).file_hash.as_deref(),
                Some("9555e8555c62dcfd")
            );
        }
    }

    #[test]
    fn reads_disabled_mod_addon_and_disabled_parent_from_stash() {
        for case in 0..3 {
            let fixture = Fixture::new(false);
            let path = "BepInEx/plugins/disabled.dll";
            let id = fixture.pending(path, case != 0);
            if case == 1 {
                fixture
                    .db
                    .set_addon_disabled(fixture.addon_id, true)
                    .unwrap();
            } else {
                fixture.db.set_mod_disabled(fixture.mod_id, true).unwrap();
            }
            write(&fixture.dirs.spt_server, path, b"wrong enabled copy");
            write(&fixture.dirs.disabled_dir(), path, b"hello");
            assert_eq!(
                rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
                1,
                "case {case}"
            );
            assert_eq!(
                fixture.file(id).file_hash.as_deref(),
                Some("9555e8555c62dcfd")
            );
        }
    }

    #[test]
    fn missing_files_retry_after_database_reopen() {
        let mut fixture = Fixture::new(false);
        let path = "BepInEx/plugins/restored.dll";
        let id = fixture.pending(path, true);
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            0
        );
        assert!(fixture.file(id).file_hash.is_none());
        assert_eq!(fixture.remaining(), 1);
        drop(fixture.db);
        fixture.db = Database::open(&fixture.dirs.db_path()).unwrap();
        write(&fixture.dirs.spt_server, path, b"hello");
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            1
        );
        assert_eq!(fixture.remaining(), 0);
        assert_eq!(
            fixture.file(id).file_hash.as_deref(),
            Some("9555e8555c62dcfd")
        );
    }

    #[test]
    fn rejects_escaping_paths_and_links_but_preserves_confined_links() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new(false);
        let outside = tempfile::tempdir().unwrap();
        let outside_file = write(outside.path(), "outside.dll", b"hello");
        for path in [
            "../outside.dll",
            "..\\outside.dll",
            outside_file.to_str().unwrap(),
            "escaping.dll",
        ] {
            fixture.pending(path, false);
        }
        symlink(&outside_file, fixture.dirs.spt_server.join("escaping.dll")).unwrap();
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            0
        );
        assert_eq!(fixture.remaining(), 4);
        assert!(fixture
            .db
            .get_all_tracked_files()
            .unwrap()
            .iter()
            .all(|file| file.file_hash.is_none()));
        write(&fixture.dirs.spt_server, "contained.dll", b"hello");
        symlink(
            fixture.dirs.spt_server.join("contained.dll"),
            fixture.dirs.spt_server.join("alias.dll"),
        )
        .unwrap();
        let alias = fixture.pending("alias.dll", false);
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            1
        );
        assert_eq!(
            fixture.file(alias).file_hash.as_deref(),
            Some("9555e8555c62dcfd")
        );
        assert_eq!(fixture.remaining(), 4);
    }

    #[test]
    fn hidden_and_shadowed_files_never_fall_back_to_lower_layers() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new(false);
        for path in [
            "plugins/hidden.dll",
            "opaque/hidden.dll",
            "shadow/hidden.dll",
            "dangling.dll",
        ] {
            fixture.pending(path, false);
            write(&fixture.dirs.spt_server, path, b"hello");
        }
        write(&fixture.dirs.runtime_upper(), "plugins/.wh.hidden.dll", b"");
        write(&fixture.dirs.runtime_upper(), "opaque/.wh..wh..opq", b"");
        write(&fixture.dirs.runtime_upper(), "shadow", b"not a directory");
        symlink(
            "missing-target",
            fixture.dirs.runtime_upper().join("dangling.dll"),
        )
        .unwrap();
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            0
        );
        assert_eq!(fixture.remaining(), 4);
        assert!(fixture
            .db
            .get_all_tracked_files()
            .unwrap()
            .iter()
            .all(|file| file.file_hash.is_none()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_empty_whiteout_is_terminal_but_restored_data_is_visible() {
        use std::os::fd::AsRawFd;
        let fixture = Fixture::new(false);
        let path = "BepInEx/plugins/native.dll";
        let id = fixture.pending(path, false);
        write(&fixture.dirs.spt_server, path, b"lower");
        let upper = write(&fixture.dirs.runtime_upper(), path, b"");
        let file = std::fs::File::open(&upper).unwrap();
        // SAFETY: the descriptor and C attribute name are live; the value is one byte.
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    c"user.overlay.whiteout".as_ptr(),
                    b"y".as_ptr().cast(),
                    1,
                    0,
                )
            },
            0
        );
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            0
        );
        assert_eq!(fixture.remaining(), 1);
        std::fs::write(upper, b"hello").unwrap();
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            1
        );
        assert_eq!(
            fixture.file(id).file_hash.as_deref(),
            Some("9555e8555c62dcfd")
        );
    }

    #[test]
    fn unreadable_files_remain_pending_without_blocking_other_files() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new(false);
        let blocked = fixture.pending("blocked.dll", false);
        let readable = fixture.pending("readable.dll", false);
        let path = write(&fixture.dirs.spt_server, "blocked.dll", b"hello");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0)).unwrap();
        write(&fixture.dirs.spt_server, "readable.dll", b"hello");
        assert_eq!(
            rebaseline_pending_hashes(&fixture.db, &fixture.dirs).unwrap(),
            1
        );
        assert!(fixture.file(blocked).file_hash.is_none());
        assert_eq!(
            fixture.file(readable).file_hash.as_deref(),
            Some("9555e8555c62dcfd")
        );
        assert_eq!(fixture.remaining(), 1);
    }

    #[test]
    fn completion_preserves_a_newer_hash() {
        let fixture = Fixture::new(false);
        let id = fixture.pending("changed.dll", false);
        fixture
            .db
            .update_file_hash(id, "1111111111111111", Some(99))
            .unwrap();
        assert_eq!(
            fixture
                .db
                .complete_file_rehash(id, "9555e8555c62dcfd", Some(5))
                .unwrap(),
            0
        );
        assert_eq!(
            fixture.file(id).file_hash.as_deref(),
            Some("1111111111111111")
        );
        assert_eq!(fixture.file(id).file_size, Some(99));
        assert_eq!(fixture.remaining(), 0);
    }

    #[test]
    fn completion_rolls_back_hash_if_pending_removal_fails() {
        let fixture = Fixture::new(false);
        let id = fixture.pending("retry.dll", false);
        fixture
            .db
            .conn()
            .execute_batch(
                "CREATE TRIGGER refuse_completion BEFORE DELETE ON pending_file_rehashes
             BEGIN SELECT RAISE(ABORT, 'simulated interruption'); END;",
            )
            .unwrap();
        assert!(fixture
            .db
            .complete_file_rehash(id, "9555e8555c62dcfd", Some(5))
            .is_err());
        assert!(fixture.file(id).file_hash.is_none());
        assert_eq!(fixture.remaining(), 1);
    }

    #[test]
    fn an_empty_queue_requires_no_installation_directories() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = QumaDirs::from_root(temp.path().join("absent-installation"));
        let db = Database::open_in_memory().unwrap();
        assert_eq!(rebaseline_pending_hashes(&db, &dirs).unwrap(), 0);
        assert!(!dirs.root.exists());
    }
}
