pub mod addons;
pub mod backups;
pub mod convoy;
pub mod headless_stats;
pub mod logs;
pub mod mods;
pub mod notes;
pub mod raids;
pub mod rbac;
pub mod requests;
pub mod schema;
pub mod users;

use std::path::Path;

use rusqlite::Connection;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// SQL-readonly reader: no migrations, RBAC, payload or sidecar creation.
    /// WAL requires BOTH existing WAL and SHM. Native SQLite reader-lock/SHM
    /// bookkeeping is allowed. The database namespace must remain stable while
    /// this connection is in use (ordinary SQLite concurrent writers are fine).
    /// Missing sidecars, rollback journals and aliased/unsafe namespaces refuse.
    pub fn open_read_only(path: &Path) -> rusqlite::Result<Self> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let invalid = || rusqlite::Error::InvalidPath(path.to_owned());
        let io_error = |e| rusqlite::Error::ToSqlConversionFailure(Box::new(e));
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let absolute = std::path::absolute(parent).map_err(io_error)?;
        let mut probe = std::path::PathBuf::new();
        for component in absolute.components() {
            probe.push(component);
            if std::fs::symlink_metadata(&probe)
                .map_err(io_error)?
                .file_type()
                .is_symlink()
            {
                return Err(invalid());
            }
        }
        let directory = std::fs::metadata(parent).map_err(io_error)?;
        // SAFETY: geteuid has no memory preconditions.
        if directory.uid() != unsafe { libc::geteuid() } || directory.mode() & 0o022 != 0 {
            return Err(invalid());
        }
        let open = |p: &Path| -> rusqlite::Result<std::fs::File> {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(p)
                .map_err(io_error)?;
            let m = f.metadata().map_err(io_error)?;
            // SAFETY: geteuid has no memory preconditions.
            if !m.is_file()
                || m.nlink() != 1
                || m.uid() != unsafe { libc::geteuid() }
                || m.mode() & 0o022 != 0
            {
                return Err(invalid());
            }
            Ok(f)
        };
        let mut main = open(path)?;
        let mut header = [0u8; 20];
        main.read_exact(&mut header).map_err(io_error)?;
        if &header[..16] != b"SQLite format 3\0" {
            return Err(invalid());
        }
        let sidecar = |suffix: &str| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            std::path::PathBuf::from(name)
        };
        let exists = |p: &Path| -> rusqlite::Result<bool> {
            match std::fs::symlink_metadata(p) {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(io_error(e)),
            }
        };
        if exists(&sidecar("-journal"))? {
            return Err(invalid());
        }
        let mut held = vec![(path.to_owned(), main)];
        if header[18] == 2 || header[19] == 2 {
            held.push((sidecar("-wal"), open(&sidecar("-wal"))?));
            held.push((sidecar("-shm"), open(&sidecar("-shm"))?));
        } else if exists(&sidecar("-wal"))? || exists(&sidecar("-shm"))? {
            return Err(invalid());
        }
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Force SQLite to join the actual WAL and establish native reader state
        // while all inspected namespace inodes are still held.
        conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))?;
        for (name, file) in held {
            let a = file.metadata().map_err(io_error)?;
            let b = std::fs::symlink_metadata(name).map_err(io_error)?;
            if a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(invalid());
            }
        }
        let current = std::fs::metadata(parent).map_err(io_error)?;
        if current.dev() != directory.dev() || current.ino() != directory.ino() {
            return Err(invalid());
        }
        Ok(Self { conn })
    }

    /// Open a file-backed database, configure pragmas, and run migrations.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    #[allow(dead_code)] // used by integration tests
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    pub fn begin_transaction(&self) -> rusqlite::Result<rusqlite::Transaction<'_>> {
        self.conn.unchecked_transaction()
    }

    #[allow(dead_code)] // used by integration tests
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn configure_and_migrate(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        schema::run_migrations(&conn)?;
        rbac::sync_builtin_role_permissions(&conn)?;

        Ok(Self { conn })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod exclusion_regressions {
    use super::Database;

    #[test]
    fn exclusion_omitted_group_member_is_ungrouped() {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_mod(None, None, "EternalCycle", None, "1", "local", None, None)
            .unwrap();
        db.save_groups_atomic(&[(
            "No Headless".into(),
            "no-headless".into(),
            "optional".into(),
            true,
            vec![id],
        )])
        .unwrap();
        assert!(crate::ops::is_excluded_from_headless(&db, id));
        db.save_groups_atomic(&[(
            "No Headless".into(),
            "no-headless".into(),
            "optional".into(),
            true,
            vec![],
        )])
        .unwrap();
        assert_eq!(db.get_mod(id).unwrap().unwrap().group_id, None);
        assert!(!crate::ops::is_excluded_from_headless(&db, id));
    }

    #[test]
    fn wal_crash_fixture_child() {
        if let Some(path) = std::env::var_os("QUMA_TEST_WAL_CRASH_PATH") {
            let db = Database::open(std::path::Path::new(&path)).unwrap();
            db.conn()
                .execute("CREATE TABLE crash_probe(value INTEGER)", [])
                .unwrap();
            db.conn()
                .execute("INSERT INTO crash_probe VALUES (19)", [])
                .unwrap();
            std::process::exit(0); // deliberately leave real committed WAL/SHM
        }
    }

    #[test]
    fn read_only_crashed_wal_and_missing_sidecar_states() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("crash.db");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "db::exclusion_regressions::wal_crash_fixture_child",
            ])
            .env("QUMA_TEST_WAL_CRASH_PATH", &path)
            .output()
            .unwrap();
        assert!(child.status.success());
        let before = std::fs::read(&path).unwrap();
        let wal = tmp.path().join("crash.db-wal");
        let wal_before = std::fs::read(&wal).unwrap();
        let reader = Database::open_read_only(&path).unwrap();
        assert_eq!(
            reader
                .conn()
                .query_row("SELECT value FROM crash_probe", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            19
        );
        drop(reader);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read(&wal).unwrap(), wal_before);
        std::fs::remove_file(tmp.path().join("crash.db-shm")).unwrap();
        assert!(Database::open_read_only(&path).is_err());
        assert!(!tmp.path().join("crash.db-shm").exists());
        std::fs::remove_file(wal).unwrap();
        assert!(Database::open_read_only(&path).is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn read_only_refuses_missing_wal_sidecars_before_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("wal.db");
        let writer = Database::open(&path).unwrap();
        writer
            .conn()
            .execute("CREATE TABLE wal_read_probe(value INTEGER)", [])
            .unwrap();
        writer
            .conn()
            .execute("INSERT INTO wal_read_probe VALUES (7)", [])
            .unwrap();
        let shm = path.with_file_name("wal.db-shm");
        // Disposable fixture only: keep the writer's inode alive but remove its name.
        std::fs::remove_file(&shm).unwrap();
        assert!(Database::open_read_only(&path).is_err());
        assert!(!shm.exists());
    }

    #[test]
    fn read_only_live_wal_sees_commits_and_never_creates_sidecars() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("wal.db");
        let writer = Database::open(&path).unwrap();
        writer
            .conn()
            .execute("CREATE TABLE wal_read_probe(value INTEGER)", [])
            .unwrap();
        writer
            .conn()
            .execute("INSERT INTO wal_read_probe VALUES (7)", [])
            .unwrap();
        let before: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let reader = Database::open_read_only(&path).unwrap();
        let tx = reader.begin_transaction().unwrap();
        assert_eq!(
            reader
                .conn()
                .query_row("SELECT SUM(value) FROM wal_read_probe", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            7
        );
        writer
            .conn()
            .execute("INSERT INTO wal_read_probe VALUES (8)", [])
            .unwrap();
        assert_eq!(
            reader
                .conn()
                .query_row("SELECT SUM(value) FROM wal_read_probe", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            7
        );
        tx.commit().unwrap();
        assert_eq!(
            reader
                .conn()
                .query_row("SELECT SUM(value) FROM wal_read_probe", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            15
        );
        let after: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn exclusion_read_only_open_does_not_migrate_or_sync_rbac() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES ('untouched'); PRAGMA user_version=1;").unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        let db = Database::open_read_only(&path).unwrap();
        let version: i64 = db
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        assert!(db
            .conn()
            .execute("INSERT INTO sentinel VALUES ('write')", [])
            .is_err());
        drop(db);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
