use rusqlite::Connection;

/// Ordered list of migration SQL scripts. Each entry corresponds to a
/// `user_version` bump: index 0 → version 1, index 1 → version 2, etc.
/// To add a new migration, append a new `include_str!` entry here.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/001_initial.sql"),
    include_str!("../../migrations/002_pending_updates.sql"),
    include_str!("../../migrations/003_log_entries.sql"),
    include_str!("../../migrations/004_invite_codes_cascade.sql"),
    include_str!("../../migrations/005_addon_support.sql"),
    include_str!("../../migrations/006_constraints.sql"),
    include_str!("../../migrations/007_log_indexes.sql"),
    include_str!("../../migrations/008_headless_users.sql"),
    include_str!("../../migrations/009_headless_session_stats.sql"),
    include_str!("../../migrations/010_url_install_support.sql"),
    include_str!("../../migrations/011_items_give_permission.sql"),
    include_str!("../../migrations/012_notes.sql"),
    include_str!("../../migrations/013_request_lifecycle.sql"),
    include_str!("../../migrations/014_config_edit_permission.sql"),
    include_str!("../../migrations/015_convoy_groups.sql"),
    include_str!("../../migrations/016_convoy_permission.sql"),
    include_str!("../../migrations/017_convoy_sync_tracking.sql"),
    include_str!("../../migrations/018_dependency_tree.sql"),
    include_str!("../../migrations/019_queue_unique_constraints.sql"),
    include_str!("../../migrations/020_multi_use_invites.sql"),
    include_str!("../../migrations/021_mod_guid.sql"),
    include_str!("../../migrations/022_update_notifications.sql"),
    include_str!("../../migrations/023_rebaseline_legacy_hashes.sql"),
];

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target_version = (i + 1) as i32;
        if current_version < target_version {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", target_version)?;
            tx.commit()?;
        }
    }

    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::Database;

    pub(crate) fn database_at_version(version: usize) -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (index, sql) in MIGRATIONS[..version].iter().enumerate() {
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute_batch(sql).unwrap();
            tx.pragma_update(None, "user_version", (index + 1) as i32)
                .unwrap();
            tx.commit().unwrap();
        }
        Database { conn }
    }

    fn seed_legacy_hashes(db: &Database) {
        db.conn()
            .execute_batch(
                "INSERT INTO installed_mods (id, forge_mod_id, forge_version_id, name, version)
                 VALUES (1, 7001, 1, 'Hash upgrade', '1.0');
                 INSERT INTO installed_addons (id, forge_addon_id, parent_mod_id, forge_version_id, name, version)
                 VALUES (1, 8001, 1, 1, 'Addon', '1.0');",
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO installed_files (id, mod_id, addon_id, file_path, file_hash) VALUES
                 (1, 1, NULL, 'server-old', ?1),
                 (2, NULL, 1, 'addon-old', ?2),
                 (3, 1, NULL, 'mod-current', '9555e8555c62dcfd'),
                 (4, NULL, 1, 'addon-current', 'b3f41db614dd77fa'),
                 (5, 1, NULL, 'mod-unknown', NULL),
                 (6, NULL, 1, 'addon-unknown', NULL)",
                rusqlite::params!["a".repeat(64), "b".repeat(64)],
            )
            .unwrap();
    }

    fn pending_ids(db: &Database) -> Vec<i64> {
        db.conn()
            .prepare("SELECT file_id FROM pending_file_rehashes ORDER BY file_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn legacy_hash_upgrade_queues_only_legacy_files() {
        for version in [20, 21, 22] {
            let db = database_at_version(version);
            seed_legacy_hashes(&db);
            run_migrations(db.conn()).unwrap();
            assert_eq!(pending_ids(&db), vec![1, 2], "upgrading version {version}");
            let hashes: Vec<Option<String>> = db
                .conn()
                .prepare("SELECT file_hash FROM installed_files ORDER BY id")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                hashes,
                vec![
                    None,
                    None,
                    Some("9555e8555c62dcfd".into()),
                    Some("b3f41db614dd77fa".into()),
                    None,
                    None
                ]
            );
            db.mark_update_notified(1, "2.0").unwrap();
            db.conn()
                .execute_batch(
                    "UPDATE installed_files SET file_hash = '94c7978acb932996' WHERE id = 2;
                     DELETE FROM pending_file_rehashes WHERE file_id = 2;",
                )
                .unwrap();
            run_migrations(db.conn()).unwrap();
            assert_eq!(pending_ids(&db), vec![1]);
            assert!(db.was_update_notified(1, "2.0").unwrap());
            db.conn()
                .execute("DELETE FROM installed_files WHERE id = 1", [])
                .unwrap();
            assert!(pending_ids(&db).is_empty());
        }
    }

    #[test]
    fn legacy_hash_pending_work_survives_database_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("quartermaster.db");
        let old_db = database_at_version(21);
        seed_legacy_hashes(&old_db);
        old_db
            .conn()
            .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
        {
            let db = Database::open(&path).unwrap();
            assert_eq!(pending_ids(&db), vec![1, 2]);
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(pending_ids(&db), vec![1, 2]);
        db.delete_mod(1).unwrap();
        assert!(pending_ids(&db).is_empty());
    }

    #[test]
    fn registered_migrations_cover_directory() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut scripts: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
            .collect();
        scripts.sort();
        assert_eq!(
            scripts.len(),
            MIGRATIONS.len(),
            "every migration must be registered"
        );
        for (index, path) in scripts.iter().enumerate() {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(name[..3].parse::<usize>().unwrap(), index + 1, "{name}");
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                MIGRATIONS[index],
                "{name}"
            );
        }
    }

    #[test]
    fn notification_upgrades_preserve_guid_and_state() {
        for version in [20, 21] {
            let db = database_at_version(version);
            db.conn()
                .execute(
                    "INSERT INTO installed_mods (id, forge_mod_id, forge_version_id, name, version)
                     VALUES (1, 7001, 1, 'Upgrade', '1.0')",
                    [],
                )
                .unwrap();
            if version == 21 {
                db.conn()
                    .execute(
                        "UPDATE installed_mods SET guid = 'preserved-guid' WHERE id = 1",
                        [],
                    )
                    .unwrap();
            }

            run_migrations(db.conn()).unwrap();
            let guid: Option<String> = db
                .conn()
                .query_row("SELECT guid FROM installed_mods WHERE id = 1", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                guid.as_deref(),
                if version == 21 {
                    Some("preserved-guid")
                } else {
                    None
                },
                "GUID state after upgrading version {version}"
            );
            assert!(!db.was_update_notified(1, "2.0").unwrap());
            assert_eq!(db.mark_update_notified(1, "2.0").unwrap(), 1);

            run_migrations(db.conn()).unwrap();
            assert!(db.was_update_notified(1, "2.0").unwrap());
            let guid_columns: i64 = db
                .conn()
                .query_row(
                    "SELECT count(*) FROM pragma_table_info('installed_mods') WHERE name = 'guid'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(guid_columns, 1);
        }
    }
}
