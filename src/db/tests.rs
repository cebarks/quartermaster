use super::Database;

fn test_db() -> Database {
    Database::open_in_memory().expect("failed to open in-memory database")
}

#[test]
fn create_in_memory_db() {
    let db = test_db();
    // Verify all six tables exist by querying sqlite_master
    let tables: Vec<String> = {
        let mut stmt = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap()
    };
    assert!(tables.contains(&"installed_mods".to_string()));
    assert!(tables.contains(&"installed_files".to_string()));
    assert!(tables.contains(&"mod_dependencies".to_string()));
    assert!(tables.contains(&"users".to_string()));
    assert!(tables.contains(&"invite_codes".to_string()));
    assert!(tables.contains(&"pending_operations".to_string()));
}

#[test]
fn insert_and_get_mod() {
    let db = test_db();
    let id = db
        .insert_mod(1001, 2001, "Test Mod", "test-mod", "1.0.0")
        .unwrap();
    assert!(id > 0);

    let m = db.get_mod(id).unwrap().expect("mod should exist");
    assert_eq!(m.forge_mod_id, 1001);
    assert_eq!(m.forge_version_id, 2001);
    assert_eq!(m.name, "Test Mod");
    assert_eq!(m.slug, "test-mod");
    assert_eq!(m.version, "1.0.0");

    // Also test get_mod_by_forge_id
    let m2 = db
        .get_mod_by_forge_id(1001)
        .unwrap()
        .expect("mod should exist");
    assert_eq!(m2.id, id);

    // Test list_mods
    let mods = db.list_mods().unwrap();
    assert_eq!(mods.len(), 1);

    // Test update_mod
    let updated = db.update_mod(id, 2002, "1.1.0").unwrap();
    assert_eq!(updated, 1);
    let m3 = db.get_mod(id).unwrap().expect("mod should exist");
    assert_eq!(m3.forge_version_id, 2002);
    assert_eq!(m3.version, "1.1.0");
}

#[test]
fn duplicate_forge_mod_id_rejected() {
    let db = test_db();
    db.insert_mod(1001, 2001, "Mod A", "mod-a", "1.0.0")
        .unwrap();
    let result = db.insert_mod(1001, 2002, "Mod B", "mod-b", "2.0.0");
    assert!(result.is_err(), "duplicate forge_mod_id should be rejected");
}

#[test]
fn delete_mod_cascades_to_files_and_deps() {
    let db = test_db();
    let mod_a = db.insert_mod(1, 100, "Mod A", "mod-a", "1.0.0").unwrap();
    let mod_b = db.insert_mod(2, 200, "Mod B", "mod-b", "1.0.0").unwrap();

    // Add files and dependencies to mod_a
    db.insert_file(mod_a, "plugins/a.dll", "abc123", 1024)
        .unwrap();
    db.insert_file(mod_a, "plugins/a.json", "def456", 512)
        .unwrap();
    db.insert_dependency(mod_a, mod_b, Some(">=1.0.0")).unwrap();

    // Verify they exist
    assert_eq!(db.get_files_for_mod(mod_a).unwrap().len(), 2);
    assert_eq!(db.get_dependencies(mod_a).unwrap().len(), 1);

    // Delete mod_a -- files and deps should cascade
    db.delete_mod(mod_a).unwrap();
    assert_eq!(db.get_files_for_mod(mod_a).unwrap().len(), 0);
    assert_eq!(db.get_dependencies(mod_a).unwrap().len(), 0);

    // mod_b should still exist
    assert!(db.get_mod(mod_b).unwrap().is_some());
}

#[test]
fn insert_and_get_files() {
    let db = test_db();
    let mod_id = db.insert_mod(1, 100, "Mod A", "mod-a", "1.0.0").unwrap();

    let f1 = db
        .insert_file(mod_id, "plugins/a.dll", "hash1", 1024)
        .unwrap();
    let f2 = db
        .insert_file(mod_id, "plugins/a.json", "hash2", 256)
        .unwrap();
    assert!(f1 > 0);
    assert!(f2 > 0);

    let files = db.get_files_for_mod(mod_id).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].file_path, "plugins/a.dll");
    assert_eq!(files[1].file_path, "plugins/a.json");

    let all = db.get_all_tracked_files().unwrap();
    assert_eq!(all.len(), 2);

    // Test delete_files_for_mod
    let deleted = db.delete_files_for_mod(mod_id).unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(db.get_files_for_mod(mod_id).unwrap().len(), 0);
}

#[test]
fn file_path_unique_constraint() {
    let db = test_db();
    let mod_id = db.insert_mod(1, 100, "Mod A", "mod-a", "1.0.0").unwrap();

    db.insert_file(mod_id, "plugins/a.dll", "hash1", 1024)
        .unwrap();
    let result = db.insert_file(mod_id, "plugins/a.dll", "hash2", 2048);
    assert!(result.is_err(), "duplicate file_path should be rejected");
}

#[test]
fn insert_and_query_dependency() {
    let db = test_db();
    let mod_a = db.insert_mod(1, 100, "Mod A", "mod-a", "1.0.0").unwrap();
    let mod_b = db.insert_mod(2, 200, "Mod B", "mod-b", "1.0.0").unwrap();

    let dep_id = db.insert_dependency(mod_a, mod_b, Some(">=1.0.0")).unwrap();
    assert!(dep_id > 0);

    let deps = db.get_dependencies(mod_a).unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].depends_on_mod_id, mod_b);
    assert_eq!(deps[0].version_constraint.as_deref(), Some(">=1.0.0"));

    // Test delete_dependencies_for_mod
    let deleted = db.delete_dependencies_for_mod(mod_a).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(db.get_dependencies(mod_a).unwrap().len(), 0);
}

#[test]
fn reverse_dependencies() {
    let db = test_db();
    let mod_a = db.insert_mod(1, 100, "Mod A", "mod-a", "1.0.0").unwrap();
    let mod_b = db.insert_mod(2, 200, "Mod B", "mod-b", "1.0.0").unwrap();
    let mod_c = db.insert_mod(3, 300, "Mod C", "mod-c", "1.0.0").unwrap();

    // Both A and C depend on B
    db.insert_dependency(mod_a, mod_b, None).unwrap();
    db.insert_dependency(mod_c, mod_b, Some(">=2.0.0")).unwrap();

    let rev = db.get_reverse_dependencies(mod_b).unwrap();
    assert_eq!(rev.len(), 2);

    // Verify both mod_a and mod_c appear as dependents
    let dependent_ids: Vec<i64> = rev.iter().map(|d| d.mod_id).collect();
    assert!(dependent_ids.contains(&mod_a));
    assert!(dependent_ids.contains(&mod_c));
}

#[test]
fn insert_and_get_user() {
    let db = test_db();
    let id = db.insert_user("alice", "hashed_pw", "admin").unwrap();
    assert!(id > 0);

    let user = db
        .get_user_by_username("alice")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.username, "alice");
    assert_eq!(user.password_hash, "hashed_pw");
    assert_eq!(user.role, "admin");

    let missing = db.get_user_by_username("bob").unwrap();
    assert!(missing.is_none());

    let users = db.list_users().unwrap();
    assert_eq!(users.len(), 1);
}

#[test]
fn admin_exists_check() {
    let db = test_db();
    assert!(!db.admin_exists().unwrap());

    db.insert_user("player1", "pw", "player").unwrap();
    assert!(!db.admin_exists().unwrap());

    db.insert_user("admin1", "pw", "admin").unwrap();
    assert!(db.admin_exists().unwrap());
}

#[test]
fn create_and_use_invite() {
    let db = test_db();
    let admin_id = db.insert_user("admin", "pw", "admin").unwrap();

    // Create an invite with no expiry
    let invite_id = db.create_invite("INVITE-123", admin_id, None).unwrap();
    assert!(invite_id > 0);

    // Verify the invite exists and is unused
    let invite = db
        .get_invite("INVITE-123")
        .unwrap()
        .expect("invite should exist");
    assert_eq!(invite.code, "INVITE-123");
    assert!(invite.used_by.is_none());

    // Use the invite
    let new_user_id = db.insert_user("newbie", "pw", "player").unwrap();
    let used = db.use_invite("INVITE-123", new_user_id).unwrap();
    assert_eq!(used, 1);

    // Verify it's now used
    let invite = db
        .get_invite("INVITE-123")
        .unwrap()
        .expect("invite should exist");
    assert_eq!(invite.used_by, Some(new_user_id));
    assert!(invite.used_at.is_some());

    // Can't reuse it
    let another_user_id = db.insert_user("another", "pw", "player").unwrap();
    let reused = db.use_invite("INVITE-123", another_user_id).unwrap();
    assert_eq!(reused, 0, "already-used invite should not be reusable");
}

#[test]
fn expired_invite_rejected() {
    let db = test_db();
    let admin_id = db.insert_user("admin", "pw", "admin").unwrap();

    // Create an invite that expired in the past
    db.create_invite("EXPIRED-1", admin_id, Some("2020-01-01 00:00:00"))
        .unwrap();

    let user_id = db.insert_user("latecomer", "pw", "player").unwrap();
    let used = db.use_invite("EXPIRED-1", user_id).unwrap();
    assert_eq!(used, 0, "expired invite should be rejected");
}

#[test]
fn pending_operations_crud() {
    let db = test_db();
    let admin_id = db.insert_user("admin", "pw", "admin").unwrap();

    // Insert a pending op
    let op_id = db
        .insert_pending_op(
            "install",
            1001,
            2001,
            "Cool Mod",
            Some("{\"source\":\"web\"}"),
            Some(admin_id),
        )
        .unwrap();
    assert!(op_id > 0);

    // Insert another
    db.insert_pending_op("update", 1002, 2002, "Other Mod", None, None)
        .unwrap();

    // List them
    let ops = db.list_pending_ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].action, "install");
    assert_eq!(ops[0].mod_name, "Cool Mod");
    assert_eq!(ops[0].metadata.as_deref(), Some("{\"source\":\"web\"}"));
    assert_eq!(ops[0].queued_by, Some(admin_id));

    // Delete one
    let deleted = db.delete_pending_op(op_id).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(db.list_pending_ops().unwrap().len(), 1);

    // Clear all
    db.clear_pending_ops().unwrap();
    assert_eq!(db.list_pending_ops().unwrap().len(), 0);
}
