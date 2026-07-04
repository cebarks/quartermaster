use rusqlite::{params, OptionalExtension};
use std::fmt;

use super::Database;

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub spt_profile_id: Option<String>,
    pub password_hash: Option<String>,
    pub role: String,
    pub disabled: bool,
    pub created_at: String,
    pub password_changed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteUserResult {
    Deleted,
    NotFound,
    LastAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteInviteResult {
    Deleted,
    NotFound,
    AlreadyUsed,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct InviteCode {
    pub id: i64,
    pub code: String,
    pub created_by: Option<i64>,
    pub used_by: Option<i64>,
    pub created_at: String,
    pub used_at: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InviteCodeWithUsers {
    pub invite: InviteCode,
    pub created_by_username: Option<String>,
    pub used_by_username: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct ResetToken {
    pub id: i64,
    pub user_id: i64,
    pub token: String,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueAction {
    Install,
    Update,
    Remove,
}

impl QueueAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueueAction::Install => "install",
            QueueAction::Update => "update",
            QueueAction::Remove => "remove",
        }
    }
}

impl TryFrom<String> for QueueAction {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "install" => Ok(QueueAction::Install),
            "update" => Ok(QueueAction::Update),
            "remove" => Ok(QueueAction::Remove),
            other => Err(format!("unknown queue action: {other}")),
        }
    }
}

impl fmt::Display for QueueAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct PendingOperation {
    pub id: i64,
    pub action: QueueAction,
    pub forge_mod_id: Option<i64>, // NULL for addon ops
    pub forge_version_id: Option<i64>,
    pub mod_name: String, // addon name for addon ops
    pub metadata: Option<String>,
    pub queued_at: String,
    pub queued_by: Option<String>,
    pub item_type: String,           // "mod" or "addon"
    pub forge_addon_id: Option<i64>, // set for addon ops
}

impl Database {
    // ── User CRUD ─────────────────────────────────────────────────────

    pub fn insert_user(
        &self,
        username: &str,
        spt_profile_id: Option<&str>,
        password_hash: Option<&str>,
        role: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO users (username, spt_profile_id, password_hash, role) VALUES (?1, ?2, ?3, ?4)",
            params![username, spt_profile_id, password_hash, role],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
                 FROM users WHERE username = ?1",
                params![username],
                row_to_user,
            )
            .optional()
    }

    pub fn list_users(&self) -> rusqlite::Result<Vec<User>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
             FROM users ORDER BY username",
        )?;
        let rows = stmt.query_map([], row_to_user)?;
        rows.collect()
    }

    pub fn has_user_manager(&self) -> rusqlite::Result<bool> {
        Ok(self
            .count_users_with_permission(crate::db::rbac::Permission::UsersManage.as_str(), None)?
            > 0)
    }

    pub fn get_user_by_id(&self, id: i64) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
                 FROM users WHERE id = ?1",
                params![id],
                row_to_user,
            )
            .optional()
    }

    pub fn get_user_by_spt_profile_id(&self, profile_id: &str) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
                 FROM users WHERE spt_profile_id = ?1",
                params![profile_id],
                row_to_user,
            )
            .optional()
    }

    pub fn update_user_role(&self, user_id: i64, new_role: &str) -> rusqlite::Result<usize> {
        // Permission-based last-admin guard: if current user has a role with
        // users.manage, ensure at least one OTHER enabled user also has it.
        let current_has_manage: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM users u
             JOIN roles r ON u.role = r.name
             JOIN role_permissions rp ON r.id = rp.role_id
             WHERE u.id = ?1 AND rp.permission = 'users.manage' AND u.disabled = 0",
            params![user_id],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        )?;

        if current_has_manage {
            // Check the NEW role still has users.manage, OR other users do
            let new_role_has_manage: bool = self.conn.query_row(
                "SELECT COUNT(*) FROM roles r
                 JOIN role_permissions rp ON r.id = rp.role_id
                 WHERE r.name = ?1 AND rp.permission = 'users.manage'",
                params![new_role],
                |row| Ok(row.get::<_, i64>(0)? > 0),
            )?;

            if !new_role_has_manage {
                let others_with_manage =
                    self.count_users_with_permission("users.manage", Some(user_id))?;
                if others_with_manage == 0 {
                    return Ok(0); // Would remove last user with users.manage
                }
            }
        }

        self.conn.execute(
            "UPDATE users SET role = ?1 WHERE id = ?2",
            params![new_role, user_id],
        )
    }

    pub fn set_user_disabled(&self, user_id: i64, disabled: bool) -> rusqlite::Result<usize> {
        if disabled {
            let others = self.count_users_with_permission("users.manage", Some(user_id))?;
            if others == 0 {
                // Check if THIS user has users.manage — if so, can't disable last one
                let has_manage: bool = self.conn.query_row(
                    "SELECT COUNT(*) FROM users u
                     JOIN roles r ON u.role = r.name
                     JOIN role_permissions rp ON r.id = rp.role_id
                     WHERE u.id = ?1 AND rp.permission = 'users.manage' AND u.disabled = 0",
                    params![user_id],
                    |row| Ok(row.get::<_, i64>(0)? > 0),
                )?;
                if has_manage {
                    return Ok(0); // Would disable last user with users.manage
                }
            }
            self.conn.execute(
                "UPDATE users SET disabled = 1 WHERE id = ?1",
                params![user_id],
            )
        } else {
            self.conn.execute(
                "UPDATE users SET disabled = 0 WHERE id = ?1",
                params![user_id],
            )
        }
    }

    pub fn update_user_password(
        &self,
        user_id: i64,
        password_hash: &str,
    ) -> rusqlite::Result<usize> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE users SET password_hash = ?1, password_changed_at = ?3 WHERE id = ?2",
            params![password_hash, user_id, now],
        )
    }

    pub fn update_user_spt_profile_id(
        &self,
        user_id: i64,
        spt_profile_id: Option<&str>,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE users SET spt_profile_id = ?1 WHERE id = ?2",
            params![spt_profile_id, user_id],
        )
    }

    #[cfg(test)]
    pub fn count_admins(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0",
            [],
            |row| row.get(0),
        )
    }

    pub fn delete_user(&self, user_id: i64) -> rusqlite::Result<DeleteUserResult> {
        let Some(user) = self.get_user_by_id(user_id)? else {
            return Ok(DeleteUserResult::NotFound);
        };

        if !user.disabled {
            let others = self.count_users_with_permission("users.manage", Some(user_id))?;
            let has_manage: bool = self.conn.query_row(
                "SELECT COUNT(*) FROM users u
                 JOIN roles r ON u.role = r.name
                 JOIN role_permissions rp ON r.id = rp.role_id
                 WHERE u.id = ?1 AND rp.permission = 'users.manage' AND u.disabled = 0",
                params![user_id],
                |row| Ok(row.get::<_, i64>(0)? > 0),
            )?;
            if has_manage && others == 0 {
                return Ok(DeleteUserResult::LastAdmin);
            }
        }

        self.conn
            .execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
        Ok(DeleteUserResult::Deleted)
    }

    // ── Invite CRUD ───────────────────────────────────────────────────

    pub fn create_invite(
        &self,
        code: &str,
        created_by: Option<i64>,
        expires_at: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO invite_codes (code, created_by, expires_at) VALUES (?1, ?2, ?3)",
            params![code, created_by, expires_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_invite(&self, code: &str) -> rusqlite::Result<Option<InviteCode>> {
        self.conn
            .query_row(
                "SELECT id, code, created_by, used_by, created_at, used_at, expires_at
                 FROM invite_codes WHERE code = ?1",
                params![code],
                row_to_invite_code,
            )
            .optional()
    }

    /// Attempt to use an invite code. Returns the number of rows affected (1 if
    /// successful, 0 if the code was already used or expired).
    pub fn use_invite(&self, code: &str, user_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE invite_codes SET used_by = ?1, used_at = datetime('now')
             WHERE code = ?2 AND used_by IS NULL
             AND (expires_at IS NULL OR expires_at > datetime('now'))",
            params![user_id, code],
        )
    }

    pub fn list_invite_codes(&self) -> rusqlite::Result<Vec<InviteCodeWithUsers>> {
        let mut stmt = self.conn.prepare(
            "SELECT ic.id, ic.code, ic.created_by, ic.used_by, ic.created_at, ic.used_at, ic.expires_at,
                    u1.username AS created_by_username,
                    u2.username AS used_by_username
             FROM invite_codes ic
             LEFT JOIN users u1 ON ic.created_by = u1.id
             LEFT JOIN users u2 ON ic.used_by = u2.id
             ORDER BY ic.created_at DESC, ic.id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(InviteCodeWithUsers {
                invite: InviteCode {
                    id: row.get(0)?,
                    code: row.get(1)?,
                    created_by: row.get(2)?,
                    used_by: row.get(3)?,
                    created_at: row.get(4)?,
                    used_at: row.get(5)?,
                    expires_at: row.get(6)?,
                },
                created_by_username: row.get(7)?,
                used_by_username: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_invite(&self, invite_id: i64) -> rusqlite::Result<DeleteInviteResult> {
        let affected = self.conn.execute(
            "DELETE FROM invite_codes WHERE id = ?1 AND used_by IS NULL",
            params![invite_id],
        )?;
        if affected > 0 {
            return Ok(DeleteInviteResult::Deleted);
        }
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM invite_codes WHERE id = ?1",
            params![invite_id],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        )?;
        if exists {
            Ok(DeleteInviteResult::AlreadyUsed)
        } else {
            Ok(DeleteInviteResult::NotFound)
        }
    }

    // ── Password Reset Token CRUD ────────────────────────────────────

    pub fn create_reset_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "DELETE FROM password_reset_tokens WHERE user_id = ?1",
            params![user_id],
        )?;
        self.conn.execute(
            "INSERT INTO password_reset_tokens (user_id, token, expires_at) VALUES (?1, ?2, ?3)",
            params![user_id, token, expires_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_reset_token(&self, token: &str) -> rusqlite::Result<Option<ResetToken>> {
        self.conn
            .query_row(
                "SELECT id, user_id, token, expires_at, created_at
                 FROM password_reset_tokens WHERE token = ?1",
                params![token],
                |row| {
                    Ok(ResetToken {
                        id: row.get(0)?,
                        user_id: row.get(1)?,
                        token: row.get(2)?,
                        expires_at: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    pub fn delete_reset_token(&self, token: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM password_reset_tokens WHERE token = ?1",
            params![token],
        )
    }

    // ── Pending Operations CRUD ───────────────────────────────────────

    pub fn insert_pending_op(
        &self,
        action: QueueAction,
        forge_mod_id: i64,
        forge_version_id: Option<i64>,
        mod_name: &str,
        metadata: Option<&str>,
        queued_by: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_operations (action, forge_mod_id, forge_version_id, mod_name, metadata, queued_by, item_type, forge_addon_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'mod', NULL)",
            params![action.as_str(), forge_mod_id, forge_version_id, mod_name, metadata, queued_by],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)] // used in Task 5
    pub fn insert_pending_addon_op(
        &self,
        action: QueueAction,
        forge_addon_id: i64,
        forge_version_id: Option<i64>,
        addon_name: &str,
        metadata: Option<&str>,
        queued_by: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_operations (action, forge_mod_id, forge_version_id, mod_name, metadata, queued_by, item_type, forge_addon_id)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'addon', ?6)",
            params![action.as_str(), forge_version_id, addon_name, metadata, queued_by, forge_addon_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn has_pending_op(&self, forge_mod_id: i64, action: QueueAction) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pending_operations WHERE forge_mod_id = ?1 AND action = ?2 AND item_type = 'mod'",
            params![forge_mod_id, action.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    #[allow(dead_code)] // used in Task 5
    pub fn has_pending_addon_op(
        &self,
        forge_addon_id: i64,
        action: QueueAction,
    ) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pending_operations WHERE forge_addon_id = ?1 AND action = ?2 AND item_type = 'addon'",
            params![forge_addon_id, action.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn list_pending_ops(&self) -> rusqlite::Result<Vec<PendingOperation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by, item_type, forge_addon_id
             FROM pending_operations ORDER BY queued_at",
        )?;
        let rows = stmt.query_map([], row_to_pending_op)?;
        rows.collect()
    }

    pub fn delete_pending_op(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM pending_operations WHERE id = ?1", params![id])
    }

    #[cfg(test)]
    pub fn clear_pending_ops(&self) -> rusqlite::Result<usize> {
        self.conn.execute("DELETE FROM pending_operations", [])
    }
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        spt_profile_id: row.get(2)?,
        password_hash: row.get(3)?,
        role: row.get(4)?,
        disabled: row.get(5)?,
        created_at: row.get(6)?,
        password_changed_at: row.get(7)?,
    })
}

fn row_to_invite_code(row: &rusqlite::Row<'_>) -> rusqlite::Result<InviteCode> {
    Ok(InviteCode {
        id: row.get(0)?,
        code: row.get(1)?,
        created_by: row.get(2)?,
        used_by: row.get(3)?,
        created_at: row.get(4)?,
        used_at: row.get(5)?,
        expires_at: row.get(6)?,
    })
}

fn row_to_pending_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingOperation> {
    let action_str: String = row.get(1)?;
    let action = QueueAction::try_from(action_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;
    Ok(PendingOperation {
        id: row.get(0)?,
        action,
        forge_mod_id: row.get(2)?,
        forge_version_id: row.get(3)?,
        mod_name: row.get(4)?,
        metadata: row.get(5)?,
        queued_at: row.get(6)?,
        queued_by: row.get(7)?,
        item_type: row.get(8)?,
        forge_addon_id: row.get(9)?,
    })
}
