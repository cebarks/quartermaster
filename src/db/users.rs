use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Moderator,
    Player,
}

impl Role {
    pub fn can_manage_mods(&self) -> bool {
        matches!(self, Role::Admin | Role::Moderator)
    }

    pub fn can_control_server(&self) -> bool {
        matches!(self, Role::Admin | Role::Moderator)
    }

    pub fn can_manage_queue(&self) -> bool {
        matches!(self, Role::Admin | Role::Moderator)
    }

    pub fn can_manage_users(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Moderator => "moderator",
            Role::Player => "player",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Role::Admin => write!(f, "Admin"),
            Role::Moderator => write!(f, "Moderator"),
            Role::Player => write!(f, "Player"),
        }
    }
}

impl TryFrom<String> for Role {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "admin" => Ok(Role::Admin),
            "moderator" => Ok(Role::Moderator),
            "player" => Ok(Role::Player),
            other => Err(format!("unknown role: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub spt_profile_id: Option<String>,
    pub password_hash: Option<String>,
    pub role: Role,
    pub disabled: bool,
    pub stash_public: bool,
    pub created_at: String,
    pub password_changed_at: Option<String>,
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

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct PendingOperation {
    pub id: i64,
    pub action: String,
    pub forge_mod_id: i64,
    pub forge_version_id: Option<i64>,
    pub mod_name: String,
    pub metadata: Option<String>,
    pub queued_at: String,
    pub queued_by: Option<String>,
}

impl Database {
    // ── User CRUD ─────────────────────────────────────────────────────

    pub fn insert_user(
        &self,
        username: &str,
        spt_profile_id: Option<&str>,
        password_hash: Option<&str>,
        role: Role,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO users (username, spt_profile_id, password_hash, role) VALUES (?1, ?2, ?3, ?4)",
            params![username, spt_profile_id, password_hash, role.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, disabled, stash_public, created_at, password_changed_at
                 FROM users WHERE username = ?1",
                params![username],
                row_to_user,
            )
            .optional()
    }

    pub fn list_users(&self) -> rusqlite::Result<Vec<User>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, username, spt_profile_id, password_hash, role, disabled, stash_public, created_at, password_changed_at
             FROM users ORDER BY username",
        )?;
        let rows = stmt.query_map([], row_to_user)?;
        rows.collect()
    }

    pub fn admin_exists(&self) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = ?1",
            params![Role::Admin.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn get_user_by_id(&self, id: i64) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, disabled, stash_public, created_at, password_changed_at
                 FROM users WHERE id = ?1",
                params![id],
                row_to_user,
            )
            .optional()
    }

    #[allow(dead_code)] // Used by Task 5-7 (proxy handlers)
    pub fn get_user_by_spt_profile_id(&self, profile_id: &str) -> rusqlite::Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, username, spt_profile_id, password_hash, role, disabled, stash_public, created_at, password_changed_at
                 FROM users WHERE spt_profile_id = ?1",
                params![profile_id],
                row_to_user,
            )
            .optional()
    }

    pub fn update_user_role(&self, user_id: i64, new_role: Role) -> rusqlite::Result<usize> {
        let new_role_str = new_role.as_str();
        self.conn.execute(
            "UPDATE users SET role = ?1
             WHERE id = ?2
             AND (?1 = 'admin' OR ?1 != 'admin' AND (
                 role != 'admin'
                 OR (SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0 AND id != ?2) > 0
             ))",
            params![new_role_str, user_id],
        )
    }

    pub fn set_user_disabled(&self, user_id: i64, disabled: bool) -> rusqlite::Result<usize> {
        if disabled {
            self.conn.execute(
                "UPDATE users SET disabled = 1
                 WHERE id = ?1
                 AND (role != 'admin'
                      OR (SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0 AND id != ?1) > 0)",
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

    pub fn set_stash_public(&self, user_id: i64, public: bool) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE users SET stash_public = ?1 WHERE id = ?2",
            params![public as i32, user_id],
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

    /// Update an invite code's user_id unconditionally (no IS NULL guard).
    /// Used after creating a user to replace the temporary 0 with the real user_id.
    pub fn update_invite_user(&self, code: &str, user_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE invite_codes SET used_by = ?1 WHERE code = ?2",
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
        action: &str,
        forge_mod_id: i64,
        forge_version_id: Option<i64>,
        mod_name: &str,
        metadata: Option<&str>,
        queued_by: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_operations (action, forge_mod_id, forge_version_id, mod_name, metadata, queued_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![action, forge_mod_id, forge_version_id, mod_name, metadata, queued_by],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_pending_ops(&self) -> rusqlite::Result<Vec<PendingOperation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by
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
    let role_str: String = row.get(4)?;
    let role = Role::try_from(role_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        spt_profile_id: row.get(2)?,
        password_hash: row.get(3)?,
        role,
        disabled: row.get(5)?,
        stash_public: row.get(6)?,
        created_at: row.get(7)?,
        password_changed_at: row.get(8)?,
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
    Ok(PendingOperation {
        id: row.get(0)?,
        action: row.get(1)?,
        forge_mod_id: row.get(2)?,
        forge_version_id: row.get(3)?,
        mod_name: row.get(4)?,
        metadata: row.get(5)?,
        queued_at: row.get(6)?,
        queued_by: row.get(7)?,
    })
}
