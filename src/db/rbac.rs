use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    ModsInstall,
    ModsUpdate,
    ModsRemove,
    ModsDisable,
    ModsyncManage,
    SvmEdit,
    RequestsResolve,
    ServerControl,
    ServerLogs,
    ServerMetrics,
    HeadlessManage,
    QueueManage,
    UsersManage,
    SettingsManage,
}

impl Permission {
    pub const ALL: &[Permission] = &[
        Permission::ModsInstall,
        Permission::ModsUpdate,
        Permission::ModsRemove,
        Permission::ModsDisable,
        Permission::ModsyncManage,
        Permission::SvmEdit,
        Permission::RequestsResolve,
        Permission::ServerControl,
        Permission::ServerLogs,
        Permission::ServerMetrics,
        Permission::HeadlessManage,
        Permission::QueueManage,
        Permission::UsersManage,
        Permission::SettingsManage,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::ModsInstall => "mods.install",
            Permission::ModsUpdate => "mods.update",
            Permission::ModsRemove => "mods.remove",
            Permission::ModsDisable => "mods.disable",
            Permission::ModsyncManage => "modsync.manage",
            Permission::SvmEdit => "svm.edit",
            Permission::RequestsResolve => "requests.resolve",
            Permission::ServerControl => "server.control",
            Permission::ServerLogs => "server.logs",
            Permission::ServerMetrics => "server.metrics",
            Permission::HeadlessManage => "headless.manage",
            Permission::QueueManage => "queue.manage",
            Permission::UsersManage => "users.manage",
            Permission::SettingsManage => "settings.manage",
        }
    }

    pub fn from_slug(s: &str) -> Option<Permission> {
        match s {
            "mods.install" => Some(Permission::ModsInstall),
            "mods.update" => Some(Permission::ModsUpdate),
            "mods.remove" => Some(Permission::ModsRemove),
            "mods.disable" => Some(Permission::ModsDisable),
            "modsync.manage" => Some(Permission::ModsyncManage),
            "svm.edit" => Some(Permission::SvmEdit),
            "requests.resolve" => Some(Permission::RequestsResolve),
            "server.control" => Some(Permission::ServerControl),
            "server.logs" => Some(Permission::ServerLogs),
            "server.metrics" => Some(Permission::ServerMetrics),
            "headless.manage" => Some(Permission::HeadlessManage),
            "queue.manage" => Some(Permission::QueueManage),
            "users.manage" => Some(Permission::UsersManage),
            "settings.manage" => Some(Permission::SettingsManage),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Permission::ModsInstall => "Install Mods",
            Permission::ModsUpdate => "Update Mods",
            Permission::ModsRemove => "Remove Mods",
            Permission::ModsDisable => "Disable Mods",
            Permission::ModsyncManage => "Manage NarcoNet",
            Permission::SvmEdit => "Edit Server Config",
            Permission::RequestsResolve => "Resolve Mod Requests",
            Permission::ServerControl => "Control Server",
            Permission::ServerLogs => "View Logs",
            Permission::ServerMetrics => "View Metrics",
            Permission::HeadlessManage => "Manage Headless Clients",
            Permission::QueueManage => "Manage Queue",
            Permission::UsersManage => "Manage Users",
            Permission::SettingsManage => "Manage Settings",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Permission::ModsInstall
            | Permission::ModsUpdate
            | Permission::ModsRemove
            | Permission::ModsDisable => "Mods",
            Permission::ModsyncManage | Permission::SvmEdit | Permission::RequestsResolve => {
                "Configuration"
            }
            Permission::ServerControl
            | Permission::ServerLogs
            | Permission::ServerMetrics
            | Permission::HeadlessManage => "Server",
            Permission::QueueManage => "Operations",
            Permission::UsersManage | Permission::SettingsManage => "Administration",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RoleRecord {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub built_in: bool,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct RoleWithPermissions {
    pub role: RoleRecord,
    pub permissions: HashSet<Permission>,
}

impl fmt::Display for RoleRecord {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.display_name)
    }
}

/// Named struct for template use (avoids tuple indexing in Askama)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PermissionInfo {
    pub slug: &'static str,
    pub display_name: &'static str,
    pub category: &'static str,
    pub checked: bool,
}

/// Validate a custom role name slug: [a-z0-9-], 1-32 chars, not a built-in name.
pub fn validate_role_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 32 {
        return Err("Role name must be 1-32 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("Role name may only contain lowercase letters, digits, and hyphens");
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("Role name cannot start or end with a hyphen");
    }
    if matches!(name, "admin" | "moderator" | "player") {
        return Err("Cannot use a built-in role name");
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Database Operations
// ──────────────────────────────────────────────────────────────────────────────

use rusqlite::{params, OptionalExtension};

use super::Database;

impl Database {
    pub fn list_roles(&self) -> rusqlite::Result<Vec<RoleRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, display_name, built_in, created_at
             FROM roles ORDER BY built_in DESC, name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RoleRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                display_name: row.get(2)?,
                built_in: row.get::<_, i32>(3)? != 0,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_role_by_name(&self, name: &str) -> rusqlite::Result<Option<RoleRecord>> {
        self.conn
            .query_row(
                "SELECT id, name, display_name, built_in, created_at
                 FROM roles WHERE name = ?1",
                params![name],
                |row| {
                    Ok(RoleRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        display_name: row.get(2)?,
                        built_in: row.get::<_, i32>(3)? != 0,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    pub fn get_permissions_for_role(
        &self,
        role_name: &str,
    ) -> rusqlite::Result<HashSet<Permission>> {
        let mut stmt = self.conn.prepare(
            "SELECT rp.permission FROM role_permissions rp
             JOIN roles r ON rp.role_id = r.id
             WHERE r.name = ?1",
        )?;
        let rows = stmt.query_map(params![role_name], |row| row.get::<_, String>(0))?;
        let mut perms = HashSet::new();
        for row in rows {
            let perm_str = row?;
            match Permission::from_slug(&perm_str) {
                Some(p) => {
                    perms.insert(p);
                }
                None => {
                    tracing::warn!(
                        role = role_name,
                        permission = perm_str.as_str(),
                        "unknown permission in role_permissions table — skipped"
                    );
                }
            }
        }
        Ok(perms)
    }

    #[allow(dead_code)]
    pub fn get_role_with_permissions(
        &self,
        role_name: &str,
    ) -> rusqlite::Result<Option<RoleWithPermissions>> {
        let Some(role) = self.get_role_by_name(role_name)? else {
            return Ok(None);
        };
        let permissions = self.get_permissions_for_role(role_name)?;
        Ok(Some(RoleWithPermissions { role, permissions }))
    }

    pub fn list_roles_with_permissions(&self) -> rusqlite::Result<Vec<RoleWithPermissions>> {
        let roles = self.list_roles()?;
        let mut result = Vec::with_capacity(roles.len());
        for role in roles {
            let permissions = self.get_permissions_for_role(&role.name)?;
            result.push(RoleWithPermissions { role, permissions });
        }
        Ok(result)
    }

    /// Create a custom role. Wrapped in a transaction for atomicity.
    pub fn create_role(
        &self,
        name: &str,
        display_name: &str,
        permissions: &[Permission],
    ) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO roles (name, display_name, built_in) VALUES (?1, ?2, 0)",
            params![name, display_name],
        )?;
        let role_id = self.conn.last_insert_rowid();
        let mut stmt =
            tx.prepare("INSERT INTO role_permissions (role_id, permission) VALUES (?1, ?2)")?;
        for perm in permissions {
            stmt.execute(params![role_id, perm.as_str()])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(role_id)
    }

    /// Update permissions for a role. Rejects the admin role (immutable).
    /// Wrapped in a transaction for atomicity.
    pub fn update_role_permissions(
        &self,
        role_name: &str,
        permissions: &[Permission],
    ) -> rusqlite::Result<usize> {
        let role = self
            .get_role_by_name(role_name)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        if role.name == "admin" {
            return Ok(0); // Admin role permissions are immutable
        }

        // Guard: don't remove users.manage if it would leave zero enabled users with it
        let old_perms = self.get_permissions_for_role(role_name)?;
        let new_has_manage = permissions.contains(&Permission::UsersManage);
        let old_has_manage = old_perms.contains(&Permission::UsersManage);
        if old_has_manage && !new_has_manage {
            // Count users on THIS role
            let users_on_role: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM users WHERE role = ?1 AND disabled = 0",
                params![role_name],
                |row| row.get(0),
            )?;
            if users_on_role > 0 {
                // Check if OTHER users (not on this role) still have users.manage
                let others_with_manage: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM users u
                     JOIN roles r ON u.role = r.name
                     JOIN role_permissions rp ON r.id = rp.role_id
                     WHERE rp.permission = 'users.manage' AND u.disabled = 0 AND u.role != ?1",
                    params![role_name],
                    |row| row.get(0),
                )?;
                if others_with_manage == 0 {
                    return Ok(0);
                }
            }
        }

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM role_permissions WHERE role_id = ?1",
            params![role.id],
        )?;
        let mut stmt =
            tx.prepare("INSERT INTO role_permissions (role_id, permission) VALUES (?1, ?2)")?;
        for perm in permissions {
            stmt.execute(params![role.id, perm.as_str()])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(1)
    }

    #[allow(dead_code)]
    pub fn update_role_display_name(
        &self,
        role_name: &str,
        display_name: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE roles SET display_name = ?1 WHERE name = ?2 AND built_in = 0",
            params![display_name, role_name],
        )
    }

    /// Delete a custom role. Returns 0 if built-in or has assigned users.
    pub fn delete_role(&self, role_name: &str) -> rusqlite::Result<DeleteRoleResult> {
        let tx = self.conn.unchecked_transaction()?;
        let Some(role) = self.get_role_by_name(role_name)? else {
            return Ok(DeleteRoleResult::NotFound);
        };
        if role.built_in {
            return Ok(DeleteRoleResult::BuiltIn);
        }
        let user_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = ?1",
            params![role_name],
            |row| row.get(0),
        )?;
        if user_count > 0 {
            return Ok(DeleteRoleResult::HasUsers(user_count));
        }
        self.conn.execute(
            "DELETE FROM roles WHERE name = ?1 AND built_in = 0",
            params![role_name],
        )?;
        tx.commit()?;
        Ok(DeleteRoleResult::Deleted)
    }

    /// Count enabled users whose role has a specific permission.
    /// Used for permission-based last-admin guards.
    pub fn count_users_with_permission(
        &self,
        permission: &str,
        exclude_user_id: Option<i64>,
    ) -> rusqlite::Result<i64> {
        if let Some(exclude_id) = exclude_user_id {
            self.conn.query_row(
                "SELECT COUNT(*) FROM users u
                 JOIN roles r ON u.role = r.name
                 JOIN role_permissions rp ON r.id = rp.role_id
                 WHERE rp.permission = ?1 AND u.disabled = 0 AND u.id != ?2",
                params![permission, exclude_id],
                |row| row.get(0),
            )
        } else {
            self.conn.query_row(
                "SELECT COUNT(*) FROM users u
                 JOIN roles r ON u.role = r.name
                 JOIN role_permissions rp ON r.id = rp.role_id
                 WHERE rp.permission = ?1 AND u.disabled = 0",
                params![permission],
                |row| row.get(0),
            )
        }
    }

    /// Single-query user + role + permissions load for auth middleware.
    /// Returns (User, role_display_name, HashSet<Permission>).
    pub fn get_user_with_permissions(
        &self,
        user_id: i64,
    ) -> rusqlite::Result<Option<(super::users::User, String, HashSet<Permission>)>> {
        let user = self.get_user_by_id(user_id)?;
        let Some(ref u) = user else {
            return Ok(None);
        };
        let role_display = self
            .get_role_by_name(&u.role)?
            .map(|r| r.display_name)
            .unwrap_or_else(|| {
                tracing::warn!(
                    user_id = u.id,
                    role = u.role.as_str(),
                    "user has role not found in roles table"
                );
                u.role.clone()
            });
        let permissions = self.get_permissions_for_role(&u.role)?;
        Ok(user.map(|u| (u, role_display, permissions)))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteRoleResult {
    Deleted,
    NotFound,
    BuiltIn,
    HasUsers(i64),
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn list_seeded_roles() {
        let db = Database::open_in_memory().unwrap();
        let roles = db.list_roles().unwrap();
        assert_eq!(roles.len(), 3);
        let names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"admin"));
        assert!(names.contains(&"moderator"));
        assert!(names.contains(&"player"));
    }

    #[test]
    fn admin_has_all_permissions() {
        let db = Database::open_in_memory().unwrap();
        let perms = db.get_permissions_for_role("admin").unwrap();
        assert_eq!(perms.len(), Permission::ALL.len());
        for p in Permission::ALL {
            assert!(perms.contains(p), "admin missing {:?}", p);
        }
    }

    #[test]
    fn moderator_permissions() {
        let db = Database::open_in_memory().unwrap();
        let perms = db.get_permissions_for_role("moderator").unwrap();
        assert!(perms.contains(&Permission::ModsInstall));
        assert!(perms.contains(&Permission::ServerControl));
        assert!(!perms.contains(&Permission::UsersManage));
        assert!(!perms.contains(&Permission::SettingsManage));
    }

    #[test]
    fn player_has_no_permissions() {
        let db = Database::open_in_memory().unwrap();
        let perms = db.get_permissions_for_role("player").unwrap();
        assert!(perms.is_empty());
    }

    #[test]
    fn create_custom_role() {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .create_role(
                "curator",
                "Mod Curator",
                &[Permission::ModsInstall, Permission::ModsUpdate],
            )
            .unwrap();
        assert!(id > 0);
        let role = db.get_role_with_permissions("curator").unwrap().unwrap();
        assert_eq!(role.role.display_name, "Mod Curator");
        assert!(!role.role.built_in);
        assert_eq!(role.permissions.len(), 2);
        assert!(role.permissions.contains(&Permission::ModsInstall));
    }

    #[test]
    fn update_role_permissions_on_custom() {
        let db = Database::open_in_memory().unwrap();
        db.create_role("tester", "Tester", &[Permission::ModsInstall])
            .unwrap();
        let affected = db
            .update_role_permissions(
                "tester",
                &[Permission::ServerLogs, Permission::ServerMetrics],
            )
            .unwrap();
        assert_eq!(affected, 1);
        let perms = db.get_permissions_for_role("tester").unwrap();
        assert_eq!(perms.len(), 2);
        assert!(perms.contains(&Permission::ServerLogs));
        assert!(!perms.contains(&Permission::ModsInstall));
    }

    #[test]
    fn update_role_permissions_rejects_admin() {
        let db = Database::open_in_memory().unwrap();
        let affected = db
            .update_role_permissions("admin", &[Permission::ModsInstall])
            .unwrap();
        assert_eq!(affected, 0);
        // Admin still has all permissions
        let perms = db.get_permissions_for_role("admin").unwrap();
        assert_eq!(perms.len(), Permission::ALL.len());
    }

    #[test]
    fn cannot_delete_builtin_role() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.delete_role("admin").unwrap(), DeleteRoleResult::BuiltIn);
    }

    #[test]
    fn delete_custom_role_with_no_users() {
        let db = Database::open_in_memory().unwrap();
        db.create_role("temp", "Temporary", &[]).unwrap();
        assert_eq!(db.delete_role("temp").unwrap(), DeleteRoleResult::Deleted);
        assert!(db.get_role_by_name("temp").unwrap().is_none());
    }

    #[test]
    fn cannot_delete_role_with_assigned_users() {
        let db = Database::open_in_memory().unwrap();
        db.create_role("custom", "Custom", &[]).unwrap();
        db.insert_user("testuser", None, Some("hash"), "custom")
            .unwrap();
        assert_eq!(
            db.delete_role("custom").unwrap(),
            DeleteRoleResult::HasUsers(1)
        );
    }

    #[test]
    fn permission_roundtrip() {
        for p in Permission::ALL {
            let s = p.as_str();
            let back = Permission::from_slug(s).unwrap();
            assert_eq!(*p, back);
        }
    }

    #[test]
    fn validate_role_name_rejects_invalid() {
        assert!(validate_role_name("").is_err());
        assert!(validate_role_name("Admin").is_err()); // uppercase
        assert!(validate_role_name("my role").is_err()); // space
        assert!(validate_role_name("admin").is_err()); // built-in
        assert!(validate_role_name("-bad").is_err()); // starts with hyphen
        assert!(validate_role_name(&"a".repeat(33)).is_err()); // too long
        assert!(validate_role_name("my/role").is_err()); // slash
    }

    #[test]
    fn validate_role_name_accepts_valid() {
        assert!(validate_role_name("curator").is_ok());
        assert!(validate_role_name("mod-curator").is_ok());
        assert!(validate_role_name("role123").is_ok());
    }
}
