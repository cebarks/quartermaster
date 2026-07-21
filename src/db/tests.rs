use super::{requests::RequestStatus, Database};

fn test_db() -> Database {
    Database::open_in_memory().expect("failed to open in-memory database")
}

#[test]
fn create_in_memory_db() {
    let db = test_db();
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
    assert!(tables.contains(&"mod_requests".to_string()));
    assert!(tables.contains(&"mod_request_votes".to_string()));
    assert!(tables.contains(&"raids".to_string()));
    assert!(tables.contains(&"raid_kills".to_string()));
}

#[test]
fn insert_and_get_mod() {
    let db = test_db();
    let id = db
        .insert_mod(
            Some(1001),
            Some(2001),
            "Test Mod",
            Some("test-mod"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    assert!(id > 0);

    let m = db.get_mod(id).unwrap().expect("mod should exist");
    assert_eq!(m.forge_mod_id, Some(1001));
    assert_eq!(m.forge_version_id, Some(2001));
    assert_eq!(m.name, "Test Mod");
    assert_eq!(m.slug.as_deref(), Some("test-mod"));
    assert_eq!(m.version, "1.0.0");
    assert!(m.updated_at.is_none());

    let m2 = db
        .get_mod_by_forge_id(1001)
        .unwrap()
        .expect("mod should exist");
    assert_eq!(m2.id, id);

    let mods = db.list_mods().unwrap();
    assert_eq!(mods.len(), 1);

    let updated = db.update_mod(id, 2002, "1.1.0").unwrap();
    assert_eq!(updated, 1);
    let m3 = db.get_mod(id).unwrap().expect("mod should exist");
    assert_eq!(m3.forge_version_id, Some(2002));
    assert_eq!(m3.version, "1.1.0");
    assert!(m3.updated_at.is_some());
}

#[test]
fn insert_mod_with_no_slug() {
    let db = test_db();
    let id = db
        .insert_mod(
            Some(1001),
            Some(2001),
            "No Slug Mod",
            None,
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    let m = db.get_mod(id).unwrap().expect("mod should exist");
    assert!(m.slug.is_none());
}

#[test]
fn duplicate_forge_mod_id_rejected() {
    let db = test_db();
    db.insert_mod(
        Some(1001),
        Some(2001),
        "Mod A",
        Some("mod-a"),
        "1.0.0",
        "forge",
        None,
    )
    .unwrap();
    let result = db.insert_mod(
        Some(1001),
        Some(2002),
        "Mod B",
        Some("mod-b"),
        "2.0.0",
        "forge",
        None,
    );
    assert!(result.is_err(), "duplicate forge_mod_id should be rejected");
}

#[test]
fn delete_mod_cascades_to_files_and_deps() {
    let db = test_db();
    let mod_a = db
        .insert_mod(
            Some(1),
            Some(100),
            "Mod A",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    let mod_b = db
        .insert_mod(
            Some(2),
            Some(200),
            "Mod B",
            Some("mod-b"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

    db.insert_file(mod_a, "plugins/a.dll", Some("abc123"), Some(1024))
        .unwrap();
    db.insert_file(mod_a, "plugins/a.json", Some("def456"), Some(512))
        .unwrap();
    db.insert_dependency(mod_a, mod_b, Some(">=1.0.0")).unwrap();

    assert_eq!(db.get_files_for_mod(mod_a).unwrap().len(), 2);
    assert_eq!(db.get_dependencies(mod_a).unwrap().len(), 1);

    db.delete_mod(mod_a).unwrap();
    assert_eq!(db.get_files_for_mod(mod_a).unwrap().len(), 0);
    assert_eq!(db.get_dependencies(mod_a).unwrap().len(), 0);

    assert!(db.get_mod(mod_b).unwrap().is_some());
}

#[test]
fn insert_and_get_files() {
    let db = test_db();
    let mod_id = db
        .insert_mod(
            Some(1),
            Some(100),
            "Mod A",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

    let f1 = db
        .insert_file(mod_id, "plugins/a.dll", Some("hash1"), Some(1024))
        .unwrap();
    let f2 = db
        .insert_file(mod_id, "plugins/a.json", Some("hash2"), Some(256))
        .unwrap();
    assert!(f1 > 0);
    assert!(f2 > 0);

    let files = db.get_files_for_mod(mod_id).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].file_path, "plugins/a.dll");
    assert_eq!(files[1].file_path, "plugins/a.json");

    let all = db.get_all_tracked_files().unwrap();
    assert_eq!(all.len(), 2);

    let deleted = db.delete_files_for_mod(mod_id).unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(db.get_files_for_mod(mod_id).unwrap().len(), 0);
}

#[test]
fn insert_file_with_no_hash() {
    let db = test_db();
    let mod_id = db
        .insert_mod(
            Some(1),
            Some(100),
            "Mod A",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    db.insert_file(mod_id, "plugins/a.dll", None, None).unwrap();
    let files = db.get_files_for_mod(mod_id).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].file_hash.is_none());
    assert!(files[0].file_size.is_none());
}

#[test]
fn file_path_unique_constraint() {
    let db = test_db();
    let mod_id = db
        .insert_mod(
            Some(1),
            Some(100),
            "Mod A",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

    db.insert_file(mod_id, "plugins/a.dll", Some("hash1"), Some(1024))
        .unwrap();
    let result = db.insert_file(mod_id, "plugins/a.dll", Some("hash2"), Some(2048));
    assert!(result.is_err(), "duplicate file_path should be rejected");
}

#[test]
fn insert_and_query_dependency() {
    let db = test_db();
    let mod_a = db
        .insert_mod(
            Some(1),
            Some(100),
            "Mod A",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    let mod_b = db
        .insert_mod(
            Some(2),
            Some(200),
            "Mod B",
            Some("mod-b"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

    let dep_id = db.insert_dependency(mod_a, mod_b, Some(">=1.0.0")).unwrap();
    assert!(dep_id > 0);

    let deps = db.get_dependencies(mod_a).unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].depends_on_mod_id, mod_b);
    assert_eq!(deps[0].version_constraint.as_deref(), Some(">=1.0.0"));

    let deleted = db.delete_dependencies_for_mod(mod_a).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(db.get_dependencies(mod_a).unwrap().len(), 0);
}

#[test]
fn reverse_dependencies() {
    let db = test_db();
    let mod_a = db
        .insert_mod(
            Some(1),
            Some(100),
            "Mod A",
            Some("mod-a"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    let mod_b = db
        .insert_mod(
            Some(2),
            Some(200),
            "Mod B",
            Some("mod-b"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
    let mod_c = db
        .insert_mod(
            Some(3),
            Some(300),
            "Mod C",
            Some("mod-c"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

    db.insert_dependency(mod_a, mod_b, None).unwrap();
    db.insert_dependency(mod_c, mod_b, Some(">=2.0.0")).unwrap();

    let rev = db.get_reverse_dependencies(mod_b).unwrap();
    assert_eq!(rev.len(), 2);

    let dependent_ids: Vec<i64> = rev.iter().map(|d| d.mod_id).collect();
    assert!(dependent_ids.contains(&mod_a));
    assert!(dependent_ids.contains(&mod_c));
}

#[test]
fn insert_and_get_user() {
    let db = test_db();
    let id = db
        .insert_user(
            "alice",
            Some("profile-abc"),
            Some("hashed_pw"),
            "admin",
            false,
        )
        .unwrap();
    assert!(id > 0);

    let user = db
        .get_user_by_username("alice")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.username, "alice");
    assert_eq!(user.spt_profile_id.as_deref(), Some("profile-abc"));
    assert_eq!(user.password_hash.as_deref(), Some("hashed_pw"));
    assert_eq!(user.role, "admin");

    let missing = db.get_user_by_username("bob").unwrap();
    assert!(missing.is_none());

    let users = db.list_users().unwrap();
    assert_eq!(users.len(), 1);
}

#[test]
fn insert_user_without_password() {
    let db = test_db();
    let id = db
        .insert_user("trusty", Some("profile-xyz"), None, "player", false)
        .unwrap();
    let user = db
        .get_user_by_username("trusty")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.id, id);
    assert!(user.password_hash.is_none());
}

#[test]
fn insert_user_without_profile() {
    let db = test_db();
    let id = db
        .insert_user("admin", None, Some("hashed_pw"), "admin", false)
        .unwrap();
    assert!(id > 0);

    let user = db
        .get_user_by_username("admin")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.username, "admin");
    assert!(user.spt_profile_id.is_none());
    assert_eq!(user.role, "admin");
}

#[test]
fn has_user_manager_check() {
    let db = test_db();
    assert!(!db.has_user_manager().unwrap());

    db.insert_user("player1", Some("p1"), Some("pw"), "player", false)
        .unwrap();
    assert!(!db.has_user_manager().unwrap());

    db.insert_user("admin1", Some("a1"), Some("pw"), "admin", false)
        .unwrap();
    assert!(db.has_user_manager().unwrap());

    // Custom role with users.manage should also count
    db.create_role("ops", "Ops", &[crate::db::rbac::Permission::UsersManage])
        .unwrap();
    db.insert_user("ops1", Some("o1"), Some("pw"), "ops", false)
        .unwrap();
    assert!(db.has_user_manager().unwrap());
}

#[test]
fn create_and_use_invite() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", Some("adm-profile"), Some("pw"), "admin", false)
        .unwrap();

    let invite_id = db
        .create_invite("INVITE-123", Some(admin_id), None)
        .unwrap();
    assert!(invite_id > 0);

    let invite = db
        .get_invite("INVITE-123")
        .unwrap()
        .expect("invite should exist");
    assert_eq!(invite.code, "INVITE-123");
    assert!(invite.used_by.is_none());

    let new_user_id = db
        .insert_user("newbie", Some("new-profile"), Some("pw"), "player", false)
        .unwrap();
    let used = db.use_invite("INVITE-123", new_user_id).unwrap();
    assert_eq!(used, 1);

    let invite = db
        .get_invite("INVITE-123")
        .unwrap()
        .expect("invite should exist");
    assert_eq!(invite.used_by, Some(new_user_id));
    assert!(invite.used_at.is_some());

    let another_user_id = db
        .insert_user("another", Some("anot-profile"), Some("pw"), "player", false)
        .unwrap();
    let reused = db.use_invite("INVITE-123", another_user_id).unwrap();
    assert_eq!(reused, 0, "already-used invite should not be reusable");
}

#[test]
fn create_invite_without_creator() {
    let db = test_db();
    let invite_id = db.create_invite("ORPHAN-1", None, None).unwrap();
    let invite = db
        .get_invite("ORPHAN-1")
        .unwrap()
        .expect("invite should exist");
    assert_eq!(invite.id, invite_id);
    assert!(invite.created_by.is_none());
}

#[test]
fn expired_invite_rejected() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", Some("adm-profile"), Some("pw"), "admin", false)
        .unwrap();

    db.create_invite("EXPIRED-1", Some(admin_id), Some("2020-01-01 00:00:00"))
        .unwrap();

    let user_id = db
        .insert_user(
            "latecomer",
            Some("late-profile"),
            Some("pw"),
            "player",
            false,
        )
        .unwrap();
    let used = db.use_invite("EXPIRED-1", user_id).unwrap();
    assert_eq!(used, 0, "expired invite should be rejected");
}

#[test]
fn pending_operations_crud() {
    let db = test_db();

    use crate::db::users::QueueAction;

    let op_id = db
        .insert_pending_op(&crate::db::users::InsertPendingOp {
            action: QueueAction::Install,
            forge_mod_id: Some(1001),
            forge_version_id: Some(2001),
            mod_name: "Cool Mod",
            metadata: Some("{\"source\":\"web\"}"),
            queued_by: Some("admin"),
            item_type: "mod",
            forge_addon_id: None,
            archive_path: None,
            source: "forge",
            source_url: None,
        })
        .unwrap();
    assert!(op_id > 0);

    db.insert_pending_op(&crate::db::users::InsertPendingOp {
        action: QueueAction::Update,
        forge_mod_id: Some(1002),
        forge_version_id: Some(2002),
        mod_name: "Other Mod",
        metadata: None,
        queued_by: None,
        item_type: "mod",
        forge_addon_id: None,
        archive_path: None,
        source: "forge",
        source_url: None,
    })
    .unwrap();

    let ops = db.list_pending_ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].action, QueueAction::Install);
    assert_eq!(ops[0].mod_name, "Cool Mod");
    assert_eq!(ops[0].metadata.as_deref(), Some("{\"source\":\"web\"}"));
    assert_eq!(ops[0].queued_by.as_deref(), Some("admin"));

    let deleted = db.delete_pending_op(op_id).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(db.list_pending_ops().unwrap().len(), 1);

    db.clear_pending_ops().unwrap();
    assert_eq!(db.list_pending_ops().unwrap().len(), 0);
}

#[test]
fn pending_op_with_no_version() {
    let db = test_db();
    let op_id = db
        .insert_pending_op(&crate::db::users::InsertPendingOp {
            action: crate::db::users::QueueAction::Remove,
            forge_mod_id: Some(1001),
            forge_version_id: None,
            mod_name: "Removed Mod",
            metadata: None,
            queued_by: None,
            item_type: "mod",
            forge_addon_id: None,
            archive_path: None,
            source: "forge",
            source_url: None,
        })
        .unwrap();
    let ops = db.list_pending_ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].id, op_id);
    assert!(ops[0].forge_version_id.is_none());
}

#[test]
fn lookup_mod_by_name_or_slug() {
    let db = Database::open_in_memory().unwrap();
    // Use a name that differs from the slug to test both paths
    db.insert_mod(
        Some(100),
        Some(200),
        "S.A.I.N.",
        Some("sain"),
        "3.0.0",
        "forge",
        None,
    )
    .unwrap();

    // Lookup by name (case-insensitive)
    let by_name = db.get_mod_by_name_or_slug("S.A.I.N.").unwrap();
    assert!(by_name.is_some());
    assert_eq!(by_name.as_ref().unwrap().forge_mod_id, Some(100));

    // Lookup by slug (distinct from name)
    let by_slug = db.get_mod_by_name_or_slug("sain").unwrap();
    assert!(by_slug.is_some());
    assert_eq!(by_slug.unwrap().name, "S.A.I.N.");

    // Not found
    let missing = db.get_mod_by_name_or_slug("nonexistent").unwrap();
    assert!(missing.is_none());
}

// Role enum tests removed — capabilities are now permission-based (see rbac::tests)

#[test]
fn get_user_by_id() {
    let db = test_db();
    let id = db
        .insert_user(
            "alice",
            Some("profile-abc"),
            Some("hashed_pw"),
            "admin",
            false,
        )
        .unwrap();
    let user = db.get_user_by_id(id).unwrap().expect("user should exist");
    assert_eq!(user.username, "alice");
    assert_eq!(user.role, "admin");
    assert!(!user.disabled);

    let missing = db.get_user_by_id(99999).unwrap();
    assert!(missing.is_none());
}

#[test]
fn user_disabled_default() {
    let db = test_db();
    let id = db
        .insert_user(
            "alice",
            Some("profile-abc"),
            Some("hashed_pw"),
            "player",
            false,
        )
        .unwrap();
    let user = db.get_user_by_id(id).unwrap().expect("user should exist");
    assert!(!user.disabled);
}

#[test]
fn update_user_role() {
    let db = test_db();
    let id = db
        .insert_user("alice", Some("p1"), Some("pw"), "player", false)
        .unwrap();
    let affected = db.update_user_role(id, "moderator").unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(id).unwrap().unwrap();
    assert_eq!(user.role, "moderator");
}

#[test]
fn update_user_role_last_admin_guard() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    // Only admin — guard should block demotion
    let affected = db.update_user_role(admin_id, "player").unwrap();
    assert_eq!(affected, 0, "should not demote the last admin");
    let user = db.get_user_by_id(admin_id).unwrap().unwrap();
    assert_eq!(user.role, "admin");
}

#[test]
fn update_user_role_allows_demotion_with_other_admins() {
    let db = test_db();
    let admin1 = db
        .insert_user("admin1", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    db.insert_user("admin2", Some("p2"), Some("pw"), "admin", false)
        .unwrap();
    let affected = db.update_user_role(admin1, "player").unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(admin1).unwrap().unwrap();
    assert_eq!(user.role, "player");
}

#[test]
fn set_user_disabled() {
    let db = test_db();
    let id = db
        .insert_user("alice", Some("p1"), Some("pw"), "player", false)
        .unwrap();
    let affected = db.set_user_disabled(id, true).unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(id).unwrap().unwrap();
    assert!(user.disabled);

    let affected = db.set_user_disabled(id, false).unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(id).unwrap().unwrap();
    assert!(!user.disabled);
}

#[test]
fn set_user_disabled_last_admin_guard() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    let affected = db.set_user_disabled(admin_id, true).unwrap();
    assert_eq!(affected, 0, "should not disable the last admin");
    let user = db.get_user_by_id(admin_id).unwrap().unwrap();
    assert!(!user.disabled);
}

#[test]
fn update_user_password() {
    let db = test_db();
    let id = db
        .insert_user("alice", Some("p1"), Some("old_hash"), "player", false)
        .unwrap();
    let affected = db.update_user_password(id, "new_hash").unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(id).unwrap().unwrap();
    assert_eq!(user.password_hash.as_deref(), Some("new_hash"));
}

#[test]
fn count_admins() {
    let db = test_db();
    assert_eq!(db.count_admins().unwrap(), 0);
    db.insert_user("admin1", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 1);
    db.insert_user("player1", Some("p2"), Some("pw"), "player", false)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 1);
    db.insert_user("admin2", Some("p3"), Some("pw"), "admin", false)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 2);
}

#[test]
fn list_invite_codes_with_usernames() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    db.create_invite("CODE-1", Some(admin_id), None).unwrap();
    db.create_invite("CODE-2", None, None).unwrap();

    let codes = db.list_invite_codes().unwrap();
    assert_eq!(codes.len(), 2);
    // Most recent first
    assert_eq!(codes[0].invite.code, "CODE-2");
    assert!(codes[0].created_by_username.is_none());
    assert_eq!(codes[1].invite.code, "CODE-1");
    assert_eq!(codes[1].created_by_username.as_deref(), Some("admin"));
}

#[test]
fn reset_token_crud() {
    let db = test_db();
    let user_id = db
        .insert_user("alice", Some("p1"), Some("pw"), "player", false)
        .unwrap();

    let token_id = db
        .create_reset_token(user_id, "token123", "2099-01-01T00:00:00Z")
        .unwrap();
    assert!(token_id > 0);

    let token = db.get_reset_token("token123").unwrap().unwrap();
    assert_eq!(token.user_id, user_id);
    assert_eq!(token.token, "token123");

    let missing = db.get_reset_token("nonexistent").unwrap();
    assert!(missing.is_none());

    let deleted = db.delete_reset_token("token123").unwrap();
    assert_eq!(deleted, 1);
    assert!(db.get_reset_token("token123").unwrap().is_none());
}

#[test]
fn reset_token_replaces_existing() {
    let db = test_db();
    let user_id = db
        .insert_user("alice", Some("p1"), Some("pw"), "player", false)
        .unwrap();

    db.create_reset_token(user_id, "token-old", "2099-01-01T00:00:00Z")
        .unwrap();
    db.create_reset_token(user_id, "token-new", "2099-01-01T00:00:00Z")
        .unwrap();

    assert!(db.get_reset_token("token-old").unwrap().is_none());
    assert!(db.get_reset_token("token-new").unwrap().is_some());
}

#[test]
fn password_reset_tokens_table_exists() {
    let db = test_db();
    let tables: Vec<String> = {
        let mut stmt = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='password_reset_tokens'")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap()
    };
    assert!(tables.contains(&"password_reset_tokens".to_string()));
}

#[test]
fn update_user_password_sets_changed_at() {
    let db = test_db();
    let id = db
        .insert_user("alice", Some("p1"), Some("old_hash"), "player", false)
        .unwrap();

    // Before update, password_changed_at should be NULL
    let before: Option<String> = db
        .conn()
        .query_row(
            "SELECT password_changed_at FROM users WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(before.is_none());

    db.update_user_password(id, "new_hash").unwrap();

    let after: Option<String> = db
        .conn()
        .query_row(
            "SELECT password_changed_at FROM users WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        after.is_some(),
        "password_changed_at should be set after update"
    );
}

#[test]
fn count_admins_excludes_disabled() {
    let db = test_db();
    let admin1 = db
        .insert_user("admin1", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    db.insert_user("admin2", Some("p2"), Some("pw"), "admin", false)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 2);
    db.set_user_disabled(admin1, true).unwrap();
    assert_eq!(db.count_admins().unwrap(), 1);
}

// -- Mod Request tests --

fn setup_user(db: &Database) -> i64 {
    db.insert_user("testuser", Some("aid1"), Some("hash123"), "player", false)
        .unwrap()
}

fn setup_admin(db: &Database) -> i64 {
    db.insert_user("admin", Some("aid2"), Some("hash456"), "admin", false)
        .unwrap()
}

#[test]
fn create_and_get_mod_request() {
    let db = test_db();
    let user_id = setup_user(&db);
    let req_id = db
        .create_mod_request(
            user_id,
            100,
            "Test Mod",
            Some("test-mod"),
            Some("A desc"),
            "unknown",
            Some("I want this"),
        )
        .unwrap();
    assert!(req_id > 0);

    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.forge_mod_id, 100);
    assert_eq!(req.mod_name, "Test Mod");
    assert_eq!(req.mod_slug.as_deref(), Some("test-mod"));
    assert_eq!(req.status, "pending");
    assert_eq!(req.reason.as_deref(), Some("I want this"));
    assert!(req.resolved_by.is_none());
}

#[test]
fn has_active_request_for_mod() {
    let db = test_db();
    let user_id = setup_user(&db);
    assert!(!db.has_active_request_for_mod(100).unwrap());

    let req_id = db
        .create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();
    assert!(db.has_active_request_for_mod(100).unwrap());

    // Approved requests should also return true (active)
    db.transition_request_status(
        req_id,
        &[RequestStatus::Pending],
        RequestStatus::Approved,
        Some(user_id),
        None,
    )
    .unwrap();
    assert!(db.has_active_request_for_mod(100).unwrap());
}

#[test]
fn resolved_request_does_not_block_new_request() {
    let db = test_db();
    let user_id = setup_user(&db);
    let admin_id = setup_admin(&db);

    let req_id = db
        .create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();
    db.transition_request_status(
        req_id,
        &[RequestStatus::Pending],
        RequestStatus::Rejected,
        Some(admin_id),
        Some("Not now"),
    )
    .unwrap();

    // Rejected requests don't block new requests
    assert!(!db.has_active_request_for_mod(100).unwrap());

    // But approved requests do block
    let req_id2 = db
        .create_mod_request(user_id, 200, "Mod2", None, None, "unknown", None)
        .unwrap();
    db.transition_request_status(
        req_id2,
        &[RequestStatus::Pending],
        RequestStatus::Approved,
        Some(admin_id),
        None,
    )
    .unwrap();
    assert!(db.has_active_request_for_mod(200).unwrap());
}

#[test]
fn transition_request_status_guards_wrong_from() {
    let db = test_db();
    let user_id = setup_user(&db);
    let admin_id = setup_admin(&db);

    let req_id = db
        .create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    // Transition from pending to approved should succeed
    let ok = db
        .transition_request_status(
            req_id,
            &[RequestStatus::Pending],
            RequestStatus::Approved,
            Some(admin_id),
            None,
        )
        .unwrap();
    assert!(ok);

    // Transition from pending to rejected should fail (status is now approved)
    let ok = db
        .transition_request_status(
            req_id,
            &[RequestStatus::Pending],
            RequestStatus::Rejected,
            Some(admin_id),
            None,
        )
        .unwrap();
    assert!(!ok);

    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.status, "approved");
}

#[test]
fn upsert_vote_and_toggle() {
    let db = test_db();
    let user_id = setup_user(&db);
    let req_id = db
        .create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    db.upsert_vote(req_id, user_id, true, Some("Great mod!"))
        .unwrap();
    let vote = db.get_vote(req_id, user_id).unwrap().unwrap();
    assert!(vote.upvote);
    assert_eq!(vote.comment.as_deref(), Some("Great mod!"));

    db.upsert_vote(req_id, user_id, false, None).unwrap();
    let vote = db.get_vote(req_id, user_id).unwrap().unwrap();
    assert!(!vote.upvote);
    assert!(vote.comment.is_none());

    db.delete_vote(req_id, user_id).unwrap();
    assert!(db.get_vote(req_id, user_id).unwrap().is_none());
}

#[test]
fn list_mod_requests_with_votes() {
    let db = test_db();
    let user1 = setup_user(&db);
    let user2 = setup_admin(&db);

    let req_id = db
        .create_mod_request(user1, 100, "Mod A", None, None, "compatible", None)
        .unwrap();

    db.upsert_vote(req_id, user1, true, Some("yes please"))
        .unwrap();
    db.upsert_vote(req_id, user2, true, None).unwrap();

    let views = db.list_mod_requests(Some("pending"), user1).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].vote_score, 2);
    assert_eq!(views[0].upvote_count, 2);
    assert_eq!(views[0].downvote_count, 0);
    assert_eq!(views[0].comment_count, 1);
    assert_eq!(views[0].current_user_vote, Some(true));
    assert_eq!(views[0].requester_username, "testuser");
}

#[test]
fn list_vote_comments_only_with_text() {
    let db = test_db();
    let user1 = setup_user(&db);
    let user2 = setup_admin(&db);

    let req_id = db
        .create_mod_request(user1, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    db.upsert_vote(req_id, user1, true, Some("Love it"))
        .unwrap();
    db.upsert_vote(req_id, user2, false, None).unwrap();

    let comments = db.list_vote_comments(req_id).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].username, "testuser");
    assert!(comments[0].upvote);
    assert_eq!(comments[0].comment, "Love it");
}

#[test]
fn update_mod_request_cache() {
    let db = test_db();
    let user_id = setup_user(&db);

    let req_id = db
        .create_mod_request(user_id, 100, "Old Name", None, None, "unknown", None)
        .unwrap();

    db.update_mod_request_cache(
        req_id,
        "New Name",
        Some("new-slug"),
        Some("New desc"),
        "compatible",
    )
    .unwrap();

    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.mod_name, "New Name");
    assert_eq!(req.mod_slug.as_deref(), Some("new-slug"));
    assert_eq!(req.fika_compatible, "compatible");
}

#[test]
fn list_mod_requests_all_statuses() {
    let db = test_db();
    let user_id = setup_user(&db);

    db.create_mod_request(user_id, 100, "Mod A", None, None, "unknown", None)
        .unwrap();
    db.create_mod_request(user_id, 200, "Mod B", None, None, "unknown", None)
        .unwrap();

    let all = db.list_mod_requests(None, user_id).unwrap();
    assert_eq!(all.len(), 2);

    let pending = db.list_mod_requests(Some("pending"), user_id).unwrap();
    assert_eq!(pending.len(), 2);

    let approved = db.list_mod_requests(Some("approved"), user_id).unwrap();
    assert_eq!(approved.len(), 0);
}

#[test]
fn list_users_alphabetical_order() {
    let db = test_db();
    db.insert_user("charlie", Some("p3"), Some("pw"), "player", false)
        .unwrap();
    db.insert_user("alice", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    db.insert_user("bob", Some("p2"), Some("pw"), "moderator", false)
        .unwrap();

    let users = db.list_users().unwrap();
    assert_eq!(users.len(), 3);
    assert_eq!(users[0].username, "alice");
    assert_eq!(users[1].username, "bob");
    assert_eq!(users[2].username, "charlie");
}

#[test]
fn disable_admin_allowed_with_backup_admin() {
    let db = test_db();
    let admin1 = db
        .insert_user("admin1", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    db.insert_user("admin2", Some("p2"), Some("pw"), "admin", false)
        .unwrap();

    let affected = db.set_user_disabled(admin1, true).unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(admin1).unwrap().unwrap();
    assert!(user.disabled);
}

#[test]
fn list_requests_mixed_votes_score() {
    let db = test_db();
    let user1 = setup_user(&db);
    let user2 = setup_admin(&db);
    let user3 = db
        .insert_user("voter3", Some("aid3"), Some("hash"), "player", false)
        .unwrap();

    let req_id = db
        .create_mod_request(user1, 100, "Mod A", None, None, "unknown", None)
        .unwrap();

    // 2 upvotes, 1 downvote → score = 1
    db.upsert_vote(req_id, user1, true, None).unwrap();
    db.upsert_vote(req_id, user2, true, Some("good mod"))
        .unwrap();
    db.upsert_vote(req_id, user3, false, Some("not needed"))
        .unwrap();

    let views = db.list_mod_requests(Some("pending"), user1).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].upvote_count, 2);
    assert_eq!(views[0].downvote_count, 1);
    assert_eq!(views[0].vote_score, 1);
    assert_eq!(views[0].comment_count, 2);
}

#[test]
fn list_requests_current_user_no_vote() {
    let db = test_db();
    let requester = setup_user(&db);
    let viewer = db
        .insert_user("viewer", Some("aid-v"), Some("hash"), "player", false)
        .unwrap();

    db.create_mod_request(requester, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    let views = db.list_mod_requests(Some("pending"), viewer).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].current_user_vote, None);
}

#[test]
fn get_user_by_spt_profile_id_found() {
    let db = test_db();
    db.insert_user("alice", Some("profile-123"), Some("hash"), "player", false)
        .unwrap();
    let user = db.get_user_by_spt_profile_id("profile-123").unwrap();
    assert!(user.is_some());
    assert_eq!(user.unwrap().username, "alice");
}

#[test]
fn get_user_by_spt_profile_id_not_found() {
    let db = test_db();
    db.insert_user("alice", Some("profile-123"), Some("hash"), "player", false)
        .unwrap();
    let user = db.get_user_by_spt_profile_id("profile-999").unwrap();
    assert!(user.is_none());
}

#[test]
fn get_user_by_spt_profile_id_null_profile() {
    let db = test_db();
    db.insert_user("bob", None, Some("hash"), "player", false)
        .unwrap();
    let user = db.get_user_by_spt_profile_id("anything").unwrap();
    assert!(user.is_none());
}

#[test]
fn has_pending_op_check() {
    use crate::db::users::QueueAction;

    let db = test_db();

    // No ops exist yet
    assert!(!db.has_pending_op(42, QueueAction::Install).unwrap());

    // Insert an Install op for forge_mod_id=42
    db.insert_pending_op(&crate::db::users::InsertPendingOp {
        action: QueueAction::Install,
        forge_mod_id: Some(42),
        forge_version_id: Some(100),
        mod_name: "Test Mod",
        metadata: None,
        queued_by: None,
        item_type: "mod",
        forge_addon_id: None,
        archive_path: None,
        source: "forge",
        source_url: None,
    })
    .unwrap();

    // Same mod_id + action → true
    assert!(db.has_pending_op(42, QueueAction::Install).unwrap());

    // Different mod_id → false
    assert!(!db.has_pending_op(99, QueueAction::Install).unwrap());

    // Same mod_id, different action → false
    assert!(!db.has_pending_op(42, QueueAction::Update).unwrap());
}

#[test]
fn delete_user_removes_user() {
    use crate::db::users::DeleteUserResult;

    let db = test_db();
    let _admin = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    let player = db
        .insert_user("player", Some("p2"), Some("pw"), "player", false)
        .unwrap();

    let result = db.delete_user(player).unwrap();
    assert!(matches!(result, DeleteUserResult::Deleted));
    assert!(db.get_user_by_id(player).unwrap().is_none());
    assert_eq!(db.list_users().unwrap().len(), 1);
}

#[test]
fn delete_user_not_found() {
    use crate::db::users::DeleteUserResult;

    let db = test_db();
    let result = db.delete_user(99999).unwrap();
    assert!(matches!(result, DeleteUserResult::NotFound));
}

#[test]
fn delete_user_last_admin_blocked() {
    use crate::db::users::DeleteUserResult;

    let db = test_db();
    let admin = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();

    let result = db.delete_user(admin).unwrap();
    assert!(matches!(result, DeleteUserResult::LastAdmin));
    assert!(db.get_user_by_id(admin).unwrap().is_some());
}

#[test]
fn delete_user_allowed_with_other_admin() {
    use crate::db::users::DeleteUserResult;

    let db = test_db();
    let admin1 = db
        .insert_user("admin1", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    db.insert_user("admin2", Some("p2"), Some("pw"), "admin", false)
        .unwrap();

    let result = db.delete_user(admin1).unwrap();
    assert!(matches!(result, DeleteUserResult::Deleted));
}

#[test]
fn delete_user_cascades_mod_requests() {
    use crate::db::users::DeleteUserResult;

    let db = test_db();
    let admin = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    let player = db
        .insert_user("player", Some("p2"), Some("pw"), "player", false)
        .unwrap();

    db.create_mod_request(player, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    let result = db.delete_user(player).unwrap();
    assert!(matches!(result, DeleteUserResult::Deleted));

    let requests = db.list_mod_requests(None, admin).unwrap();
    assert_eq!(requests.len(), 0);
}

#[test]
fn delete_user_sets_invite_created_by_null() {
    use crate::db::users::DeleteUserResult;

    let db = test_db();
    db.insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    let player = db
        .insert_user("player", Some("p2"), Some("pw"), "player", false)
        .unwrap();

    db.create_invite("CODE-BY-PLAYER", Some(player), None)
        .unwrap();

    let result = db.delete_user(player).unwrap();
    assert!(matches!(result, DeleteUserResult::Deleted));

    let invite = db
        .get_invite("CODE-BY-PLAYER")
        .unwrap()
        .expect("invite should still exist");
    assert!(
        invite.created_by.is_none(),
        "created_by should be NULL after user deletion"
    );
}

#[test]
fn delete_invite_unused() {
    use crate::db::users::DeleteInviteResult;

    let db = test_db();
    let admin = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    let invite_id = db.create_invite("CODE-DEL", Some(admin), None).unwrap();

    let result = db.delete_invite(invite_id).unwrap();
    assert!(matches!(result, DeleteInviteResult::Deleted));
    assert!(db.get_invite("CODE-DEL").unwrap().is_none());
}

#[test]
fn delete_invite_already_used() {
    use crate::db::users::DeleteInviteResult;

    let db = test_db();
    let admin = db
        .insert_user("admin", Some("p1"), Some("pw"), "admin", false)
        .unwrap();
    let invite_id = db.create_invite("CODE-USED", Some(admin), None).unwrap();
    let player = db
        .insert_user("player", Some("p2"), Some("pw"), "player", false)
        .unwrap();
    db.use_invite("CODE-USED", player).unwrap();

    let result = db.delete_invite(invite_id).unwrap();
    assert!(matches!(result, DeleteInviteResult::AlreadyUsed));
    assert!(db.get_invite("CODE-USED").unwrap().is_some());
}

#[test]
fn delete_invite_not_found() {
    use crate::db::users::DeleteInviteResult;

    let db = test_db();
    let result = db.delete_invite(99999).unwrap();
    assert!(matches!(result, DeleteInviteResult::NotFound));
}

#[test]
fn request_status_round_trip() {
    for status in ["pending", "approved", "queued", "installed", "rejected"] {
        let parsed: RequestStatus = status.parse().unwrap();
        assert_eq!(parsed.as_str(), status);
    }
    assert!("bogus".parse::<RequestStatus>().is_err());
}

#[test]
fn transition_request_status_logs_change() {
    // Create a request, transition it, verify the log entry exists
    let db = test_db();
    let user_id = db
        .insert_user("admin", Some("aid"), Some("hash"), "admin", false)
        .unwrap();
    let req_id = db
        .create_mod_request(user_id, 100, "TestMod", None, None, "unknown", None)
        .unwrap();

    let ok = db
        .transition_request_status(
            req_id,
            &[RequestStatus::Pending],
            RequestStatus::Approved,
            Some(user_id),
            Some("looks good"),
        )
        .unwrap();
    assert!(ok);

    let log = db.get_request_status_log(req_id).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].from_status, "pending");
    assert_eq!(log[0].to_status, "approved");
    assert_eq!(log[0].comment.as_deref(), Some("looks good"));
}

#[test]
fn transition_request_status_rejects_wrong_from() {
    let db = test_db();
    let user_id = db
        .insert_user("admin", Some("aid"), Some("hash"), "admin", false)
        .unwrap();
    let req_id = db
        .create_mod_request(user_id, 100, "TestMod", None, None, "unknown", None)
        .unwrap();

    // Try to transition from approved (but it's pending) — should return false
    let ok = db
        .transition_request_status(
            req_id,
            &[RequestStatus::Approved],
            RequestStatus::Rejected,
            Some(user_id),
            None,
        )
        .unwrap();
    assert!(!ok);

    // Status should still be pending
    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.status, "pending");
}

#[test]
fn transition_request_by_forge_mod_id() {
    let db = test_db();
    let user_id = db
        .insert_user("admin", Some("aid"), Some("hash"), "admin", false)
        .unwrap();

    // Create three requests: two for mod 100, one for mod 200
    let req1 = db
        .create_mod_request(user_id, 100, "Mod100", None, None, "unknown", None)
        .unwrap();
    let req2 = db
        .create_mod_request(user_id, 100, "Mod100", None, None, "unknown", None)
        .unwrap();
    let req3 = db
        .create_mod_request(user_id, 200, "Mod200", None, None, "unknown", None)
        .unwrap();

    // Approve req1
    db.transition_request_status(
        req1,
        &[RequestStatus::Pending],
        RequestStatus::Approved,
        Some(user_id),
        None,
    )
    .unwrap();

    // Transition all mod 100 requests from approved to installed
    let count = db
        .transition_request_by_forge_mod_id(
            100,
            &[RequestStatus::Approved],
            RequestStatus::Installed,
            None,
            None,
        )
        .unwrap();
    assert_eq!(count, 1);

    let r1 = db.get_mod_request(req1).unwrap().unwrap();
    let r2 = db.get_mod_request(req2).unwrap().unwrap();
    let r3 = db.get_mod_request(req3).unwrap().unwrap();

    assert_eq!(r1.status, "installed");
    assert_eq!(r2.status, "pending"); // not approved, so not transitioned
    assert_eq!(r3.status, "pending"); // different mod, not transitioned
}

#[test]
fn set_request_resolver() {
    let db = test_db();
    let user1 = db
        .insert_user("user1", Some("p1"), Some("hash"), "admin", false)
        .unwrap();
    let user2 = db
        .insert_user("user2", Some("p2"), Some("hash"), "admin", false)
        .unwrap();

    let req_id = db
        .create_mod_request(user1, 100, "TestMod", None, None, "unknown", None)
        .unwrap();

    // Set resolver for first time
    db.set_request_resolver(req_id, user2, Some("resolving"))
        .unwrap();

    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.resolved_by, Some(user2));
    assert_eq!(req.resolve_comment.as_deref(), Some("resolving"));

    // Try to set resolver again — should have no effect (already set)
    db.set_request_resolver(req_id, user1, Some("second attempt"))
        .unwrap();

    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.resolved_by, Some(user2)); // unchanged
    assert_eq!(req.resolve_comment.as_deref(), Some("resolving")); // unchanged
}

#[test]
fn insert_pending_op_stores_all_fields() {
    use crate::db::users::{InsertPendingOp, QueueAction};

    let db = test_db();
    db.insert_pending_op(&InsertPendingOp {
        action: QueueAction::Install,
        forge_mod_id: Some(42),
        forge_version_id: Some(100),
        mod_name: "TestMod",
        metadata: Some("{\"version\":\"1.0.0\"}"),
        queued_by: Some("testuser"),
        item_type: "mod",
        forge_addon_id: None,
        archive_path: Some("/tmp/test.zip"),
        source: "forge",
        source_url: Some("https://forge.sp-tarkov.com/dl/123"),
    })
    .unwrap();

    let ops = db.list_pending_ops().unwrap();
    assert_eq!(ops.len(), 1);
    let op = &ops[0];
    assert_eq!(op.forge_mod_id, Some(42));
    assert_eq!(op.archive_path.as_deref(), Some("/tmp/test.zip"));
    assert_eq!(op.source, "forge");
    assert_eq!(
        op.source_url.as_deref(),
        Some("https://forge.sp-tarkov.com/dl/123")
    );
}

#[test]
fn list_dep_ops_for_parent_filters_correctly() {
    use crate::db::users::{InsertPendingOp, QueueAction};

    let db = test_db();
    // Insert a dep op queued for parent forge_mod_id 42 (SAIN)
    db.insert_pending_op(&InsertPendingOp {
        action: QueueAction::Install,
        forge_mod_id: Some(10),
        forge_version_id: Some(1),
        mod_name: "BigBrain",
        metadata: Some("{\"version\":\"1.0\",\"queued_for\":[42]}"),
        queued_by: None,
        item_type: "mod",
        forge_addon_id: None,
        archive_path: Some("/tmp/bigbrain.zip"),
        source: "forge",
        source_url: None,
    })
    .unwrap();
    // Insert a non-dep op (no queued_for)
    db.insert_pending_op(&InsertPendingOp {
        action: QueueAction::Install,
        forge_mod_id: Some(20),
        forge_version_id: Some(2),
        mod_name: "OtherMod",
        metadata: None,
        queued_by: None,
        item_type: "mod",
        forge_addon_id: None,
        archive_path: Some("/tmp/other.zip"),
        source: "forge",
        source_url: None,
    })
    .unwrap();

    let deps = db.list_dep_ops_for_parent(42).unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].mod_name, "BigBrain");

    // No deps for parent 99
    let deps = db.list_dep_ops_for_parent(99).unwrap();
    assert_eq!(deps.len(), 0);
}

#[test]
fn cascade_cancel_removes_parent_from_queued_for() {
    use crate::db::users::{InsertPendingOp, QueueAction};

    let db = test_db();
    // Dep queued for two parents: 42 and 55
    db.insert_pending_op(&InsertPendingOp {
        action: QueueAction::Install,
        forge_mod_id: Some(10),
        forge_version_id: Some(1),
        mod_name: "SharedDep",
        metadata: Some("{\"version\":\"1.0\",\"queued_for\":[42,55]}"),
        queued_by: None,
        item_type: "mod",
        forge_addon_id: None,
        archive_path: Some("/tmp/shared.zip"),
        source: "forge",
        source_url: None,
    })
    .unwrap();

    // Simulate cancelling parent 42: remove from queued_for
    let deps = db.list_dep_ops_for_parent(42).unwrap();
    assert_eq!(deps.len(), 1);

    let dep = &deps[0];
    let mut meta: serde_json::Value = serde_json::from_str(dep.metadata.as_ref().unwrap()).unwrap();
    let arr = meta.get_mut("queued_for").unwrap().as_array_mut().unwrap();
    arr.retain(|v| v.as_i64() != Some(42));
    assert_eq!(arr.len(), 1); // still has parent 55

    db.update_pending_op_metadata(dep.id, &serde_json::to_string(&meta).unwrap())
        .unwrap();

    // Now parent 42 should find no deps
    let deps = db.list_dep_ops_for_parent(42).unwrap();
    assert_eq!(deps.len(), 0);

    // Parent 55 still finds the dep
    let deps = db.list_dep_ops_for_parent(55).unwrap();
    assert_eq!(deps.len(), 1);
}
