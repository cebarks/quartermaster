# Admin User Management UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a tabbed admin panel (`/admin`) with Users and Invites tabs, user role management, account disabling, password reset via token, and invite code creation from the web UI.

**Architecture:** HTMX-driven tab panel at `/admin` behind a scoped `from_fn` middleware that enforces `can_manage_users()`. Tab content loads via partial swaps. Inline user row updates via HTMX POST → partial re-render. Password reset uses CSPRNG tokens with 24h expiry, stored in a new DB table. SPT profile stats are parsed from game JSON files and displayed alongside user data. Shared invite utilities extracted from CLI to a new `src/invite.rs` module.

**Tech Stack:** Rust, actix-web 4, actix-session, Askama templates, HTMX, rusqlite (SQLite WAL), Argon2 (password hashing), subtle (constant-time comparison), OsRng (CSPRNG token generation)

## Global Constraints

- All admin routes require `can_manage_users()` enforced via scoped `from_fn` middleware — not per-handler checks.
- All POST handlers validate CSRF tokens via `csrf::validate_token()` before processing.
- All HTMX partial templates include CSRF tokens as hidden form inputs.
- Self-protection: handlers reject requests where `user_id == session_user.user_id` with 403.
- Last-admin guard: atomic SQL (subquery in UPDATE WHERE), not check-then-update.
- Password reset tokens generated with CSPRNG (`OsRng`), compared with `subtle::ConstantTimeEq`.
- Identical error messages for all reset token failure modes (prevents enumeration).
- All action buttons include `hx-disabled-elt="this"` to prevent double-submission.
- Reset password routes rate-limited via Governor middleware (5/min/IP).

---

### Task 1: Migration, DB functions, and shared invite module

**Files:**
- Create: `migrations/006_password_reset_tokens.sql`
- Create: `src/invite.rs`
- Modify: `src/db/schema.rs`
- Modify: `src/db/users.rs`
- Modify: `src/main.rs`
- Modify: `src/cli/invite.rs`
- Modify: `Cargo.toml`
- Test: `src/db/tests.rs`

**Interfaces:**
- Consumes: `Database` struct, `Role` enum, `InviteCode` struct, `row_to_user`, `row_to_invite_code` (all in `src/db/users.rs`)
- Produces:
  - `Database::update_user_role(user_id: i64, new_role: Role) -> rusqlite::Result<usize>` — atomic with last-admin guard, returns rows affected (0 = guard blocked)
  - `Database::set_user_disabled(user_id: i64, disabled: bool) -> rusqlite::Result<usize>` — atomic with last-admin guard when disabling an admin, returns rows affected
  - `Database::update_user_password(user_id: i64, password_hash: &str) -> rusqlite::Result<usize>` — also sets `password_changed_at = datetime('now')`
  - `Database::count_admins() -> rusqlite::Result<i64>`
  - `Database::list_invite_codes() -> rusqlite::Result<Vec<InviteCodeWithUsers>>`
  - `Database::create_reset_token(user_id: i64, token: &str, expires_at: &str) -> rusqlite::Result<i64>` — deletes existing token for user first
  - `Database::get_reset_token(token: &str) -> rusqlite::Result<Option<ResetToken>>`
  - `Database::delete_reset_token(token: &str) -> rusqlite::Result<usize>`
  - `ResetToken { id: i64, user_id: i64, token: String, expires_at: String, created_at: String }`
  - `InviteCodeWithUsers { invite: InviteCode, created_by_username: Option<String>, used_by_username: Option<String> }`
  - `crate::invite::generate_invite_code() -> String`
  - `crate::invite::parse_expiry(input: &str) -> Result<String>`

- [ ] **Step 1: Add `subtle` dependency to Cargo.toml**

In `Cargo.toml`, add under `[dependencies]` after the `rand` line:

```toml
subtle = "2"
```

- [ ] **Step 2: Create migration file**

Create `migrations/006_password_reset_tokens.sql`:

```sql
CREATE TABLE password_reset_tokens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token       TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE users ADD COLUMN password_changed_at TEXT;
```

- [ ] **Step 3: Register migration in schema.rs**

In `src/db/schema.rs`, add after the `MIGRATION_005` const:

```rust
const MIGRATION_006: &str = include_str!("../../migrations/006_password_reset_tokens.sql");
```

And add after the `if current_version < 5` block:

```rust
    if current_version < 6 {
        conn.execute_batch(MIGRATION_006)?;
        conn.pragma_update(None, "user_version", 6)?;
    }
```

- [ ] **Step 4: Create shared invite module**

Create `src/invite.rs`:

```rust
use anyhow::{bail, Result};
use rand::distr::Alphanumeric;
use rand::Rng;

pub fn generate_invite_code() -> String {
    let suffix: String = rand::rng()
        .sample_iter(Alphanumeric)
        .take(10)
        .map(char::from)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("quma-{suffix}")
}

pub fn parse_expiry(input: &str) -> Result<String> {
    let input = input.trim();
    let (num_str, unit) = if let Some(stripped) = input.strip_suffix('d') {
        (stripped, "days")
    } else if let Some(stripped) = input.strip_suffix('h') {
        (stripped, "hours")
    } else if let Some(stripped) = input.strip_suffix('m') {
        (stripped, "minutes")
    } else {
        bail!("invalid expiry format: use e.g. '24h', '7d', '30m'");
    };

    let num: i64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number in expiry: '{num_str}'"))?;

    if num <= 0 {
        bail!("expiry must be positive");
    }

    let duration = match unit {
        "days" => chrono::Duration::days(num),
        "hours" => chrono::Duration::hours(num),
        "minutes" => chrono::Duration::minutes(num),
        _ => unreachable!(),
    };

    let expires_at = chrono::Utc::now() + duration;
    Ok(expires_at.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_code_format() {
        let code = generate_invite_code();
        assert!(code.starts_with("quma-"), "code should start with 'quma-'");
        assert_eq!(code.len(), 15, "code should be 15 chars: 'quma-' + 10");
        let suffix = &code[5..];
        assert!(
            suffix.chars().all(|c| c.is_ascii_alphanumeric()),
            "suffix should be alphanumeric"
        );
    }

    #[test]
    fn parse_expiry_hours() {
        let result = parse_expiry("24h").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_hours() >= 23 && diff.num_hours() <= 24);
    }

    #[test]
    fn parse_expiry_days() {
        let result = parse_expiry("7d").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_days() >= 6 && diff.num_days() <= 7);
    }

    #[test]
    fn parse_expiry_minutes() {
        let result = parse_expiry("30m").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_minutes() >= 29 && diff.num_minutes() <= 30);
    }

    #[test]
    fn parse_expiry_invalid() {
        assert!(parse_expiry("abc").is_err());
        assert!(parse_expiry("0h").is_err());
        assert!(parse_expiry("-5d").is_err());
    }
}
```

- [ ] **Step 5: Register invite module in main.rs and update CLI**

In `src/main.rs`, add after `mod health;`:

```rust
mod invite;
```

In `src/cli/invite.rs`, replace the `generate_invite_code` function and `parse_expiry` function with imports. The file should become:

```rust
use anyhow::Result;

use super::common::CliContext;
use crate::invite::{generate_invite_code, parse_expiry};

pub fn run(expires: Option<&str>, ctx: &CliContext) -> Result<()> {
    let code = generate_invite_code();

    let expires_at = match expires {
        Some(exp) => Some(parse_expiry(exp)?),
        None => None,
    };

    ctx.db
        .create_invite(&code, None, expires_at.as_deref())
        .map_err(|e| anyhow::anyhow!("failed to create invite: {e}"))?;

    let display_host = if ctx.config.web_bind == "0.0.0.0" {
        "localhost"
    } else {
        &ctx.config.web_bind
    };

    println!("Invite code: {code}");
    println!(
        "Registration URL: http://{display_host}:{}/register?code={code}",
        ctx.config.web_port
    );

    if let Some(ref exp) = expires_at {
        println!("Expires: {exp}");
    } else {
        println!("Expires: never");
    }

    Ok(())
}
```

Remove the `#[cfg(test)] mod tests` block from `src/cli/invite.rs` — those tests now live in `src/invite.rs`.

- [ ] **Step 6: Write failing tests for new DB functions**

Add the following tests to `src/db/tests.rs`:

```rust
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
```

- [ ] **Step 7: Run tests to verify they fail**

Run: `cargo test -p quartermaster -- update_user_role set_user_disabled update_user_password count_admins list_invite_codes reset_token password_reset_tokens_table`
Expected: compile errors — functions and structs don't exist yet.

- [ ] **Step 8: Add new structs to `src/db/users.rs`**

After the `InviteCode` struct, add:

```rust
#[derive(Debug, Clone)]
pub struct InviteCodeWithUsers {
    pub invite: InviteCode,
    pub created_by_username: Option<String>,
    pub used_by_username: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResetToken {
    pub id: i64,
    pub user_id: i64,
    pub token: String,
    pub expires_at: String,
    pub created_at: String,
}
```

- [ ] **Step 9: Implement DB functions in `src/db/users.rs`**

Add the following methods to the `impl Database` block, in the `// ── User CRUD` section after `get_user_by_id`:

```rust
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

    pub fn update_user_password(&self, user_id: i64, password_hash: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE users SET password_hash = ?1, password_changed_at = datetime('now') WHERE id = ?2",
            params![password_hash, user_id],
        )
    }

    pub fn count_admins(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0",
            [],
            |row| row.get(0),
        )
    }
```

Add in the `// ── Invite CRUD` section after `update_invite_user`:

```rust
    pub fn list_invite_codes(&self) -> rusqlite::Result<Vec<InviteCodeWithUsers>> {
        let mut stmt = self.conn.prepare(
            "SELECT ic.id, ic.code, ic.created_by, ic.used_by, ic.created_at, ic.used_at, ic.expires_at,
                    u1.username AS created_by_username,
                    u2.username AS used_by_username
             FROM invite_codes ic
             LEFT JOIN users u1 ON ic.created_by = u1.id
             LEFT JOIN users u2 ON ic.used_by = u2.id
             ORDER BY ic.created_at DESC",
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
```

Add a new section after `update_invite_user`:

```rust
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
```

- [ ] **Step 10: Run tests to verify they pass**

Run: `cargo test -p quartermaster`
Expected: all tests pass, including the new ones and existing ones.

- [ ] **Step 11: Commit**

```bash
git add migrations/006_password_reset_tokens.sql src/invite.rs src/db/schema.rs src/db/users.rs src/db/tests.rs src/main.rs src/cli/invite.rs Cargo.toml
git commit -m "feat: add DB layer for admin user management and shared invite module

Add migration 006 (password_reset_tokens table, password_changed_at column),
8 new DB functions (role update with last-admin guard, disable with guard,
password update, count admins, list invites with JOINed usernames, reset
token CRUD), and extract invite code utilities to shared src/invite.rs."
```

---

### Task 2: SPT profile stats parsing

**Files:**
- Modify: `src/spt/profiles.rs`

**Interfaces:**
- Consumes: Existing `SptProfile`, `list_profiles()`, `ProfileJson`, `ProfileInfo` from `src/spt/profiles.rs`
- Produces:
  - `pub struct SptProfileStats` — all fields `Option<T>`: `nickname: Option<String>`, `level: Option<i64>`, `side: Option<String>`, `experience: Option<i64>`, `registration_date: Option<i64>`, `raid_count: Option<i64>`, `survival_rate: Option<f64>`, `kill_count: Option<usize>`
  - `pub enum ProfileStatus` — `Found(SptProfileStats)`, `NotFound`, `ParseError`
  - `pub fn load_all_profile_stats(spt_dir: &Path) -> HashMap<String, SptProfileStats>` — returns map keyed by AID

- [ ] **Step 1: Write failing tests for profile stats parsing**

Add the following to the `#[cfg(test)] mod tests` block in `src/spt/profiles.rs`:

```rust
    fn create_full_profile(dir: &Path, aid: &str, username: &str) {
        let profiles_dir = dir.join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = serde_json::json!({
            "info": {"id": aid, "username": username},
            "characters": {
                "pmc": {
                    "Info": {
                        "Nickname": username,
                        "Level": 42,
                        "Side": "Usec",
                        "Experience": 1234567,
                        "RegistrationDate": 1700000000
                    },
                    "Stats": {
                        "Eft": {
                            "OverallCounters": {
                                "Items": [
                                    {"Key": ["Sessions", "Pmc", "Survived"], "Value": 30},
                                    {"Key": ["Sessions", "Pmc", "Died"], "Value": 10},
                                    {"Key": ["Sessions", "Pmc", "RunThrough"], "Value": 5},
                                    {"Key": ["KilledSavages"], "Value": 100}
                                ]
                            },
                            "Victims": [
                                {"Name": "Bot1"},
                                {"Name": "Bot2"},
                                {"Name": "Bot3"}
                            ]
                        }
                    }
                }
            }
        });
        std::fs::write(
            profiles_dir.join(format!("{aid}.json")),
            serde_json::to_string(&content).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_profile_stats_full() {
        let tmp = tempfile::tempdir().unwrap();
        create_full_profile(tmp.path(), "abc123", "Player1");

        let stats = load_all_profile_stats(tmp.path());
        assert_eq!(stats.len(), 1);
        let s = &stats["abc123"];
        assert_eq!(s.nickname.as_deref(), Some("Player1"));
        assert_eq!(s.level, Some(42));
        assert_eq!(s.side.as_deref(), Some("Usec"));
        assert_eq!(s.experience, Some(1234567));
        assert_eq!(s.registration_date, Some(1700000000));
        assert_eq!(s.raid_count, Some(45)); // 30 + 10 + 5
        assert!((s.survival_rate.unwrap() - 66.67).abs() < 0.1); // 30/45*100
        assert_eq!(s.kill_count, Some(3));
    }

    #[test]
    fn load_profile_stats_missing_pmc() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path().join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = r#"{"info":{"id":"abc","username":"NoCharacters"}}"#;
        std::fs::write(profiles_dir.join("abc.json"), content).unwrap();

        let stats = load_all_profile_stats(tmp.path());
        assert_eq!(stats.len(), 1);
        let s = &stats["abc"];
        assert!(s.nickname.is_none());
        assert!(s.level.is_none());
    }

    #[test]
    fn load_profile_stats_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/profiles")).unwrap();
        let stats = load_all_profile_stats(tmp.path());
        assert!(stats.is_empty());
    }

    #[test]
    fn load_profile_stats_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let stats = load_all_profile_stats(tmp.path());
        assert!(stats.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p quartermaster -- load_profile_stats`
Expected: compile errors — structs and functions don't exist yet.

- [ ] **Step 3: Implement SptProfileStats, ProfileStatus, and load_all_profile_stats**

In `src/spt/profiles.rs`, add after the existing imports:

```rust
use std::collections::HashMap;
```

After the existing `SptProfile` struct, add:

```rust
#[derive(Debug, Clone, Default)]
pub struct SptProfileStats {
    pub nickname: Option<String>,
    pub level: Option<i64>,
    pub side: Option<String>,
    pub experience: Option<i64>,
    pub registration_date: Option<i64>,
    pub raid_count: Option<i64>,
    pub survival_rate: Option<f64>,
    pub kill_count: Option<usize>,
}

pub enum ProfileStatus {
    Found(SptProfileStats),
    NotFound,
    ParseError,
}

#[derive(Deserialize, Default)]
struct FullProfileJson {
    info: Option<FullProfileInfo>,
    characters: Option<Characters>,
}

#[derive(Deserialize, Default)]
struct FullProfileInfo {
    id: Option<String>,
}

#[derive(Deserialize, Default)]
struct Characters {
    pmc: Option<PmcData>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct PmcData {
    info: Option<PmcInfo>,
    stats: Option<PmcStats>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct PmcInfo {
    nickname: Option<String>,
    level: Option<i64>,
    side: Option<String>,
    experience: Option<i64>,
    registration_date: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct PmcStats {
    eft: Option<EftStats>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct EftStats {
    overall_counters: Option<OverallCounters>,
    victims: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct OverallCounters {
    items: Option<Vec<CounterItem>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CounterItem {
    key: Vec<String>,
    value: i64,
}

pub fn load_all_profile_stats(spt_dir: &Path) -> HashMap<String, SptProfileStats> {
    let profiles_dir = spt_dir.join("SPT/user/profiles");
    let mut map = HashMap::new();

    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(e) => e,
        Err(_) => return map,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let parsed: FullProfileJson = match serde_json::from_str(&contents) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let aid = match parsed.info.and_then(|i| i.id) {
            Some(id) => id,
            None => continue,
        };

        let mut stats = SptProfileStats::default();

        if let Some(pmc) = parsed.characters.and_then(|c| c.pmc) {
            if let Some(info) = pmc.info {
                stats.nickname = info.nickname;
                stats.level = info.level;
                stats.side = info.side;
                stats.experience = info.experience;
                stats.registration_date = info.registration_date;
            }

            if let Some(pmc_stats) = pmc.stats {
                if let Some(eft) = pmc_stats.eft {
                    if let Some(counters) = eft.overall_counters.and_then(|c| c.items) {
                        let mut total_raids: i64 = 0;
                        let mut survived: i64 = 0;
                        for item in &counters {
                            if item.key.len() >= 2
                                && item.key[0] == "Sessions"
                                && item.key[1] == "Pmc"
                            {
                                total_raids += item.value;
                                if item.key.len() == 3 && item.key[2] == "Survived" {
                                    survived = item.value;
                                }
                            }
                        }
                        if total_raids > 0 {
                            stats.raid_count = Some(total_raids);
                            stats.survival_rate =
                                Some((survived as f64 / total_raids as f64 * 100.0 * 100.0).round() / 100.0);
                        } else {
                            stats.raid_count = Some(0);
                        }
                    }

                    stats.kill_count = eft.victims.map(|v| v.len());
                }
            }
        }

        map.insert(aid, stats);
    }

    map
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p quartermaster -- load_profile_stats`
Expected: all 4 new tests pass.

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p quartermaster`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/spt/profiles.rs
git commit -m "feat: add SPT profile stats parsing for admin user panel

Parse PMC data from SPT profile JSON files: nickname, level, side,
experience, raid count (from OverallCounters), survival rate, and kill
count (from Victims array). Uses serde defaults for graceful degradation
on partial/corrupt profiles."
```

---

### Task 3: Auth middleware session invalidation support

**Files:**
- Modify: `src/web/auth.rs`
- Modify: `src/db/users.rs` (add `password_changed_at` to `User` struct and `row_to_user`)

**Interfaces:**
- Consumes: `SessionUser`, `set_session_user()`, `auth_middleware()`, `User` struct, `row_to_user()` from earlier tasks
- Produces:
  - `User.password_changed_at: Option<String>` — new field
  - `set_session_user()` now also stores `session_created_at` (RFC3339 timestamp) in the session
  - `auth_middleware()` now checks `password_changed_at > session_created_at` and forces re-login if so

- [ ] **Step 1: Add `password_changed_at` to User struct**

In `src/db/users.rs`, add to the `User` struct after `disabled: bool`:

```rust
    pub password_changed_at: Option<String>,
```

Update `row_to_user` to read the new column. The SELECT queries already use positional column indices (0-6). Add the new column at index 7. Update all SELECTs in `get_user_by_username`, `list_users`, and `get_user_by_id` to include `password_changed_at`:

Change every occurrence of:
```
SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at
```
to:
```
SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
```

Update `row_to_user` to include:
```rust
        password_changed_at: row.get(7)?,
```

- [ ] **Step 2: Update `set_session_user` to store `session_created_at`**

In `src/web/auth.rs`, modify `set_session_user`:

```rust
pub fn set_session_user(session: &Session, user: &SessionUser) -> Result<()> {
    session
        .insert("user_id", user.user_id)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("username", &user.username)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("role", user.role.as_str())
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("session_created_at", chrono::Utc::now().to_rfc3339())
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    Ok(())
}
```

Add `use chrono;` to the imports if not already present (it is available via `Cargo.toml`).

- [ ] **Step 3: Update auth middleware to check `password_changed_at`**

In `src/web/auth.rs`, in the `auth_middleware` function, after the `Ok(Ok(Some(user))) if !user.disabled => user` arm, add the `password_changed_at` check. Replace the block that creates the `session_user` and checks the cached role:

```rust
    let session_user = SessionUser {
        user_id: verified_user.id,
        username: verified_user.username,
        role: verified_user.role,
    };

    // Check if password was changed after session was created
    if let Some(ref changed_at) = verified_user.password_changed_at {
        let session_created = session.get::<String>("session_created_at").unwrap_or(None);
        let should_invalidate = match session_created {
            None => true, // Old session without timestamp — force re-login
            Some(ref created) => changed_at.as_str() > created.as_str(),
        };
        if should_invalidate {
            tracing::info!(user_id = verified_user.id, "session invalidated due to password change");
            session.purge();
            return Ok(req.into_response(redirect_login()).map_into_boxed_body());
        }
    }

    let cached_role = session.get::<String>("role").unwrap_or(None);
    if cached_role.as_deref() != Some(session_user.role.as_str()) {
        if let Err(e) = set_session_user(&session, &session_user) {
            tracing::debug!(user_id, error = %e, "failed to update session cookie");
        }
    }
    req.extensions_mut().insert(session_user);
    next.call(req).await
```

- [ ] **Step 4: Run tests to verify everything compiles and passes**

Run: `cargo test -p quartermaster`
Expected: all tests pass. The `password_changed_at` field defaults to `NULL` for existing users, so old sessions are unaffected.

- [ ] **Step 5: Commit**

```bash
git add src/db/users.rs src/web/auth.rs
git commit -m "feat: session invalidation after password reset

Add password_changed_at to User struct. set_session_user() now stores
session_created_at timestamp. Auth middleware checks if password was
changed after session creation and forces re-login if so."
```

---

### Task 4: Admin panel templates and CSS

**Files:**
- Create: `templates/admin.html`
- Create: `templates/admin/partials/users.html`
- Create: `templates/admin/partials/user_row.html`
- Create: `templates/admin/partials/invites.html`
- Create: `templates/reset_password.html`
- Modify: `templates/partials/nav.html`
- Modify: `src/assets/style.css` (add admin panel styles)

**Interfaces:**
- Consumes: `base.html` template structure, `nav` macro from `partials/nav.html`, `SessionUser` (user), `SptProfileStats`, `ProfileStatus`, `InviteCodeWithUsers`, `User`, `Role`
- Produces: Template files that the Task 5 handlers will render

- [ ] **Step 1: Add "Admin" link to nav.html**

In `templates/partials/nav.html`, add after the Logs link (line 10, before the `</div>`):

```html
    {% if user.role.can_manage_users() %}
    <a href="/admin"{% if active == "admin" %} class="active"{% endif %}>{% call icons::shield() %}{% endcall %} Admin</a>
    {% endif %}
```

If the `icons::shield` macro doesn't exist, add it in `templates/partials/icons.html`. Alternatively, use `icons::settings` or just text. Check what icons exist:

Check `templates/partials/icons.html` for available icon macros. If no "shield" icon exists, add one using a simple SVG:

```html
{% macro shield() %}<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>{% endmacro %}
```

- [ ] **Step 2: Create `templates/admin.html`**

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% block title %}Admin — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("admin", user, csrf_token) %}{% endcall %}{% endblock %}
{% block content %}
<h1>Admin Panel</h1>

<div class="tab-bar">
    <button class="tab active" id="tab-users"
            hx-get="/api/admin/users"
            hx-target="#admin-content"
            hx-swap="innerHTML"
            hx-push-url="#users"
            onclick="setActiveTab(this)">Users</button>
    <button class="tab" id="tab-invites"
            hx-get="/api/admin/invites"
            hx-target="#admin-content"
            hx-swap="innerHTML"
            hx-push-url="#invites"
            onclick="setActiveTab(this)">Invites</button>
</div>

<div id="admin-content">
    {% include "admin/partials/users.html" %}
</div>

<script>
function setActiveTab(el) {
    document.querySelectorAll('.tab-bar .tab').forEach(t => t.classList.remove('active'));
    el.classList.add('active');
}
document.addEventListener('DOMContentLoaded', function() {
    if (location.hash === '#invites') {
        var btn = document.getElementById('tab-invites');
        if (btn) { btn.click(); }
    }
});
</script>
{% endblock %}
```

- [ ] **Step 3: Create `templates/admin/partials/users.html`**

Note: This template does NOT use `{% include %}` for user_row.html because Askama's include shares parent scope, and `user_row.html` references `reset_link`/`row_message` fields that only exist in single-row HTMX responses. The row markup is duplicated between this file and `user_row.html` — this is the standard HTMX pattern where the initial full render and the partial swap render are separate templates.

```html
<table>
    <thead>
        <tr>
            <th>Username</th>
            <th>Role</th>
            <th>Status</th>
            <th>SPT Profile</th>
            <th>Registered</th>
            <th>Actions</th>
        </tr>
    </thead>
    <tbody>
        {% for (u, profile) in users %}
        <tr{% if u.id == current_user_id %} class="current-user"{% endif %}>
            <td><strong>{{ u.username }}</strong></td>
            <td>
                {% if u.id != current_user_id %}
                <form style="display:inline">
                    <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                    <select name="role"
                            hx-post="/api/admin/users/{{ u.id }}/role"
                            hx-target="closest tr"
                            hx-swap="outerHTML"
                            hx-include="[name='csrf_token']"
                            hx-disabled-elt="this">
                        <option value="admin"{% if u.role.as_str() == "admin" %} selected{% endif %}>Admin</option>
                        <option value="moderator"{% if u.role.as_str() == "moderator" %} selected{% endif %}>Moderator</option>
                        <option value="player"{% if u.role.as_str() == "player" %} selected{% endif %}>Player</option>
                    </select>
                </form>
                {% else %}
                <span class="badge badge-muted">{{ u.role }}</span>
                {% endif %}
            </td>
            <td>
                {% if u.disabled %}
                <span class="badge badge-danger">Disabled</span>
                {% else %}
                <span class="badge badge-success">Active</span>
                {% endif %}
            </td>
            <td>
                {% match profile %}
                {% when ProfileStatus::Found with (stats) %}
                    <div class="profile-card">
                        <strong>{{ stats.nickname.as_deref().unwrap_or("—") }}</strong>
                        {% if let Some(level) = stats.level %}
                        <span class="text-muted text-sm">Lv.{{ level }}</span>
                        {% endif %}
                        {% if let Some(ref side) = stats.side %}
                        <span class="text-muted text-sm">{{ side }}</span>
                        {% endif %}
                        {% if let Some(raids) = stats.raid_count %}
                        <br><span class="text-muted text-sm">{{ raids }} raids</span>
                        {% endif %}
                        {% if let Some(sr) = stats.survival_rate %}
                        <span class="text-muted text-sm">{{ sr|fmt("{:.1}") }}% SR</span>
                        {% endif %}
                        {% if let Some(kills) = stats.kill_count %}
                        <span class="text-muted text-sm">{{ kills }} kills</span>
                        {% endif %}
                    </div>
                {% when ProfileStatus::NotFound %}
                    <span class="text-muted">No profile linked</span>
                {% when ProfileStatus::ParseError %}
                    <span class="text-muted">Profile data unavailable</span>
                {% endmatch %}
            </td>
            <td class="text-muted text-sm">{{ u.created_at }}</td>
            <td>
                {% if u.id != current_user_id %}
                <div class="action-buttons">
                    <form style="display:inline">
                        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                        <button type="button" class="btn btn-sm {% if u.disabled %}btn-success{% else %}btn-danger{% endif %}"
                                hx-post="/api/admin/users/{{ u.id }}/disable"
                                hx-target="closest tr"
                                hx-swap="outerHTML"
                                hx-include="closest form"
                                hx-disabled-elt="this">
                            {% if u.disabled %}Enable{% else %}Disable{% endif %}
                        </button>
                    </form>
                    <form style="display:inline">
                        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                        <button type="button" class="btn btn-sm btn-outline"
                                hx-post="/api/admin/users/{{ u.id }}/reset-password"
                                hx-target="closest tr"
                                hx-swap="outerHTML"
                                hx-include="closest form"
                                hx-disabled-elt="this">
                            Reset Password
                        </button>
                    </form>
                </div>
                {% endif %}
            </td>
        </tr>
        {% endfor %}
    </tbody>
</table>
{% if users.is_empty() %}
<div class="empty-state"><p>No users registered.</p></div>
{% endif %}
```

The template struct imports `ProfileStatus` via `use crate::spt::profiles::ProfileStatus;` in the handler file — Askama resolves type references from the struct's module scope.

- [ ] **Step 4: Create `templates/admin/partials/user_row.html`**

This template is used ONLY for single-row HTMX swap responses (not included from users.html). It has extra fields (`reset_link`, `row_message`) not present in the table view. The `UserRowTemplate` struct imports `ProfileStatus` so Askama can resolve the enum variants without `crate::` prefixes.

```html
<tr{% if u.id == current_user_id %} class="current-user"{% endif %}>
    {% if let Some(ref msg) = row_message %}
    <td colspan="6"><div class="toast toast-success" style="margin:0">{{ msg }}</div></td>
</tr>
<tr>
    {% endif %}
    <td><strong>{{ u.username }}</strong></td>
    <td>
        {% if u.id != current_user_id %}
        <form style="display:inline">
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
            <select name="role"
                    hx-post="/api/admin/users/{{ u.id }}/role"
                    hx-target="closest tr"
                    hx-swap="outerHTML"
                    hx-include="[name='csrf_token']"
                    hx-disabled-elt="this">
                <option value="admin"{% if u.role.as_str() == "admin" %} selected{% endif %}>Admin</option>
                <option value="moderator"{% if u.role.as_str() == "moderator" %} selected{% endif %}>Moderator</option>
                <option value="player"{% if u.role.as_str() == "player" %} selected{% endif %}>Player</option>
            </select>
        </form>
        {% else %}
        <span class="badge badge-muted">{{ u.role }}</span>
        {% endif %}
    </td>
    <td>
        {% if u.disabled %}
        <span class="badge badge-danger">Disabled</span>
        {% else %}
        <span class="badge badge-success">Active</span>
        {% endif %}
    </td>
    <td>
        {% match profile %}
        {% when ProfileStatus::Found with (stats) %}
            <div class="profile-card">
                <strong>{{ stats.nickname.as_deref().unwrap_or("—") }}</strong>
                {% if let Some(level) = stats.level %}
                <span class="text-muted text-sm">Lv.{{ level }}</span>
                {% endif %}
                {% if let Some(ref side) = stats.side %}
                <span class="text-muted text-sm">{{ side }}</span>
                {% endif %}
                {% if let Some(raids) = stats.raid_count %}
                <br><span class="text-muted text-sm">{{ raids }} raids</span>
                {% endif %}
                {% if let Some(sr) = stats.survival_rate %}
                <span class="text-muted text-sm">{{ sr|fmt("{:.1}") }}% SR</span>
                {% endif %}
                {% if let Some(kills) = stats.kill_count %}
                <span class="text-muted text-sm">{{ kills }} kills</span>
                {% endif %}
            </div>
        {% when ProfileStatus::NotFound %}
            <span class="text-muted">No profile linked</span>
        {% when ProfileStatus::ParseError %}
            <span class="text-muted">Profile data unavailable</span>
        {% endmatch %}
    </td>
    <td class="text-muted text-sm">{{ u.created_at }}</td>
    <td>
        {% if u.id != current_user_id %}
        <div class="action-buttons">
            <form style="display:inline">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <button type="button" class="btn btn-sm {% if u.disabled %}btn-success{% else %}btn-danger{% endif %}"
                        hx-post="/api/admin/users/{{ u.id }}/disable"
                        hx-target="closest tr"
                        hx-swap="outerHTML"
                        hx-include="closest form"
                        hx-disabled-elt="this">
                    {% if u.disabled %}Enable{% else %}Disable{% endif %}
                </button>
            </form>
            <form style="display:inline">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <button type="button" class="btn btn-sm btn-outline"
                        hx-post="/api/admin/users/{{ u.id }}/reset-password"
                        hx-target="closest tr"
                        hx-swap="outerHTML"
                        hx-include="closest form"
                        hx-disabled-elt="this">
                    Reset Password
                </button>
            </form>
        </div>
        {% if let Some(ref link) = reset_link %}
        <div class="reset-link-display" style="margin-top:0.5rem">
            <input type="text" readonly value="{{ link }}" style="width:100%;font-family:monospace;font-size:0.85rem" onclick="this.select()">
            <button type="button" class="btn btn-sm btn-outline" onclick="navigator.clipboard.writeText('{{ link }}').then(() => { this.textContent='Copied!'; setTimeout(() => this.textContent='Copy Link', 2000); })">Copy Link</button>
        </div>
        {% endif %}
        {% endif %}
    </td>
</tr>
```

- [ ] **Step 5: Create `templates/admin/partials/invites.html`**

Note: Askama cannot call free functions from templates. The invite status (available/used/expired) is pre-computed in the handler and passed as a string field `status` on a view struct. See `InviteView` in Task 5.

```html
<div class="card" style="margin-bottom:1rem">
    <h3>Create Invite Code</h3>
    <form hx-post="/api/admin/invites"
          hx-target="#admin-content"
          hx-swap="innerHTML"
          style="display:flex;gap:0.5rem;align-items:center">
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
        <label for="expiry">Expires in:</label>
        <select name="expiry" id="expiry">
            <option value="1h">1 hour</option>
            <option value="24h">24 hours</option>
            <option value="7d" selected>7 days</option>
            <option value="30d">30 days</option>
            <option value="never">Never</option>
        </select>
        <button type="submit" class="btn" hx-disabled-elt="this">Create</button>
    </form>
</div>

<table>
    <thead>
        <tr>
            <th>Code</th>
            <th>Created By</th>
            <th>Created</th>
            <th>Expires</th>
            <th>Status</th>
            <th>Used By</th>
        </tr>
    </thead>
    <tbody>
        {% for iv in invites %}
        <tr{% if let Some(ref h) = highlight_code %}{% if iv.code == *h %} class="highlight-flash"{% endif %}{% endif %}>
            <td>
                <code>{{ iv.code }}</code>
                {% if iv.status == "available" %}
                <button type="button" class="btn btn-sm btn-outline" style="margin-left:0.5rem"
                        onclick="navigator.clipboard.writeText('{{ iv.code }}').then(() => { this.textContent='Copied!'; setTimeout(() => this.textContent='Copy', 2000); })">Copy</button>
                {% endif %}
            </td>
            <td>{{ iv.created_by_username.as_deref().unwrap_or("—") }}</td>
            <td class="text-muted text-sm">{{ iv.created_at }}</td>
            <td class="text-muted text-sm">{{ iv.expires_at.as_deref().unwrap_or("Never") }}</td>
            <td>
                {% if iv.status == "used" %}
                <span class="badge badge-info">Used</span>
                {% elif iv.status == "expired" %}
                <span class="badge badge-danger">Expired</span>
                {% else %}
                <span class="badge badge-success">Available</span>
                {% endif %}
            </td>
            <td>{{ iv.used_by_username.as_deref().unwrap_or("") }}</td>
        </tr>
        {% endfor %}
    </tbody>
</table>
{% if invites.is_empty() %}
<div class="empty-state"><p>No invite codes created yet.</p></div>
{% endif %}
```

- [ ] **Step 6: Create `templates/reset_password.html`**

```html
{% extends "base.html" %}
{% block title %}Reset Password — Quartermaster{% endblock %}
{% block content %}
<div style="max-width:400px;margin:2rem auto">
    <h1>Set New Password</h1>
    {% if let Some(ref err) = error %}
    <div class="toast toast-error">{{ err }}</div>
    {% endif %}
    {% if token_valid %}
    <form method="post" action="/reset-password">
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
        <input type="hidden" name="token" value="{{ token }}">
        <div class="form-group">
            <label for="password">New Password</label>
            <input type="password" name="password" id="password" minlength="8" maxlength="128" required>
        </div>
        <div class="form-group">
            <label for="password_confirm">Confirm Password</label>
            <input type="password" name="password_confirm" id="password_confirm" minlength="8" maxlength="128" required>
        </div>
        <button type="submit" class="btn" style="width:100%">Set Password</button>
    </form>
    {% endif %}
</div>
{% endblock %}
```

- [ ] **Step 7: Add admin panel CSS**

In `src/assets/style.css`, add the following styles (append to end of file):

```css
/* Admin panel */
.tab-bar { display: flex; gap: 0; border-bottom: 2px solid var(--border); margin-bottom: 1rem; }
.tab-bar .tab { padding: 0.5rem 1.25rem; background: none; border: none; border-bottom: 2px solid transparent; margin-bottom: -2px; cursor: pointer; color: var(--text-muted); font-size: 0.95rem; }
.tab-bar .tab.active { border-bottom-color: var(--accent); color: var(--text); font-weight: 600; }
.tab-bar .tab:hover { color: var(--text); }

.profile-card { line-height: 1.5; }
.action-buttons { display: flex; gap: 0.25rem; flex-wrap: wrap; }
.reset-link-display { display: flex; gap: 0.25rem; align-items: center; }
.reset-link-display input { font-family: monospace; font-size: 0.85rem; }

.form-group { margin-bottom: 1rem; }
.form-group label { display: block; margin-bottom: 0.25rem; font-weight: 500; }
.form-group input { width: 100%; padding: 0.5rem; border: 1px solid var(--border); border-radius: 4px; background: var(--bg); color: var(--text); }

.current-user td { opacity: 0.7; }

@keyframes highlight { from { background: var(--warning-bg, rgba(255,193,7,0.2)); } to { background: transparent; } }
.highlight-flash { animation: highlight 2s ease-out; }

.badge-info { background: var(--info, #17a2b8); color: #fff; }
```

- [ ] **Step 8: Run the Rust compiler to check templates compile**

Run: `cargo check`
Expected: templates won't compile yet because the handler code (Askama template structs) doesn't exist. That's expected — we verify templates in Task 5.

- [ ] **Step 9: Commit**

```bash
git add templates/admin.html templates/admin/partials/users.html templates/admin/partials/user_row.html templates/admin/partials/invites.html templates/reset_password.html templates/partials/nav.html templates/partials/icons.html src/assets/style.css
git commit -m "feat: add admin panel templates and navigation

Tab-based admin panel with Users and Invites partials, user row with
inline HTMX actions (role dropdown, disable toggle, reset password),
invite table with create form, and password reset page template."
```

---

### Task 5: Admin handlers, routes, and password reset handlers

**Files:**
- Create: `src/web/handlers/admin.rs`
- Modify: `src/web/handlers/mod.rs`
- Modify: `src/web/handlers/auth.rs` (add reset password handlers)
- Modify: `src/web/mod.rs` (add routes)
- Modify: `src/web/error.rs` (add `UnprocessableEntity` variant for 422)

**Interfaces:**
- Consumes:
  - `Database::list_users()`, `Database::get_user_by_id()`, `Database::update_user_role()`, `Database::set_user_disabled()`, `Database::create_reset_token()`, `Database::list_invite_codes()`, `Database::create_invite()`, `Database::get_reset_token()`, `Database::delete_reset_token()`, `Database::update_user_password()` (from Task 1)
  - `load_all_profile_stats()`, `SptProfileStats`, `ProfileStatus` (from Task 2)
  - `set_session_user()` with `session_created_at` (from Task 3)
  - All templates from Task 4
  - `crate::invite::{generate_invite_code, parse_expiry}` (from Task 1)
  - `auth::hash_password()` (existing)
  - `csrf::get_or_create_token()`, `csrf::validate_token()` (existing)
  - `subtle::ConstantTimeEq` (from `subtle` crate added in Task 1)
- Produces: All 9 route handlers, wired into actix-web routes

- [ ] **Step 1: Add `UnprocessableEntity` variant to WebError**

In `src/web/error.rs`, add a new variant to the `WebError` enum:

```rust
    UnprocessableEntity(String),
```

Add the match arms in `Display`:
```rust
            WebError::UnprocessableEntity(msg) => write!(f, "unprocessable: {msg}"),
```

In `status_code()`:
```rust
            WebError::UnprocessableEntity(_) => StatusCode::UNPROCESSABLE_ENTITY,
```

In `error_response()` (the `(title, message)` match):
```rust
            WebError::UnprocessableEntity(msg) => ("Unprocessable Entity".to_string(), msg.clone()),
```

- [ ] **Step 2: Add `pub mod admin;` to handler module**

In `src/web/handlers/mod.rs`, add:

```rust
pub mod admin;
```

- [ ] **Step 3: Create admin handlers**

Create `src/web/handlers/admin.rs`:

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::HttpRequest;
use askama::Template;

use crate::db::users::{InviteCodeWithUsers, Role, User};
use crate::spt::profiles::{load_all_profile_stats, ProfileStatus, SptProfileStats};
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

fn is_invite_expired(expires_at: Option<&str>) -> bool {
    let Some(exp) = expires_at else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(exp) {
        Ok(dt) => dt < chrono::Utc::now(),
        Err(_) => exp < chrono::Utc::now().to_rfc3339().as_str(),
    }
}

fn build_user_profiles(
    users: &[User],
    spt_dir: &std::path::Path,
    profile_stats: &std::collections::HashMap<String, SptProfileStats>,
) -> Vec<ProfileStatus> {
    users
        .iter()
        .map(|u| {
            if u.spt_profile_id.is_empty() {
                return ProfileStatus::NotFound;
            }
            match profile_stats.get(&u.spt_profile_id) {
                Some(stats) => ProfileStatus::Found(stats.clone()),
                None => {
                    let profile_path = spt_dir
                        .join("SPT/user/profiles")
                        .join(format!("{}.json", u.spt_profile_id));
                    if profile_path.exists() {
                        ProfileStatus::ParseError
                    } else {
                        ProfileStatus::NotFound
                    }
                }
            }
        })
        .collect()
}

// -- Templates --

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminPageTemplate {
    user: SessionUser,
    csrf_token: String,
    users: Vec<(User, ProfileStatus)>,
    current_user_id: i64,
    flash: Option<crate::web::flash::FlashMessage>,
}

#[derive(Template)]
#[template(path = "admin/partials/users.html")]
struct UsersPartialTemplate {
    users: Vec<(User, ProfileStatus)>,
    current_user_id: i64,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "admin/partials/user_row.html")]
struct UserRowTemplate {
    u: User,
    profile: ProfileStatus,
    current_user_id: i64,
    csrf_token: String,
    reset_link: Option<String>,
    row_message: Option<String>,
}

// InviteView — pre-computed view struct for invites template
// (Askama can't call free functions, so we pre-compute status)
pub struct InviteView {
    pub code: String,
    pub created_by_username: Option<String>,
    pub used_by_username: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub status: String, // "available", "used", or "expired"
}

impl InviteView {
    fn from_db(ic: InviteCodeWithUsers) -> Self {
        let status = if ic.invite.used_by.is_some() {
            "used"
        } else if is_invite_expired(ic.invite.expires_at.as_deref()) {
            "expired"
        } else {
            "available"
        };
        InviteView {
            code: ic.invite.code,
            created_by_username: ic.created_by_username,
            used_by_username: ic.used_by_username,
            created_at: ic.invite.created_at,
            expires_at: ic.invite.expires_at,
            status: status.to_string(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/partials/invites.html")]
struct InvitesPartialTemplate {
    invites: Vec<InviteView>,
    csrf_token: String,
    highlight_code: Option<String>,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct RoleForm {
    role: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct CsrfOnly {
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct InviteForm {
    expiry: String,
    csrf_token: String,
}

// -- Admin scope middleware --

pub async fn admin_middleware(
    req: actix_web::dev::ServiceRequest,
    next: actix_web::middleware::Next<actix_web::body::BoxBody>,
) -> Result<actix_web::dev::ServiceResponse<actix_web::body::BoxBody>, actix_web::Error> {
    let user = req
        .extensions()
        .get::<SessionUser>()
        .cloned()
        .ok_or(WebError::Forbidden)?;

    if !user.role.can_manage_users() {
        return Err(WebError::Forbidden.into());
    }

    next.call(req).await
}

// -- Handlers --

pub async fn admin_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    crate::web::auth::require_capability(&user, Role::can_manage_users)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let flash = crate::web::flash::take_flash(&session);

    let db = state.db.clone();
    let all_users = web::block(move || {
        let db = db.lock();
        db.list_users()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profiles = build_user_profiles(&all_users, &state.spt_dir, &profile_stats);
    let users: Vec<(User, ProfileStatus)> = all_users.into_iter().zip(profiles).collect();
    let current_user_id = user.user_id;

    let tmpl = AdminPageTemplate {
        user,
        csrf_token,
        users,
        current_user_id,
        flash,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn admin_users(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let all_users = web::block(move || {
        let db = db.lock();
        db.list_users()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profiles = build_user_profiles(&all_users, &state.spt_dir, &profile_stats);
    let users: Vec<(User, ProfileStatus)> = all_users.into_iter().zip(profiles).collect();

    let tmpl = UsersPartialTemplate {
        users,
        current_user_id: user.user_id,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn change_role(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<RoleForm>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();
    if target_id == current_user.user_id {
        return Err(WebError::Forbidden.into());
    }

    let new_role = Role::try_from(form.role.clone())
        .map_err(|_| WebError::BadRequest("Invalid role".to_string()))?;

    let db = state.db.clone();
    let affected = web::block(move || {
        let db = db.lock();
        db.update_user_role(target_id, new_role)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if affected == 0 {
        return Err(WebError::UnprocessableEntity("Cannot demote the last admin".to_string()).into());
    }

    render_user_row(&state, &session, target_id, current_user.user_id, None, None).await
}

pub async fn toggle_disable(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<CsrfOnly>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();
    if target_id == current_user.user_id {
        return Err(WebError::Forbidden.into());
    }

    let db = state.db.clone();
    let target_user = web::block(move || {
        let db = db.lock();
        db.get_user_by_id(target_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let new_disabled = !target_user.disabled;
    let db = state.db.clone();
    let affected = web::block(move || {
        let db = db.lock();
        db.set_user_disabled(target_id, new_disabled)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if affected == 0 {
        return Err(WebError::UnprocessableEntity("Cannot disable the last admin".to_string()).into());
    }

    let message = if new_disabled {
        Some("User disabled — will be logged out on their next request.".to_string())
    } else {
        None
    };

    render_user_row(&state, &session, target_id, current_user.user_id, None, message).await
}

pub async fn create_reset_token(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<CsrfOnly>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();

    // Verify user exists
    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.get_user_by_id(target_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    // Generate CSPRNG token (OsRng from rand crate, not argon2 re-export)
    use rand::RngCore;
    let mut token_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token_bytes);
    let token = base64_url_encode(&token_bytes);

    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339();

    let db = state.db.clone();
    let token_clone = token.clone();
    web::block(move || {
        let db = db.lock();
        db.create_reset_token(target_id, &token_clone, &expires_at)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let reset_link = format!("/reset-password?token={token}");
    render_user_row(&state, &session, target_id, current_user.user_id, Some(reset_link), None).await
}

pub async fn admin_invites(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let db_invites = web::block(move || {
        let db = db.lock();
        db.list_invite_codes()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let invites: Vec<InviteView> = db_invites.into_iter().map(InviteView::from_db).collect();

    let tmpl = InvitesPartialTemplate {
        invites,
        csrf_token,
        highlight_code: None,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn create_invite(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<InviteForm>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let code = crate::invite::generate_invite_code();

    let expires_at = if form.expiry == "never" {
        None
    } else {
        Some(
            crate::invite::parse_expiry(&form.expiry)
                .map_err(|_| WebError::BadRequest("Invalid expiry value".to_string()))?,
        )
    };

    let db = state.db.clone();
    let code_clone = code.clone();
    let user_id = current_user.user_id;
    web::block(move || {
        let db = db.lock();
        db.create_invite(&code_clone, Some(user_id), expires_at.as_deref())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();
    let db_invites = web::block(move || {
        let db = db.lock();
        db.list_invite_codes()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let invites: Vec<InviteView> = db_invites.into_iter().map(InviteView::from_db).collect();

    let tmpl = InvitesPartialTemplate {
        invites,
        csrf_token,
        highlight_code: Some(code),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

// -- Helpers --

async fn render_user_row(
    state: &Data<AppState>,
    session: &Session,
    user_id: i64,
    current_user_id: i64,
    reset_link: Option<String>,
    row_message: Option<String>,
) -> actix_web::Result<Html> {
    let csrf_token = crate::web::csrf::get_or_create_token(session);

    let db = state.db.clone();
    let user = web::block(move || {
        let db = db.lock();
        db.get_user_by_id(user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let spt_dir = state.spt_dir.clone();
    let aid = user.spt_profile_id.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profile = if aid.is_empty() {
        ProfileStatus::NotFound
    } else {
        match profile_stats.get(&aid) {
            Some(stats) => ProfileStatus::Found(stats.clone()),
            None => {
                let path = state
                    .spt_dir
                    .join("SPT/user/profiles")
                    .join(format!("{aid}.json"));
                if path.exists() {
                    ProfileStatus::ParseError
                } else {
                    ProfileStatus::NotFound
                }
            }
        }
    };

    let tmpl = UserRowTemplate {
        u: user,
        profile,
        current_user_id,
        csrf_token,
        reset_link,
        row_message,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

fn base64_url_encode(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 0x3F) as usize] as char);
        }
    }
    result
}
```

- [ ] **Step 4: Add reset password handlers to auth.rs**

In `src/web/handlers/auth.rs`, add the following form structs and handlers:

```rust
#[derive(serde::Deserialize)]
pub struct ResetPasswordQuery {
    token: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ResetPasswordForm {
    token: String,
    password: String,
    password_confirm: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "reset_password.html")]
struct ResetPasswordTemplate {
    error: Option<String>,
    token: String,
    token_valid: bool,
    csrf_token: String,
    flash: Option<crate::web::flash::FlashMessage>,
}

pub async fn reset_password_page(
    query: Query<ResetPasswordQuery>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let token = query.token.clone().unwrap_or_default();

    if token.is_empty() {
        let tmpl = ResetPasswordTemplate {
            error: Some("This password reset link is invalid or has already been used. Please contact an administrator for a new link.".to_string()),
            token: String::new(),
            token_valid: false,
            csrf_token,
            flash: None,
        };
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let db = state.db.clone();
    let token_clone = token.clone();
    let reset_token = web::block(move || {
        let db = db.lock();
        db.get_reset_token(&token_clone)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let valid = match reset_token {
        Some(ref rt) => {
            use subtle::ConstantTimeEq;
            let ct_match = token.as_bytes().ct_eq(rt.token.as_bytes());
            bool::from(ct_match) && !is_token_expired(&rt.expires_at)
        }
        None => false,
    };

    if !valid {
        let tmpl = ResetPasswordTemplate {
            error: Some("This password reset link is invalid or has already been used. Please contact an administrator for a new link.".to_string()),
            token: String::new(),
            token_valid: false,
            csrf_token,
            flash: None,
        };
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let tmpl = ResetPasswordTemplate {
        error: None,
        token,
        token_valid: true,
        csrf_token,
        flash: None,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn reset_password_submit(
    form: Form<ResetPasswordForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let render_error = |msg: &str, token: String| -> actix_web::Result<HttpResponse> {
        let tmpl = ResetPasswordTemplate {
            error: Some(msg.to_string()),
            token,
            token_valid: true,
            csrf_token: csrf_token.clone(),
            flash: None,
        };
        Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?))
    };

    if form.password.len() < MIN_PASSWORD_LEN {
        return render_error("Password must be 8-128 characters", form.token);
    }
    if form.password.len() > MAX_PASSWORD_LEN {
        return render_error("Password must be 8-128 characters", form.token);
    }
    if form.password != form.password_confirm {
        return render_error("Passwords do not match", form.token);
    }

    let db = state.db.clone();
    let token_clone = form.token.clone();
    let reset_token = web::block(move || {
        let db = db.lock();
        db.get_reset_token(&token_clone)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rt = match reset_token {
        Some(rt) => {
            use subtle::ConstantTimeEq;
            let ct_match = form.token.as_bytes().ct_eq(rt.token.as_bytes());
            if !bool::from(ct_match) || is_token_expired(&rt.expires_at) {
                return render_error(
                    "This password reset link is invalid or has already been used. Please contact an administrator for a new link.",
                    String::new(),
                );
            }
            rt
        }
        None => {
            return render_error(
                "This password reset link is invalid or has already been used. Please contact an administrator for a new link.",
                String::new(),
            );
        }
    };

    let password = form.password.clone();
    let password_hash = web::block(move || hash_password(&password))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let db = state.db.clone();
    let user_id = rt.user_id;
    let token_to_delete = rt.token.clone();
    web::block(move || {
        let db = db.lock();
        db.update_user_password(user_id, &password_hash)?;
        db.delete_reset_token(&token_to_delete)?;
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    crate::web::flash::set_flash(&session, "Password updated — please log in.", "success");

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish())
}

fn is_token_expired(expires_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(expires_at) {
        Ok(dt) => dt < chrono::Utc::now(),
        Err(_) => expires_at < chrono::Utc::now().to_rfc3339().as_str(),
    }
}
```

Add `use subtle;` to the imports at the top of the file.

- [ ] **Step 5: Wire routes in `src/web/mod.rs`**

In `src/web/mod.rs`, add the admin scope and reset password routes. Inside the `HttpServer::new` closure, add before the `// HTMX API` comment:

```rust
            // Password reset (public, rate-limited)
            .service(
                web::resource("/reset-password")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::get().to(handlers::auth::reset_password_page))
                    .route(web::post().to(handlers::auth::reset_password_submit)),
            )
```

Inside the `/api` scope (after the existing routes, before the closing `)` of the `/api` scope), add a nested admin scope:

```rust
                    // Admin API (requires can_manage_users via scoped middleware)
                    .service(
                        web::scope("/admin")
                            .wrap(from_fn(handlers::admin::admin_middleware))
                            .route("/users", web::get().to(handlers::admin::admin_users))
                            .route("/invites", web::get().to(handlers::admin::admin_invites))
                            .route("/users/{id}/role", web::post().to(handlers::admin::change_role))
                            .route("/users/{id}/disable", web::post().to(handlers::admin::toggle_disable))
                            .route("/users/{id}/reset-password", web::post().to(handlers::admin::create_reset_token))
                            .route("/invites", web::post().to(handlers::admin::create_invite)),
                    )
```

Add the `/admin` page route inside the authenticated scope (the `web::scope("")` block), after the `/logs` route:

```rust
                    .route("/admin", web::get().to(handlers::admin::admin_page))
```

- [ ] **Step 6: Build and fix compilation errors**

Run: `cargo check`

Fix any template compilation or type mismatch issues. Common issues to watch for:
- Askama template struct field types must match what the template expects
- The `round` filter in Askama for `survival_rate` — may need a custom filter or use `format!` in the template
- `ProfileStatus` enum needs to be accessible in templates (may need `use` in the template or qualified paths)
- The `is_invite_expired` function in the invites template needs to be available as an Askama function or called differently

- [ ] **Step 7: Run all tests**

Run: `cargo test -p quartermaster`
Expected: all tests pass.

- [ ] **Step 8: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add src/web/handlers/admin.rs src/web/handlers/mod.rs src/web/handlers/auth.rs src/web/mod.rs src/web/error.rs
git commit -m "feat: add admin panel handlers and password reset flow

Admin panel with scoped middleware for can_manage_users(), 7 admin
handlers (page, users/invites partials, role change, disable toggle,
reset token, invite creation), and 2 unauthenticated reset password
handlers. CSRF validation on all POST endpoints. Self-protection and
last-admin guard enforced server-side. CSPRNG tokens with constant-time
comparison via subtle crate."
```

---

### Task 6: Integration testing and polish

**Files:**
- Modify: `TODO.md`
- Possibly modify: templates, handlers, CSS for issues found during manual testing

**Interfaces:**
- Consumes: everything from Tasks 1-5
- Produces: working, tested admin panel

- [ ] **Step 1: Start the dev server**

Run: `QUMA_SPT_DIR=~/spt-server cargo run -- serve`

- [ ] **Step 2: Test the golden path**

1. Log in as admin
2. Click "Admin" in nav bar → should show tabbed admin panel with Users tab active
3. Verify user table shows all users with roles, status, and SPT profile data
4. Test role dropdown: change a user's role → row should update inline
5. Test disable toggle: disable a user → row should update, badge changes to "Disabled"
6. Test enable toggle: re-enable → badge changes back to "Active"
7. Switch to Invites tab → should load invites table
8. Create an invite with 7d expiry → table refreshes, new code highlighted
9. Copy an invite code → clipboard should work
10. Test reset password: click Reset Password → row shows copyable reset link
11. Visit reset link in incognito → should show password form
12. Set new password → should redirect to login with flash message
13. Log in with new password → should succeed

- [ ] **Step 3: Test edge cases**

1. Try to change your own role → should see 403 (dropdown should be hidden)
2. As last admin, try to demote yourself via crafted request → 403
3. Try to disable yourself → 403
4. Visit expired reset link → generic error message
5. Visit already-used reset link → same generic error
6. Refresh admin page with `#invites` hash → should open Invites tab
7. Create user with no SPT profile → should show "No profile linked"

- [ ] **Step 4: Update TODO.md**

The stash value TODO was already added. Verify it's present:

```
- stash value in admin user profile cards (requires inventory iteration + item price data)
```

- [ ] **Step 5: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix: polish admin panel after integration testing"
```

Only commit if there are actual changes. Skip if everything worked on first pass.
