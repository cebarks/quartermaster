use spt_quartermaster::spt::mods::compute_file_hash;

#[test]
fn tracked_file_hash_is_canonical_xxh3_128() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"abc").unwrap();
    assert_eq!(
        compute_file_hash(file.path()).unwrap(),
        "06b05ab6733a618578af5f94892f3950"
    );
}

#[test]
fn legacy_hash_migration_preserves_baselines_and_unknowns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mut migrations: Vec<_> =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
            .unwrap()
            .map(|item| item.unwrap().path())
            .collect();
    migrations.sort();
    for (index, migration) in migrations.iter().take(24).enumerate() {
        conn.execute_batch(&std::fs::read_to_string(migration).unwrap())
            .unwrap();
        conn.pragma_update(None, "user_version", (index + 1) as i32)
            .unwrap();
    }
    conn.execute_batch(
        "INSERT INTO installed_mods (id, name, version) VALUES (1, 'fixture', '1');
        INSERT INTO installed_files (id, mod_id, file_path, file_hash) VALUES
        (1, 1, 'BepInEx/plugins/old.dll', '78af5f94892f3950'),
        (2, 1, 'BepInEx/plugins/unknown.dll', NULL);",
    )
    .unwrap();
    drop(conn);
    let db = spt_quartermaster::db::Database::open(&path).unwrap();
    let (hash, algorithm): (String, String) = db
        .conn()
        .query_row(
            "SELECT hash, algorithm FROM legacy_file_hashes WHERE file_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(hash, "78af5f94892f3950");
    assert_eq!(algorithm, "xxh3_64");
    let files = db.get_files_for_mod(1).unwrap();
    assert_eq!(
        files
            .iter()
            .find(|f| f.id == 1)
            .unwrap()
            .file_hash
            .as_deref(),
        Some("78af5f94892f3950")
    );
    assert!(files
        .iter()
        .find(|f| f.id == 2)
        .unwrap()
        .file_hash
        .is_none());
    let pending: Vec<i64> = db
        .conn()
        .prepare("SELECT file_id FROM pending_file_rehashes ORDER BY file_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        pending,
        vec![1],
        "only the legacy baseline is pending; unknown must remain unknown"
    );
}

#[test]
fn pre_023_sha_baseline_is_not_discarded_during_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mut migrations: Vec<_> =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
            .unwrap()
            .map(|item| item.unwrap().path())
            .collect();
    migrations.sort();
    for (index, migration) in migrations.iter().take(22).enumerate() {
        conn.execute_batch(&std::fs::read_to_string(migration).unwrap())
            .unwrap();
        conn.pragma_update(None, "user_version", (index + 1) as i32)
            .unwrap();
    }
    conn.execute_batch("INSERT INTO installed_mods (id, name, version) VALUES (1, 'fixture', '1');
        INSERT INTO installed_files (id, mod_id, file_path, file_hash) VALUES
        (1, 1, 'BepInEx/plugins/old.dll', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');").unwrap();
    drop(conn);
    let db = spt_quartermaster::db::Database::open(&path).unwrap();
    let (hash, algorithm): (Option<String>, String) = db
        .conn()
        .query_row(
            "SELECT hash, algorithm FROM legacy_file_hashes WHERE file_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        hash.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(algorithm, "sha256");
}
