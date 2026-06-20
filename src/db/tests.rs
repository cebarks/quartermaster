use super::Database;
use crate::db::users::Role;

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
}

#[test]
fn insert_and_get_mod() {
    let db = test_db();
    let id = db
        .insert_mod(1001, 2001, "Test Mod", Some("test-mod"), "1.0.0")
        .unwrap();
    assert!(id > 0);

    let m = db.get_mod(id).unwrap().expect("mod should exist");
    assert_eq!(m.forge_mod_id, 1001);
    assert_eq!(m.forge_version_id, 2001);
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
    assert_eq!(m3.forge_version_id, 2002);
    assert_eq!(m3.version, "1.1.0");
    assert!(m3.updated_at.is_some());
}

#[test]
fn insert_mod_with_no_slug() {
    let db = test_db();
    let id = db
        .insert_mod(1001, 2001, "No Slug Mod", None, "1.0.0")
        .unwrap();
    let m = db.get_mod(id).unwrap().expect("mod should exist");
    assert!(m.slug.is_none());
}

#[test]
fn duplicate_forge_mod_id_rejected() {
    let db = test_db();
    db.insert_mod(1001, 2001, "Mod A", Some("mod-a"), "1.0.0")
        .unwrap();
    let result = db.insert_mod(1001, 2002, "Mod B", Some("mod-b"), "2.0.0");
    assert!(result.is_err(), "duplicate forge_mod_id should be rejected");
}

#[test]
fn delete_mod_cascades_to_files_and_deps() {
    let db = test_db();
    let mod_a = db
        .insert_mod(1, 100, "Mod A", Some("mod-a"), "1.0.0")
        .unwrap();
    let mod_b = db
        .insert_mod(2, 200, "Mod B", Some("mod-b"), "1.0.0")
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
        .insert_mod(1, 100, "Mod A", Some("mod-a"), "1.0.0")
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
        .insert_mod(1, 100, "Mod A", Some("mod-a"), "1.0.0")
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
        .insert_mod(1, 100, "Mod A", Some("mod-a"), "1.0.0")
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
        .insert_mod(1, 100, "Mod A", Some("mod-a"), "1.0.0")
        .unwrap();
    let mod_b = db
        .insert_mod(2, 200, "Mod B", Some("mod-b"), "1.0.0")
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
        .insert_mod(1, 100, "Mod A", Some("mod-a"), "1.0.0")
        .unwrap();
    let mod_b = db
        .insert_mod(2, 200, "Mod B", Some("mod-b"), "1.0.0")
        .unwrap();
    let mod_c = db
        .insert_mod(3, 300, "Mod C", Some("mod-c"), "1.0.0")
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
        .insert_user("alice", "profile-abc", Some("hashed_pw"), Role::Admin)
        .unwrap();
    assert!(id > 0);

    let user = db
        .get_user_by_username("alice")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.username, "alice");
    assert_eq!(user.spt_profile_id, "profile-abc");
    assert_eq!(user.password_hash.as_deref(), Some("hashed_pw"));
    assert_eq!(user.role, Role::Admin);

    let missing = db.get_user_by_username("bob").unwrap();
    assert!(missing.is_none());

    let users = db.list_users().unwrap();
    assert_eq!(users.len(), 1);
}

#[test]
fn insert_user_without_password() {
    let db = test_db();
    let id = db
        .insert_user("trusty", "profile-xyz", None, Role::Player)
        .unwrap();
    let user = db
        .get_user_by_username("trusty")
        .unwrap()
        .expect("user should exist");
    assert_eq!(user.id, id);
    assert!(user.password_hash.is_none());
}

#[test]
fn admin_exists_check() {
    let db = test_db();
    assert!(!db.admin_exists().unwrap());

    db.insert_user("player1", "p1", Some("pw"), Role::Player)
        .unwrap();
    assert!(!db.admin_exists().unwrap());

    db.insert_user("admin1", "a1", Some("pw"), Role::Admin)
        .unwrap();
    assert!(db.admin_exists().unwrap());
}

#[test]
fn create_and_use_invite() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", "adm-profile", Some("pw"), Role::Admin)
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
        .insert_user("newbie", "new-profile", Some("pw"), Role::Player)
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
        .insert_user("another", "anot-profile", Some("pw"), Role::Player)
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
        .insert_user("admin", "adm-profile", Some("pw"), Role::Admin)
        .unwrap();

    db.create_invite("EXPIRED-1", Some(admin_id), Some("2020-01-01 00:00:00"))
        .unwrap();

    let user_id = db
        .insert_user("latecomer", "late-profile", Some("pw"), Role::Player)
        .unwrap();
    let used = db.use_invite("EXPIRED-1", user_id).unwrap();
    assert_eq!(used, 0, "expired invite should be rejected");
}

#[test]
fn pending_operations_crud() {
    let db = test_db();

    let op_id = db
        .insert_pending_op(
            "install",
            1001,
            Some(2001),
            "Cool Mod",
            Some("{\"source\":\"web\"}"),
            Some("admin"),
        )
        .unwrap();
    assert!(op_id > 0);

    db.insert_pending_op("update", 1002, Some(2002), "Other Mod", None, None)
        .unwrap();

    let ops = db.list_pending_ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].action, "install");
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
        .insert_pending_op("remove", 1001, None, "Removed Mod", None, None)
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
    db.insert_mod(100, 200, "S.A.I.N.", Some("sain"), "3.0.0")
        .unwrap();

    // Lookup by name (case-insensitive)
    let by_name = db.get_mod_by_name_or_slug("S.A.I.N.").unwrap();
    assert!(by_name.is_some());
    assert_eq!(by_name.as_ref().unwrap().forge_mod_id, 100);

    // Lookup by slug (distinct from name)
    let by_slug = db.get_mod_by_name_or_slug("sain").unwrap();
    assert!(by_slug.is_some());
    assert_eq!(by_slug.unwrap().name, "S.A.I.N.");

    // Not found
    let missing = db.get_mod_by_name_or_slug("nonexistent").unwrap();
    assert!(missing.is_none());
}

#[test]
fn role_capabilities() {
    assert!(Role::Admin.can_manage_mods());
    assert!(Role::Admin.can_control_server());
    assert!(Role::Admin.can_manage_queue());
    assert!(Role::Admin.can_manage_users());

    assert!(Role::Moderator.can_manage_mods());
    assert!(Role::Moderator.can_control_server());
    assert!(Role::Moderator.can_manage_queue());
    assert!(!Role::Moderator.can_manage_users());

    assert!(!Role::Player.can_manage_mods());
    assert!(!Role::Player.can_control_server());
    assert!(!Role::Player.can_manage_queue());
    assert!(!Role::Player.can_manage_users());
}

#[test]
fn role_serialization_roundtrip() {
    assert_eq!(Role::Admin.as_str(), "admin");
    assert_eq!(Role::Moderator.as_str(), "moderator");
    assert_eq!(Role::Player.as_str(), "player");

    assert_eq!(Role::try_from("admin".to_string()), Ok(Role::Admin));
    assert_eq!(Role::try_from("moderator".to_string()), Ok(Role::Moderator));
    assert_eq!(Role::try_from("player".to_string()), Ok(Role::Player));
    assert!(Role::try_from("unknown".to_string()).is_err());
}

#[test]
fn role_display() {
    assert_eq!(format!("{}", Role::Admin), "Admin");
    assert_eq!(format!("{}", Role::Moderator), "Moderator");
    assert_eq!(format!("{}", Role::Player), "Player");
}

#[test]
fn role_serde_lowercase() {
    let json = serde_json::to_string(&Role::Admin).unwrap();
    assert_eq!(json, "\"admin\"");
    let parsed: Role = serde_json::from_str("\"moderator\"").unwrap();
    assert_eq!(parsed, Role::Moderator);
}

#[test]
fn get_user_by_id() {
    let db = test_db();
    let id = db
        .insert_user("alice", "profile-abc", Some("hashed_pw"), Role::Admin)
        .unwrap();
    let user = db.get_user_by_id(id).unwrap().expect("user should exist");
    assert_eq!(user.username, "alice");
    assert_eq!(user.role, Role::Admin);
    assert!(!user.disabled);

    let missing = db.get_user_by_id(99999).unwrap();
    assert!(missing.is_none());
}

#[test]
fn user_disabled_default() {
    let db = test_db();
    let id = db
        .insert_user("alice", "profile-abc", Some("hashed_pw"), Role::Player)
        .unwrap();
    let user = db.get_user_by_id(id).unwrap().expect("user should exist");
    assert!(!user.disabled);
}

#[test]
fn update_user_role() {
    let db = test_db();
    let id = db
        .insert_user("alice", "p1", Some("pw"), Role::Player)
        .unwrap();
    let affected = db.update_user_role(id, Role::Moderator).unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(id).unwrap().unwrap();
    assert_eq!(user.role, Role::Moderator);
}

#[test]
fn update_user_role_last_admin_guard() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", "p1", Some("pw"), Role::Admin)
        .unwrap();
    // Only admin — guard should block demotion
    let affected = db.update_user_role(admin_id, Role::Player).unwrap();
    assert_eq!(affected, 0, "should not demote the last admin");
    let user = db.get_user_by_id(admin_id).unwrap().unwrap();
    assert_eq!(user.role, Role::Admin);
}

#[test]
fn update_user_role_allows_demotion_with_other_admins() {
    let db = test_db();
    let admin1 = db
        .insert_user("admin1", "p1", Some("pw"), Role::Admin)
        .unwrap();
    db.insert_user("admin2", "p2", Some("pw"), Role::Admin)
        .unwrap();
    let affected = db.update_user_role(admin1, Role::Player).unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(admin1).unwrap().unwrap();
    assert_eq!(user.role, Role::Player);
}

#[test]
fn set_user_disabled() {
    let db = test_db();
    let id = db
        .insert_user("alice", "p1", Some("pw"), Role::Player)
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
        .insert_user("admin", "p1", Some("pw"), Role::Admin)
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
        .insert_user("alice", "p1", Some("old_hash"), Role::Player)
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
    db.insert_user("admin1", "p1", Some("pw"), Role::Admin)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 1);
    db.insert_user("player1", "p2", Some("pw"), Role::Player)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 1);
    db.insert_user("admin2", "p3", Some("pw"), Role::Admin)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 2);
}

#[test]
fn list_invite_codes_with_usernames() {
    let db = test_db();
    let admin_id = db
        .insert_user("admin", "p1", Some("pw"), Role::Admin)
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
        .insert_user("alice", "p1", Some("pw"), Role::Player)
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
        .insert_user("alice", "p1", Some("pw"), Role::Player)
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
        .insert_user("alice", "p1", Some("old_hash"), Role::Player)
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
        .insert_user("admin1", "p1", Some("pw"), Role::Admin)
        .unwrap();
    db.insert_user("admin2", "p2", Some("pw"), Role::Admin)
        .unwrap();
    assert_eq!(db.count_admins().unwrap(), 2);
    db.set_user_disabled(admin1, true).unwrap();
    assert_eq!(db.count_admins().unwrap(), 1);
}

// -- Mod Request tests --

fn setup_user(db: &Database) -> i64 {
    db.insert_user("testuser", "aid1", Some("hash123"), Role::Player)
        .unwrap()
}

fn setup_admin(db: &Database) -> i64 {
    db.insert_user("admin", "aid2", Some("hash456"), Role::Admin)
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
fn has_pending_request_for_mod() {
    let db = test_db();
    let user_id = setup_user(&db);
    assert!(!db.has_pending_request_for_mod(100).unwrap());

    db.create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();
    assert!(db.has_pending_request_for_mod(100).unwrap());
}

#[test]
fn resolved_request_does_not_block_new_request() {
    let db = test_db();
    let user_id = setup_user(&db);
    let admin_id = setup_admin(&db);

    let req_id = db
        .create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();
    db.resolve_mod_request(req_id, "rejected", admin_id, Some("Not now"))
        .unwrap();

    assert!(!db.has_pending_request_for_mod(100).unwrap());
}

#[test]
fn resolve_mod_request_only_pending() {
    let db = test_db();
    let user_id = setup_user(&db);
    let admin_id = setup_admin(&db);

    let req_id = db
        .create_mod_request(user_id, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    let rows = db
        .resolve_mod_request(req_id, "approved", admin_id, None)
        .unwrap();
    assert_eq!(rows, 1);

    let rows = db
        .resolve_mod_request(req_id, "rejected", admin_id, None)
        .unwrap();
    assert_eq!(rows, 0);

    let req = db.get_mod_request(req_id).unwrap().unwrap();
    assert_eq!(req.status, "approved");
    assert!(req.resolved_at.is_some());
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
