# Mod Request & Voting System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let players request mods from SPT Forge, vote on requests with optional comments, and let admins/moderators approve or reject with optional auto-install.

**Architecture:** New `mod_requests` and `mod_request_votes` tables (migration 007). New `src/db/requests.rs` for CRUD, `src/web/handlers/requests.rs` for all handlers. Requests tab added to existing mods page via HTMX tab bar (same pattern as admin panel). HTMX live search proxies Forge API with Governor rate limiting. Forge metadata cached at request creation, refreshed lazily via background `tokio::spawn` when stale.

**Tech Stack:** Rust, actix-web 4, rusqlite, Askama templates, HTMX, chrono, Governor rate limiting, tokio::spawn for background refresh.

## Global Constraints

- All times stored as RFC3339 strings via `chrono::Utc::now().to_rfc3339()`
- CSRF validation on all POST endpoints via `crate::web::csrf::validate_token()`
- Authentication via existing `auth_middleware` and `require_auth()` — all users can request/vote
- Mod management actions gated by `Role::can_manage_mods()` (Admin or Moderator)
- Database access in web handlers uses `web::block(move || { let db = db.lock(); ... })` pattern
- No new Role capabilities — use existing `can_manage_mods()`
- Askama 0.16 templates — no `round` filter, no calling free functions, `{% include %}` shares parent scope
- Follow existing patterns: `FlashMessage` for feedback, `WebError` for errors, `SessionUser` for auth
- Config env overrides use `QUMA_` prefix
- `forge_cache_ttl` defaults to 86400 seconds (24 hours)

---

### Task 1: Database Migration & CRUD Layer

**Files:**
- Create: `migrations/007_mod_requests.sql`
- Create: `src/db/requests.rs`
- Modify: `src/db/schema.rs` — add MIGRATION_007 const and version 7 block
- Modify: `src/db/mod.rs` — add `pub mod requests;`
- Modify: `src/db/tests.rs` — add tests for new DB functions

**Interfaces:**
- Consumes: `Database` struct from `src/db/mod.rs`, `rusqlite::params!` macro
- Produces: All types and DB methods used by Tasks 2-5:
  - `ModRequest` struct: `{ id: i64, user_id: i64, forge_mod_id: i64, mod_name: String, mod_slug: Option<String>, mod_description: Option<String>, fika_compatible: String, reason: Option<String>, status: String, resolved_by: Option<i64>, resolved_at: Option<String>, resolve_comment: Option<String>, created_at: String, forge_cached_at: String }`
  - `ModRequestVote` struct: `{ id: i64, request_id: i64, user_id: i64, upvote: bool, comment: Option<String>, created_at: String }`
  - `ModRequestView` struct: `{ request: ModRequest, requester_username: String, vote_score: i64, upvote_count: i64, downvote_count: i64, comment_count: i64, current_user_vote: Option<bool>, resolver_username: Option<String> }`
  - `VoteComment` struct: `{ username: String, upvote: bool, comment: String, created_at: String }`
  - DB methods on `Database`:
    - `create_mod_request(&self, user_id: i64, forge_mod_id: i64, mod_name: &str, mod_slug: Option<&str>, mod_description: Option<&str>, fika_compatible: &str, reason: Option<&str>) -> rusqlite::Result<i64>`
    - `list_mod_requests(&self, status: Option<&str>, current_user_id: i64) -> rusqlite::Result<Vec<ModRequestView>>`
    - `get_mod_request(&self, id: i64) -> rusqlite::Result<Option<ModRequest>>`
    - `has_pending_request_for_mod(&self, forge_mod_id: i64) -> rusqlite::Result<bool>`
    - `resolve_mod_request(&self, id: i64, status: &str, resolved_by: i64, comment: Option<&str>) -> rusqlite::Result<usize>`
    - `update_mod_request_cache(&self, id: i64, mod_name: &str, mod_slug: Option<&str>, mod_description: Option<&str>, fika_compatible: &str) -> rusqlite::Result<usize>`
    - `upsert_vote(&self, request_id: i64, user_id: i64, upvote: bool, comment: Option<&str>) -> rusqlite::Result<()>`
    - `delete_vote(&self, request_id: i64, user_id: i64) -> rusqlite::Result<usize>`
    - `get_vote(&self, request_id: i64, user_id: i64) -> rusqlite::Result<Option<ModRequestVote>>`
    - `list_vote_comments(&self, request_id: i64) -> rusqlite::Result<Vec<VoteComment>>`

- [ ] **Step 1: Create migration file**

Create `migrations/007_mod_requests.sql`:

```sql
CREATE TABLE mod_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id),
    forge_mod_id INTEGER NOT NULL,
    mod_name TEXT NOT NULL,
    mod_slug TEXT,
    mod_description TEXT,
    fika_compatible TEXT NOT NULL DEFAULT 'unknown',
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    resolved_by INTEGER REFERENCES users(id),
    resolved_at TEXT,
    resolve_comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    forge_cached_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE mod_request_votes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL REFERENCES mod_requests(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    upvote INTEGER NOT NULL,
    comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(request_id, user_id)
);

CREATE INDEX idx_mod_requests_status ON mod_requests(status);
CREATE INDEX idx_mod_requests_forge_mod_id ON mod_requests(forge_mod_id);
CREATE INDEX idx_mod_request_votes_request_id ON mod_request_votes(request_id);
```

- [ ] **Step 2: Register migration in schema.rs**

In `src/db/schema.rs`, add after line 8:

```rust
const MIGRATION_007: &str = include_str!("../../migrations/007_mod_requests.sql");
```

Add after line 41 (the `if current_version < 6` block):

```rust
    if current_version < 7 {
        conn.execute_batch(MIGRATION_007)?;
        conn.pragma_update(None, "user_version", 7)?;
    }
```

- [ ] **Step 3: Add module declaration**

In `src/db/mod.rs`, add after line 7 (`pub mod users;`):

```rust
pub mod requests;
```

- [ ] **Step 4: Write the DB module with structs and all CRUD methods**

Create `src/db/requests.rs`:

```rust
use rusqlite::params;

use super::Database;

#[derive(Debug, Clone)]
pub struct ModRequest {
    pub id: i64,
    pub user_id: i64,
    pub forge_mod_id: i64,
    pub mod_name: String,
    pub mod_slug: Option<String>,
    pub mod_description: Option<String>,
    pub fika_compatible: String,
    pub reason: Option<String>,
    pub status: String,
    pub resolved_by: Option<i64>,
    pub resolved_at: Option<String>,
    pub resolve_comment: Option<String>,
    pub created_at: String,
    pub forge_cached_at: String,
}

#[derive(Debug, Clone)]
pub struct ModRequestVote {
    pub id: i64,
    pub request_id: i64,
    pub user_id: i64,
    pub upvote: bool,
    pub comment: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ModRequestView {
    pub request: ModRequest,
    pub requester_username: String,
    pub vote_score: i64,
    pub upvote_count: i64,
    pub downvote_count: i64,
    pub comment_count: i64,
    pub current_user_vote: Option<bool>,
    pub resolver_username: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VoteComment {
    pub username: String,
    pub upvote: bool,
    pub comment: String,
    pub created_at: String,
}

impl Database {
    pub fn create_mod_request(
        &self,
        user_id: i64,
        forge_mod_id: i64,
        mod_name: &str,
        mod_slug: Option<&str>,
        mod_description: Option<&str>,
        fika_compatible: &str,
        reason: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO mod_requests (user_id, forge_mod_id, mod_name, mod_slug, mod_description, fika_compatible, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![user_id, forge_mod_id, mod_name, mod_slug, mod_description, fika_compatible, reason],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_mod_requests(
        &self,
        status: Option<&str>,
        current_user_id: i64,
    ) -> rusqlite::Result<Vec<ModRequestView>> {
        let base_query = "
            SELECT
                r.id, r.user_id, r.forge_mod_id, r.mod_name, r.mod_slug,
                r.mod_description, r.fika_compatible, r.reason, r.status,
                r.resolved_by, r.resolved_at, r.resolve_comment,
                r.created_at, r.forge_cached_at,
                u.username AS requester_username,
                COALESCE(SUM(CASE WHEN v.upvote = 1 THEN 1 ELSE 0 END), 0) AS upvote_count,
                COALESCE(SUM(CASE WHEN v.upvote = 0 THEN 1 ELSE 0 END), 0) AS downvote_count,
                COALESCE(SUM(CASE WHEN v.comment IS NOT NULL AND v.comment != '' THEN 1 ELSE 0 END), 0) AS comment_count,
                cv.upvote AS current_user_vote,
                ru.username AS resolver_username
            FROM mod_requests r
            JOIN users u ON r.user_id = u.id
            LEFT JOIN mod_request_votes v ON r.id = v.request_id
            LEFT JOIN mod_request_votes cv ON r.id = cv.request_id AND cv.user_id = ?1
            LEFT JOIN users ru ON r.resolved_by = ru.id
        ";

        let (query, do_filter) = match status {
            Some(s) if !s.is_empty() => (
                format!(
                    "{base_query} WHERE r.status = ?2
                     GROUP BY r.id
                     ORDER BY (COALESCE(SUM(CASE WHEN v.upvote = 1 THEN 1 ELSE 0 END), 0)
                             - COALESCE(SUM(CASE WHEN v.upvote = 0 THEN 1 ELSE 0 END), 0)) DESC,
                              r.created_at DESC"
                ),
                Some(s),
            ),
            _ => (
                format!(
                    "{base_query}
                     GROUP BY r.id
                     ORDER BY (COALESCE(SUM(CASE WHEN v.upvote = 1 THEN 1 ELSE 0 END), 0)
                             - COALESCE(SUM(CASE WHEN v.upvote = 0 THEN 1 ELSE 0 END), 0)) DESC,
                              r.created_at DESC"
                ),
                None,
            ),
        };

        let mut stmt = self.conn.prepare(&query)?;

        let rows = if let Some(s) = do_filter {
            stmt.query_map(params![current_user_id, s], row_to_request_view)?
        } else {
            stmt.query_map(params![current_user_id], row_to_request_view)?
        };

        rows.collect()
    }

    pub fn get_mod_request(&self, id: i64) -> rusqlite::Result<Option<ModRequest>> {
        self.conn
            .query_row(
                "SELECT id, user_id, forge_mod_id, mod_name, mod_slug, mod_description,
                        fika_compatible, reason, status, resolved_by, resolved_at,
                        resolve_comment, created_at, forge_cached_at
                 FROM mod_requests WHERE id = ?1",
                params![id],
                row_to_request,
            )
            .optional()
    }

    pub fn has_pending_request_for_mod(&self, forge_mod_id: i64) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM mod_requests WHERE forge_mod_id = ?1 AND status = 'pending'",
            params![forge_mod_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn resolve_mod_request(
        &self,
        id: i64,
        status: &str,
        resolved_by: i64,
        comment: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE mod_requests SET status = ?1, resolved_by = ?2, resolved_at = ?3, resolve_comment = ?4
             WHERE id = ?5 AND status = 'pending'",
            params![status, resolved_by, now, comment, id],
        )
    }

    pub fn update_mod_request_cache(
        &self,
        id: i64,
        mod_name: &str,
        mod_slug: Option<&str>,
        mod_description: Option<&str>,
        fika_compatible: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE mod_requests SET mod_name = ?1, mod_slug = ?2, mod_description = ?3,
                    fika_compatible = ?4, forge_cached_at = datetime('now')
             WHERE id = ?5",
            params![mod_name, mod_slug, mod_description, fika_compatible, id],
        )
    }

    pub fn upsert_vote(
        &self,
        request_id: i64,
        user_id: i64,
        upvote: bool,
        comment: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO mod_request_votes (request_id, user_id, upvote, comment)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(request_id, user_id) DO UPDATE SET upvote = ?3, comment = ?4",
            params![request_id, user_id, upvote as i32, comment],
        )?;
        Ok(())
    }

    pub fn delete_vote(&self, request_id: i64, user_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM mod_request_votes WHERE request_id = ?1 AND user_id = ?2",
            params![request_id, user_id],
        )
    }

    pub fn get_vote(&self, request_id: i64, user_id: i64) -> rusqlite::Result<Option<ModRequestVote>> {
        self.conn
            .query_row(
                "SELECT id, request_id, user_id, upvote, comment, created_at
                 FROM mod_request_votes WHERE request_id = ?1 AND user_id = ?2",
                params![request_id, user_id],
                |row| {
                    Ok(ModRequestVote {
                        id: row.get(0)?,
                        request_id: row.get(1)?,
                        user_id: row.get(2)?,
                        upvote: row.get::<_, i32>(3)? != 0,
                        comment: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    pub fn list_vote_comments(&self, request_id: i64) -> rusqlite::Result<Vec<VoteComment>> {
        let mut stmt = self.conn.prepare(
            "SELECT u.username, v.upvote, v.comment, v.created_at
             FROM mod_request_votes v
             JOIN users u ON v.user_id = u.id
             WHERE v.request_id = ?1 AND v.comment IS NOT NULL AND v.comment != ''
             ORDER BY v.created_at DESC",
        )?;
        let rows = stmt.query_map(params![request_id], |row| {
            Ok(VoteComment {
                username: row.get(0)?,
                upvote: row.get::<_, i32>(1)? != 0,
                comment: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        rows.collect()
    }
}

use rusqlite::OptionalExtension;

fn row_to_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModRequest> {
    Ok(ModRequest {
        id: row.get(0)?,
        user_id: row.get(1)?,
        forge_mod_id: row.get(2)?,
        mod_name: row.get(3)?,
        mod_slug: row.get(4)?,
        mod_description: row.get(5)?,
        fika_compatible: row.get(6)?,
        reason: row.get(7)?,
        status: row.get(8)?,
        resolved_by: row.get(9)?,
        resolved_at: row.get(10)?,
        resolve_comment: row.get(11)?,
        created_at: row.get(12)?,
        forge_cached_at: row.get(13)?,
    })
}

fn row_to_request_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModRequestView> {
    let request = ModRequest {
        id: row.get(0)?,
        user_id: row.get(1)?,
        forge_mod_id: row.get(2)?,
        mod_name: row.get(3)?,
        mod_slug: row.get(4)?,
        mod_description: row.get(5)?,
        fika_compatible: row.get(6)?,
        reason: row.get(7)?,
        status: row.get(8)?,
        resolved_by: row.get(9)?,
        resolved_at: row.get(10)?,
        resolve_comment: row.get(11)?,
        created_at: row.get(12)?,
        forge_cached_at: row.get(13)?,
    };
    let upvote_count: i64 = row.get(15)?;
    let downvote_count: i64 = row.get(16)?;
    Ok(ModRequestView {
        request,
        requester_username: row.get(14)?,
        vote_score: upvote_count - downvote_count,
        upvote_count,
        downvote_count,
        comment_count: row.get(17)?,
        current_user_vote: row.get::<_, Option<i32>>(18)?.map(|v| v != 0),
        resolver_username: row.get(19)?,
    })
}
```

- [ ] **Step 5: Write tests for all DB functions**

Add to `src/db/tests.rs`:

```rust
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
        .create_mod_request(user_id, 100, "Test Mod", Some("test-mod"), Some("A desc"), "unknown", Some("I want this"))
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

    let rows = db.resolve_mod_request(req_id, "approved", admin_id, None).unwrap();
    assert_eq!(rows, 1);

    let rows = db.resolve_mod_request(req_id, "rejected", admin_id, None).unwrap();
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

    db.upsert_vote(req_id, user_id, true, Some("Great mod!")).unwrap();
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

    db.upsert_vote(req_id, user1, true, Some("yes please")).unwrap();
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

    db.upsert_vote(req_id, user1, true, Some("Love it")).unwrap();
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

    db.update_mod_request_cache(req_id, "New Name", Some("new-slug"), Some("New desc"), "compatible")
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

    db.create_mod_request(user_id, 100, "Mod A", None, None, "unknown", None).unwrap();
    db.create_mod_request(user_id, 200, "Mod B", None, None, "unknown", None).unwrap();

    let all = db.list_mod_requests(None, user_id).unwrap();
    assert_eq!(all.len(), 2);

    let pending = db.list_mod_requests(Some("pending"), user_id).unwrap();
    assert_eq!(pending.len(), 2);

    let approved = db.list_mod_requests(Some("approved"), user_id).unwrap();
    assert_eq!(approved.len(), 0);
}
```

- [ ] **Step 6: Run tests to verify**

Run: `cargo test -p quartermaster -- mod_request`

Expected: All tests pass.

- [ ] **Step 7: Run full test suite and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All tests pass, no clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add migrations/007_mod_requests.sql src/db/requests.rs src/db/schema.rs src/db/mod.rs src/db/tests.rs
git commit -m "feat: mod request & voting database layer (migration 007)"
```

---

### Task 2: Config & Forge Cache TTL

**Files:**
- Modify: `src/config.rs` — add `forge_cache_ttl` field, default fn, env override, Default impl

**Interfaces:**
- Consumes: Existing `Config` struct and `apply_env_overrides()` method
- Produces: `Config.forge_cache_ttl: Option<u64>` field (default `Some(86400)`), used by Task 4 handlers for staleness checks

- [ ] **Step 1: Add default function**

In `src/config.rs`, add after `default_update_check_interval` function (after line 29):

```rust
fn default_forge_cache_ttl() -> Option<u64> {
    Some(86400)
}
```

- [ ] **Step 2: Add field to Config struct**

In `src/config.rs`, add after the `update_check_interval` field (after line 309):

```rust
    #[serde(default = "default_forge_cache_ttl")]
    pub forge_cache_ttl: Option<u64>,
```

- [ ] **Step 3: Add to Default impl**

In `src/config.rs`, in the `Default` impl, add after `update_check_interval: 300,` (after line 335):

```rust
            forge_cache_ttl: Some(86400),
```

- [ ] **Step 4: Add env override**

In `src/config.rs`, in `apply_env_overrides()`, add after the `QUMA_UPDATE_CHECK_INTERVAL` block (after line 449):

```rust
        if let Ok(val) = std::env::var("QUMA_FORGE_CACHE_TTL") {
            if let Ok(secs) = val.parse::<u64>() {
                tracing::debug!(var = "QUMA_FORGE_CACHE_TTL", value = %val, "env var override applied");
                self.forge_cache_ttl = Some(secs);
            }
        }
```

- [ ] **Step 5: Run tests and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All pass. Existing config tests still pass (serde defaults handle the new field).

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat: add forge_cache_ttl config option (default 24h)"
```

---

### Task 3: Web Handlers — Request CRUD, Search, Voting, Resolution

**Files:**
- Create: `src/web/handlers/requests.rs`
- Modify: `src/web/handlers/mod.rs` — add `pub mod requests;`
- Modify: `src/web/mod.rs` — add routes for all request endpoints
- Modify: `src/web/handlers/mods.rs` — remove `require_capability` from `list_mods` so Players can access `/mods` (the template already guards admin-only UI with `{% if user.role.can_manage_mods() %}`)

**Interfaces:**
- Consumes:
  - From Task 1: All `Database` methods in `src/db/requests.rs`, `ModRequest`, `ModRequestView`, `VoteComment`
  - From Task 2: `Config.forge_cache_ttl`
  - Existing: `AppState`, `SessionUser`, `require_auth()`, `require_capability()`, `WebError`, `FlashMessage`, `set_flash()`, `take_flash()`, CSRF functions, `ForgeClient::search_mods()`, `ForgeClient::get_mod()`, `ForgeClient::get_versions()`, `queue::should_queue()`, `Database::insert_pending_op()`, `Database::get_mod_by_forge_id()`
- Produces: All handler functions used by route registration in `src/web/mod.rs` and Askama template structs used by Task 4 templates:
  - `requests_tab(state, req, session, query) -> Html` — HTMX partial for requests list
  - `search_mods(state, req, query) -> Html` — Forge search proxy returning search result cards
  - `create_request(state, req, session, form) -> Result<HttpResponse>` — create a mod request
  - `vote(state, req, session, path, form) -> Html` — cast/change/remove vote
  - `vote_comments(state, req, path) -> Html` — list vote comments
  - `resolve_request(state, req, session, path, form) -> Html` — approve/reject
  - Template structs: `RequestsTabTemplate`, `SearchResultsTemplate`, `RequestCardTemplate`, `VoteCommentsTemplate`
  - Helper: `parse_forge_url(input: &str) -> Option<i64>` — extract mod ID from Forge URLs

- [ ] **Step 1: Remove `can_manage_mods` gate from `list_mods` handler**

In `src/web/handlers/mods.rs`, line 117, remove the `require_capability` call so Players can access the `/mods` page (needed for the Requests tab). The template already gates admin-only UI with `{% if user.role.can_manage_mods() %}`.

Remove this line:
```rust
    require_capability(&user, Role::can_manage_mods)?;
```

The handler should go from:
```rust
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    let flash = take_flash(&session);
```

To:
```rust
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
```

Also remove the unused import `Role` from line 7 if it becomes unused (check — it may still be used by other handlers in the file via `Role::can_manage_mods` in `require_capability` calls in `install_mod`, `update_mod`, `remove_mod`, etc. — only remove if unused).

- [ ] **Step 2: Add module declaration**

In `src/web/handlers/mod.rs`, add:

```rust
pub mod requests;
```

- [ ] **Step 3: Write the handlers file**

Create `src/web/handlers/requests.rs`:

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::requests::{ModRequestView, VoteComment};
use crate::forge::models::FikaCompat;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::flash::set_flash;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

// -- Query / Form structs --

#[derive(serde::Deserialize)]
pub struct StatusQuery {
    pub status: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CreateRequestForm {
    pub forge_mod_id: i64,
    pub reason: Option<String>,
    pub csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct VoteForm {
    pub upvote: String,
    pub comment: Option<String>,
    pub csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct ResolveForm {
    pub action: String,
    pub comment: Option<String>,
    pub install: Option<String>,
    pub csrf_token: String,
}

// -- Templates --

#[derive(Template)]
#[template(path = "mods/partials/requests.html")]
struct RequestsTabTemplate {
    user: SessionUser,
    requests: Vec<ModRequestView>,
    active_filter: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/partials/search_results.html")]
struct SearchResultsTemplate {
    results: Vec<SearchResult>,
    error: Option<String>,
}

pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub fika_compatible: String,
}

#[derive(Template)]
#[template(path = "mods/partials/request_card.html")]
struct RequestCardTemplate {
    user: SessionUser,
    r: ModRequestView,
    csrf_token: String,
    message: Option<String>,
}

#[derive(Template)]
#[template(path = "mods/partials/vote_comments.html")]
struct VoteCommentsTemplate {
    comments: Vec<VoteComment>,
}

// -- Helpers --

pub fn parse_forge_url(input: &str) -> Option<i64> {
    let input = input.trim();
    if let Ok(id) = input.parse::<i64>() {
        return Some(id);
    }
    if input.contains("forge.sp-tarkov.com") {
        // Strip query parameters before parsing
        let url_path = input.split('?').next().unwrap_or(input);
        let parts: Vec<&str> = url_path.split('/').collect();
        if let Some(segment) = parts.iter().rev().find(|s| !s.is_empty()) {
            if let Some(id_str) = segment.split('-').next() {
                if let Ok(id) = id_str.parse::<i64>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

fn fika_compat_to_string(fc: &Option<FikaCompat>) -> String {
    match fc {
        Some(FikaCompat::Compatible) => "compatible".to_string(),
        Some(FikaCompat::Incompatible) => "incompatible".to_string(),
        _ => "unknown".to_string(),
    }
}

fn is_cache_stale(forge_cached_at: &str, ttl_secs: u64) -> bool {
    use chrono::{NaiveDateTime, Utc};
    let cached = NaiveDateTime::parse_from_str(forge_cached_at, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc())
        .unwrap_or_else(|_| {
            chrono::DateTime::parse_from_rfc3339(forge_cached_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        });
    let age = Utc::now().signed_duration_since(cached);
    age.num_seconds() > ttl_secs as i64
}

// -- Handlers --

pub async fn requests_tab(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<StatusQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = csrf::get_or_create_token(&session);

    let filter = query.status.clone().unwrap_or_else(|| "pending".to_string());
    let filter_param = if filter == "all" { None } else { Some(filter.as_str()) };

    let db = state.db.clone();
    let user_id = user.user_id;
    let filter_owned = filter_param.map(|s| s.to_string());
    let requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(filter_owned.as_deref(), user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // Spawn background cache refresh for stale entries
    if let Some(ttl) = state.config.forge_cache_ttl {
        for rv in &requests {
            if is_cache_stale(&rv.request.forge_cached_at, ttl) {
                let db = state.db.clone();
                let forge = state.forge.clone();
                let request_id = rv.request.id;
                let forge_mod_id = rv.request.forge_mod_id;
                tokio::spawn(async move {
                    match forge.get_mod(forge_mod_id, false).await {
                        Ok(m) => {
                            let fc = fika_compat_to_string(&m.fika_compatibility);
                            let _ = web::block(move || {
                                let db = db.lock();
                                db.update_mod_request_cache(
                                    request_id,
                                    &m.name,
                                    m.slug.as_deref(),
                                    m.description.as_deref(),
                                    &fc,
                                )
                            })
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                forge_mod_id,
                                error = %e,
                                "failed to refresh Forge cache for mod request"
                            );
                        }
                    }
                });
            }
        }
    }

    let tmpl = RequestsTabTemplate {
        user,
        requests,
        active_filter: filter,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn search_mods(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<SearchQuery>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let q = query.q.as_deref().unwrap_or("").trim().to_string();

    if q.len() < 2 {
        let tmpl = SearchResultsTemplate {
            results: vec![],
            error: None,
        };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    // Check for direct ID or URL
    if let Some(mod_id) = parse_forge_url(&q) {
        match state.forge.get_mod(mod_id, false).await {
            Ok(m) => {
                let tmpl = SearchResultsTemplate {
                    results: vec![SearchResult {
                        id: m.id,
                        name: m.name,
                        slug: m.slug,
                        description: m.description,
                        fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                    }],
                    error: None,
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
            Err(_) => {
                let tmpl = SearchResultsTemplate {
                    results: vec![],
                    error: Some(format!("Mod with ID {mod_id} not found on Forge.")),
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
        }
    }

    match state.forge.search_mods(&q).await {
        Ok(mods) => {
            let results = mods
                .into_iter()
                .map(|m| SearchResult {
                    id: m.id,
                    name: m.name,
                    slug: m.slug,
                    description: m.description,
                    fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                })
                .collect();
            let tmpl = SearchResultsTemplate {
                results,
                error: None,
            };
            Ok(Html::new(tmpl.render().map_err(WebError::from)?))
        }
        Err(_) => {
            let tmpl = SearchResultsTemplate {
                results: vec![],
                error: Some("Could not reach SPT Forge. Try again later.".to_string()),
            };
            Ok(Html::new(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn create_request(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<CreateRequestForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let forge_mod_id = form.forge_mod_id;
    let csrf_token = csrf::get_or_create_token(&session);
    let user_id = user.user_id;

    // Check if mod is already installed
    let db = state.db.clone();
    let is_installed = web::block(move || {
        let db = db.lock();
        Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(forge_mod_id)?.is_some())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if is_installed {
        return Err(WebError::BadRequest("This mod is already installed.".to_string()).into());
    }

    // Check for existing pending request
    let db = state.db.clone();
    let has_pending = web::block(move || {
        let db = db.lock();
        db.has_pending_request_for_mod(forge_mod_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if has_pending {
        return Err(WebError::BadRequest("A pending request for this mod already exists.".to_string()).into());
    }

    // Fetch fresh mod info from Forge
    let mod_info = state
        .forge
        .get_mod(forge_mod_id, false)
        .await
        .map_err(|_| {
            WebError::BadRequest("Could not verify mod on SPT Forge.".to_string())
        })?;

    let fc = fika_compat_to_string(&mod_info.fika_compatibility);
    let reason = form.reason.as_deref().filter(|s| !s.trim().is_empty());
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();
    let mod_desc = mod_info.description.clone();

    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.create_mod_request(
            user_id,
            forge_mod_id,
            &mod_name,
            mod_slug.as_deref(),
            mod_desc.as_deref(),
            &fc,
            reason,
        )
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // Return the updated requests tab
    let db = state.db.clone();
    let requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(Some("pending"), user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = RequestsTabTemplate {
        user,
        requests,
        active_filter: "pending".to_string(),
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn vote(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<VoteForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let request_id = path.into_inner();
    let upvote = form.upvote == "true";
    let comment = form.comment.as_deref().filter(|s| !s.trim().is_empty());

    // Check request exists and is pending
    let db = state.db.clone();
    let request = web::block({
        let db = db.clone();
        move || {
            let db = db.lock();
            db.get_mod_request(request_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    if request.status != "pending" {
        return Err(WebError::BadRequest("Voting is only allowed on pending requests.".to_string()).into());
    }

    // Check if user already voted the same way (toggle off)
    let db = state.db.clone();
    let user_id = user.user_id;
    let existing_vote = web::block({
        let db = db.clone();
        move || {
            let db = db.lock();
            db.get_vote(request_id, user_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let db = state.db.clone();
    if let Some(existing) = existing_vote {
        if existing.upvote == upvote {
            // Toggle off — remove vote
            web::block(move || {
                let db = db.lock();
                db.delete_vote(request_id, user_id)
            })
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;
        } else {
            // Change vote direction
            let comment_owned = comment.map(|s| s.to_string());
            web::block(move || {
                let db = db.lock();
                db.upsert_vote(request_id, user_id, upvote, comment_owned.as_deref())
            })
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;
        }
    } else {
        // New vote
        let comment_owned = comment.map(|s| s.to_string());
        web::block(move || {
            let db = db.lock();
            db.upsert_vote(request_id, user_id, upvote, comment_owned.as_deref())
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    }

    // Re-fetch the request view to render the updated card
    let db = state.db.clone();
    let views = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rv = views
        .into_iter()
        .find(|v| v.request.id == request_id)
        .ok_or(WebError::NotFound)?;

    let csrf_token = csrf::get_or_create_token(&session);
    let tmpl = RequestCardTemplate {
        user,
        r: rv,
        csrf_token,
        message: None,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn vote_comments(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let request_id = path.into_inner();

    let db = state.db.clone();
    let comments = web::block(move || {
        let db = db.lock();
        db.list_vote_comments(request_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = VoteCommentsTemplate { comments };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn resolve_request(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<ResolveForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_capability(&user, crate::db::users::Role::can_manage_mods)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let request_id = path.into_inner();
    let action = form.action.as_str();
    if action != "approve" && action != "reject" {
        return Err(WebError::BadRequest("Invalid action.".to_string()).into());
    }

    let status = if action == "approve" { "approved" } else { "rejected" };
    let comment = form.comment.as_deref().filter(|s| !s.trim().is_empty());

    // Resolve the request (only if pending)
    let db = state.db.clone();
    let resolved_by = user.user_id;
    let comment_owned = comment.map(|s| s.to_string());
    let status_owned = status.to_string();
    let rows = web::block({
        let db = db.clone();
        let status_owned = status_owned.clone();
        let comment_owned = comment_owned.clone();
        move || {
            let db = db.lock();
            db.resolve_mod_request(request_id, &status_owned, resolved_by, comment_owned.as_deref())
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if rows == 0 {
        return Err(WebError::BadRequest("This request has already been resolved.".to_string()).into());
    }

    let mut message = if action == "approve" {
        "Request approved.".to_string()
    } else {
        "Request rejected.".to_string()
    };

    // Install-on-approve
    if action == "approve" && form.install.as_deref() == Some("true") {
        let request = web::block({
            let db = db.clone();
            move || {
                let db = db.lock();
                db.get_mod_request(request_id)
            }
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?
        .ok_or(WebError::NotFound)?;

        let forge_mod_id = request.forge_mod_id;

        match state
            .forge
            .get_versions(forge_mod_id, Some(&state.spt_info.spt_version))
            .await
        {
            Ok(versions) if !versions.is_empty() => {
                let version = versions.last().unwrap();
                let should_queue = crate::queue::should_queue(
                    &state.config,
                    false,
                    &state.spt_dir,
                    state.container_mgr.as_deref(),
                )
                .await
                .unwrap_or(false);

                if should_queue {
                    let db = state.db.clone();
                    let mod_name = request.mod_name.clone();
                    let version_id = version.id;
                    let username = user.username.clone();
                    web::block(move || {
                        let db = db.lock();
                        db.insert_pending_op(
                            "install",
                            forge_mod_id,
                            Some(version_id),
                            &mod_name,
                            None,
                            Some(&username),
                        )
                    })
                    .await
                    .map_err(WebError::from)?
                    .map_err(WebError::from)?;
                    message = "Approved and queued for install.".to_string();
                } else {
                    // Direct install via async task (same pattern as mods::install_mod)
                    let task_id = state
                        .tasks
                        .start("Installing", &request.mod_name, forge_mod_id);
                    let tasks = state.tasks.clone();
                    let forge = state.forge.clone();
                    let spt_dir = state.spt_dir.clone();
                    let db = state.db.clone();
                    let version = version.clone();
                    let mod_name = request.mod_name.clone();
                    let mod_slug = request.mod_slug.clone();
                    let update_cache = state.update_cache.clone();

                    tokio::spawn(async move {
                        let result = async {
                            let link = version
                                .link
                                .as_deref()
                                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
                            let tmp_dir = tempfile::tempdir()?;
                            let archive_path = tmp_dir.path().join("mod.zip");
                            forge.download_file(link, &archive_path).await?;

                            let spt_dir2 = spt_dir.clone();
                            let extracted = actix_web::web::block(move || {
                                crate::spt::mods::extract_mod(&archive_path, &spt_dir2)
                            })
                            .await??;

                            let version_id = version.id;
                            let version_str = version.version.clone();
                            let spt_dir2 = spt_dir.clone();
                            let db2 = db.clone();
                            let db_id = actix_web::web::block(move || {
                                let db = db.lock();
                                let db_id = db.insert_mod(
                                    forge_mod_id,
                                    version_id,
                                    &mod_name,
                                    mod_slug.as_deref(),
                                    &version_str,
                                )?;
                                for file in &extracted {
                                    db.insert_file(
                                        db_id,
                                        &file.path,
                                        Some(&file.hash),
                                        Some(file.size as i64),
                                    )?;
                                }
                                Ok::<_, anyhow::Error>(db_id)
                            })
                            .await??;

                            let _ = actix_web::web::block(move || {
                                crate::ops::scan_and_record_runtime_files(&db2, db_id, &spt_dir2)
                            })
                            .await;

                            Ok::<_, anyhow::Error>(())
                        }
                        .await;

                        match result {
                            Ok(()) => {
                                tracing::info!(forge_mod_id, "mod installed via request approval");
                                update_cache.invalidate();
                                tasks.complete(task_id, "Mod installed successfully".to_string());
                            }
                            Err(e) => {
                                tracing::error!(forge_mod_id, error = %e, "install from request approval failed");
                                tasks.fail(task_id, format!("Install failed: {e}"));
                            }
                        }
                    });
                    message = "Approved and installing now.".to_string();
                }
            }
            Ok(_) => {
                message = format!(
                    "Approved but no compatible version found for SPT {}.",
                    state.spt_info.spt_version
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to fetch versions for install-on-approve");
                message = "Approved. Could not fetch versions for auto-install.".to_string();
            }
        }
    }

    // Re-fetch the request view
    let db = state.db.clone();
    let user_id = user.user_id;
    let views = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rv = views
        .into_iter()
        .find(|v| v.request.id == request_id)
        .ok_or(WebError::NotFound)?;

    let csrf_token = csrf::get_or_create_token(&session);
    let tmpl = RequestCardTemplate {
        user,
        r: rv,
        csrf_token,
        message: Some(message),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 4: Add routes to src/web/mod.rs**

In the `/api` scope (after the log routes around line 195, before the admin scope), add a new Governor config and the request routes:

First, add a second Governor config for search OUTSIDE the `HttpServer::new` closure, right after the existing `governor_conf` definition (after line 89):

```rust
    let search_governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(6) // 10 per minute = 1 per 6 seconds replenish
        .burst_size(10)
        .finish()
        .expect("invalid search governor config");
```

Then in the `/api` scope, after the log stream routes (after line 195), add:

```rust
                    // Mod request routes
                    .service(
                        web::resource("/requests/search")
                            .wrap(Governor::new(&search_governor_conf))
                            .route(web::get().to(handlers::requests::search_mods)),
                    )
                    .route(
                        "/mods/requests",
                        web::get().to(handlers::requests::requests_tab),
                    )
                    .route("/requests", web::post().to(handlers::requests::create_request))
                    .route(
                        "/requests/{id}/vote",
                        web::post().to(handlers::requests::vote),
                    )
                    .route(
                        "/requests/{id}/votes",
                        web::get().to(handlers::requests::vote_comments),
                    )
                    .route(
                        "/requests/{id}/resolve",
                        web::post().to(handlers::requests::resolve_request),
                    )
```

- [ ] **Step 5: Run tests and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All pass. (Templates don't exist yet so the compile will fail here — that's expected. If using Askama compile-time checks, create stub template files first. See Task 4.)

Note: This task should create minimal stub template files to satisfy the Askama compile-time checker:

Create these stub files (empty or with minimal content) so the handler compiles:

`templates/mods/partials/requests.html`:
```html
requests placeholder
```

`templates/mods/partials/search_results.html`:
```html
search placeholder
```

`templates/mods/partials/request_card.html`:
```html
card placeholder
```

`templates/mods/partials/vote_comments.html`:
```html
comments placeholder
```

- [ ] **Step 6: Commit**

```bash
git add src/web/handlers/requests.rs src/web/handlers/mod.rs src/web/mod.rs src/web/handlers/mods.rs templates/mods/partials/requests.html templates/mods/partials/search_results.html templates/mods/partials/request_card.html templates/mods/partials/vote_comments.html
git commit -m "feat: mod request web handlers — search, create, vote, resolve"
```

---

### Task 4: Templates — Mods Page Tabs, Request Cards, Search, Voting UI

**Files:**
- Modify: `templates/mods/list.html` — add tab bar wrapping existing content
- Create: `templates/mods/partials/requests.html` — request list with filters and create button
- Create: `templates/mods/partials/request_form.html` — search + create form (included in requests.html)
- Create: `templates/mods/partials/request_card.html` — single request card for HTMX swap
- Create: `templates/mods/partials/search_results.html` — Forge search result cards
- Create: `templates/mods/partials/vote_comments.html` — expandable vote comment list

**Interfaces:**
- Consumes: Template structs from Task 3: `RequestsTabTemplate`, `SearchResultsTemplate`, `RequestCardTemplate`, `VoteCommentsTemplate`
- Produces: Complete UI for the mod request feature

- [ ] **Step 1: Add tab bar to mods list page**

Replace the content of `templates/mods/list.html` to wrap existing content in tabs. The key change: add a tab bar before the existing content, wrap existing content in an `#installed-content` div, add a `#requests-content` div for HTMX loading.

Replace `templates/mods/list.html`:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% import "partials/icons.html" as icons %}
{% block title %}Mods — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("mods", user, csrf_token, fika_installed) %}{% endcall %}{% endblock %}
{% block content %}
<div hx-get="/api/tasks/status" hx-trigger="load" hx-swap="outerHTML" id="task-status"></div>

{% if let Some(flash) = flash %}
<div class="alert alert-{{ flash.flash_type }}">{{ flash.message }}</div>
{% endif %}

<div class="tab-bar">
    <button class="tab active" id="tab-installed"
            onclick="setModTab(this, 'installed')">Installed</button>
    <button class="tab" id="tab-requests"
            hx-get="/api/mods/requests"
            hx-target="#mods-tab-content"
            hx-swap="innerHTML"
            onclick="setModTab(this, 'requests')">Requests</button>
</div>

<div id="mods-tab-content">
    <div class="flex-between">
        <h2>Installed Mods</h2>
        {% if user.role.can_manage_mods() %}
        <div class="flex gap-1">
            <form method="post" action="/mods/update-all" style="display:inline"
                  onsubmit="return confirm('Update all mods?')">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <button type="submit" class="btn btn-sm btn-outline">{% call icons::refresh() %}{% endcall %} Update All</button>
            </form>
        </div>
        {% endif %}
    </div>

    <div class="card" style="padding: 0.75rem 1.25rem">
        <div class="flex-between">
            <span>
                <strong>SPT {{ spt_version }}</strong>
                <span class="text-muted text-sm" style="margin-left: 1rem">Tarkov {{ tarkov_version }}</span>
            </span>
            <a href="https://forge.sp-tarkov.com" class="btn btn-sm btn-outline" target="_blank" rel="noopener">SPT Forge ↗</a>
        </div>
    </div>

    {% if mods.is_empty() %}
    <div class="card">
        <div class="empty-state">
            <p>No mods installed.</p>
            {% if user.role.can_manage_mods() %}<p class="mt-1">Use the form below to install your first mod.</p>{% endif %}
        </div>
    </div>
    {% else %}
    <div class="card">
        <table>
            <thead>
                <tr>
                    <th>Name</th>
                    <th>Version</th>
                    <th>Files</th>
                    <th>Size</th>
                    <th>Installed</th>
                    {% if user.role.can_manage_mods() %}<th>Actions</th>{% endif %}
                </tr>
            </thead>
            <tbody id="mod-list-body"
                   hx-get="/api/mods/list"
                   hx-trigger="sse:modsChanged"
                   hx-swap="innerHTML">
                {% for m in &mods %}
                <tr>
                    <td><a href="/mods/{{ m.mod_info.id }}">{{ m.mod_info.name }}</a></td>
                    <td><span id="mod-version-{{ m.mod_info.id }}">{{ m.mod_info.version }}</span></td>
                    <td class="text-muted">{{ m.file_count }}</td>
                    <td class="text-muted">{{ m.total_size|format_size_i64 }}</td>
                    <td class="text-muted text-sm">{{ m.mod_info.installed_at }}</td>
                    {% if user.role.can_manage_mods() %}
                    <td>
                        <span id="mod-update-{{ m.mod_info.id }}">
                            <form method="post" action="/mods/{{ m.mod_info.id }}/update" style="display:inline">
                                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                                <button type="submit" class="btn btn-sm btn-outline">{% call icons::refresh() %}{% endcall %} Update</button>
                            </form>
                        </span>
                        <form method="post" action="/mods/{{ m.mod_info.id }}/remove" style="display:inline"
                              data-name="{{ m.mod_info.name }}"
                              onsubmit="return confirm('Remove ' + this.dataset.name + '?')">
                            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                            <button type="submit" class="btn btn-sm btn-danger">{% call icons::trash() %}{% endcall %} Remove</button>
                        </form>
                    </td>
                    {% endif %}
                </tr>
                {% endfor %}
                <tr class="tfoot-row">
                    <td colspan="{% if user.role.can_manage_mods() %}6{% else %}5{% endif %}" class="text-muted text-sm">
                        Total: {{ mods.len() }} mods — {{ grand_total_size|format_size_i64 }}
                    </td>
                </tr>
            </tbody>
        </table>
        <span hx-get="/api/mods/update-status" hx-trigger="load, sse:modsChanged" hx-swap="none"></span>
    </div>
    {% endif %}

    {% if user.role.can_manage_mods() %}
    <div class="card">
        <h2>Install Mod</h2>
        <form method="post" action="/mods/install">
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
            <label for="mod_ref">Forge Mod ID or Name</label>
            <input type="text" id="mod_ref" name="mod_ref" placeholder="e.g. 2326 or SAIN" required>
            <button type="submit" class="btn">{% call icons::download() %}{% endcall %} Install</button>
        </form>
    </div>
    {% endif %}
</div>

<script>
function setModTab(el, tab) {
    document.querySelectorAll('.tab-bar .tab').forEach(t => t.classList.remove('active'));
    el.classList.add('active');
    location.hash = tab;
}
document.addEventListener('DOMContentLoaded', function() {
    if (location.hash === '#requests') {
        var btn = document.getElementById('tab-requests');
        if (btn) { btn.click(); }
    }
});
</script>
{% endblock %}
```

- [ ] **Step 2: Create the requests tab partial**

Create `templates/mods/partials/requests.html`:

```html
<div class="status-filter mb-1">
    <button class="btn btn-sm {% if active_filter == "pending" %}btn-outline active{% else %}btn-outline{% endif %}"
            hx-get="/api/mods/requests?status=pending"
            hx-target="#mods-tab-content"
            hx-swap="innerHTML">Pending</button>
    <button class="btn btn-sm {% if active_filter == "approved" %}btn-outline active{% else %}btn-outline{% endif %}"
            hx-get="/api/mods/requests?status=approved"
            hx-target="#mods-tab-content"
            hx-swap="innerHTML">Approved</button>
    <button class="btn btn-sm {% if active_filter == "rejected" %}btn-outline active{% else %}btn-outline{% endif %}"
            hx-get="/api/mods/requests?status=rejected"
            hx-target="#mods-tab-content"
            hx-swap="innerHTML">Rejected</button>
    <button class="btn btn-sm {% if active_filter == "all" %}btn-outline active{% else %}btn-outline{% endif %}"
            hx-get="/api/mods/requests?status=all"
            hx-target="#mods-tab-content"
            hx-swap="innerHTML">All</button>
</div>

<div class="card" id="request-form-area">
    <h3>Request a Mod</h3>
    <div>
        <input type="text" id="mod-search-input" placeholder="Search Forge by name, paste a URL, or enter a mod ID..."
               hx-get="/api/requests/search"
               hx-trigger="input changed delay:300ms"
               hx-target="#search-results"
               hx-swap="innerHTML"
               hx-indicator="#search-spinner"
               name="q"
               autocomplete="off"
               style="width:100%;margin-bottom:0.5rem">
        <span id="search-spinner" class="htmx-indicator text-muted text-sm">Searching...</span>
    </div>
    <div id="search-results"></div>
    <form id="request-create-form" style="display:none"
          hx-post="/api/requests"
          hx-target="#mods-tab-content"
          hx-swap="innerHTML">
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
        <input type="hidden" name="forge_mod_id" id="selected-mod-id">
        <div id="selected-mod-preview" class="card" style="padding:0.75rem;margin:0.5rem 0"></div>
        <label for="request-reason">Why do you want this mod? (optional)</label>
        <textarea name="reason" id="request-reason" rows="2" placeholder="Tell the admins why this mod would be great..."
                  style="width:100%;background:var(--bg-input);border:1px solid var(--border);border-radius:var(--radius);color:var(--text);padding:0.5rem;font-size:0.9rem;resize:vertical"></textarea>
        <button type="submit" class="btn mt-1">Submit Request</button>
    </form>
</div>

{% if requests.is_empty() %}
<div class="card">
    <div class="empty-state">
        <p>No {{ active_filter }} requests yet.</p>
    </div>
</div>
{% else %}
{% for r in &requests %}
<div id="request-{{ r.request.id }}">
<div class="request-card card">
    <div class="flex-between">
        <div>
            {% if let Some(slug) = r.request.mod_slug.as_deref() %}
            <a href="https://forge.sp-tarkov.com/mods/{{ r.request.forge_mod_id }}-{{ slug }}" target="_blank" rel="noopener">
                <strong>{{ r.request.mod_name }}</strong> ↗
            </a>
            {% else %}
            <strong>{{ r.request.mod_name }}</strong>
            {% endif %}
            {% if r.request.fika_compatible == "compatible" %}
            <span class="badge badge-success" style="margin-left:0.5rem">Fika ✓</span>
            {% elif r.request.fika_compatible == "incompatible" %}
            <span class="badge badge-danger" style="margin-left:0.5rem">Fika ✗</span>
            {% endif %}
        </div>
        <div>
            {% if r.request.status == "pending" %}
            <span class="badge badge-muted">Pending</span>
            {% elif r.request.status == "approved" %}
            <span class="badge badge-success">Approved</span>
            {% else %}
            <span class="badge badge-danger">Rejected</span>
            {% endif %}
        </div>
    </div>
    {% if let Some(desc) = r.request.mod_description.as_deref() %}
    <p class="text-muted text-sm mt-1" style="max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{{ desc }}</p>
    {% endif %}
    {% if let Some(reason) = r.request.reason.as_deref() %}
    <p class="text-sm mt-1" style="font-style:italic;color:var(--text-muted)">"{{ reason }}"</p>
    {% endif %}
    <div class="flex-between mt-1">
        <span class="text-muted text-sm">by {{ r.requester_username }} · {{ r.request.created_at }}</span>
        <div class="vote-buttons flex gap-1" style="align-items:center;flex-wrap:wrap">
            {% if r.request.status == "pending" %}
            <form hx-post="/api/requests/{{ r.request.id }}/vote" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML" style="display:inline">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <input type="hidden" name="upvote" value="true">
                <input type="hidden" name="comment" class="vote-comment-field" value="">
                <button type="submit" class="btn btn-sm {% if r.current_user_vote == Some(true) %}btn-success{% else %}btn-outline{% endif %}">▲</button>
            </form>
            <span class="vote-score" style="font-weight:700;min-width:2ch;text-align:center">{{ r.vote_score }}</span>
            <form hx-post="/api/requests/{{ r.request.id }}/vote" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML" style="display:inline">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <input type="hidden" name="upvote" value="false">
                <input type="hidden" name="comment" class="vote-comment-field" value="">
                <button type="submit" class="btn btn-sm {% if r.current_user_vote == Some(false) %}btn-danger{% else %}btn-outline{% endif %}">▼</button>
            </form>
            <button type="button" class="btn btn-sm btn-outline" onclick="toggleVoteComment(this, {{ r.request.id }})">💬</button>
            {% else %}
            <span class="vote-score text-muted" style="font-weight:700">{{ r.vote_score }}</span>
            {% endif %}
            <span class="text-muted text-sm" style="margin-left:0.5rem">{{ r.upvote_count }}↑ {{ r.downvote_count }}↓</span>
            {% if r.comment_count > 0 %}
            <button class="btn btn-sm btn-outline" style="margin-left:0.25rem"
                    hx-get="/api/requests/{{ r.request.id }}/votes"
                    hx-target="#comments-{{ r.request.id }}"
                    hx-swap="innerHTML">{{ r.comment_count }} comment{% if r.comment_count != 1 %}s{% endif %}</button>
            {% endif %}
            <div class="vote-comment-input" id="vote-comment-{{ r.request.id }}" style="display:none;width:100%;margin-top:0.25rem">
                <input type="text" placeholder="Add a comment with your vote..." style="width:100%;font-size:0.85rem"
                       oninput="updateVoteComments(this, {{ r.request.id }})">
            </div>
        </div>
    </div>
    {% if let Some(resolver) = r.resolver_username.as_deref() %}
    <div class="text-sm text-muted mt-1">
        Resolved by {{ resolver }} · {{ r.request.resolved_at.as_deref().unwrap_or("") }}
        {% if let Some(rc) = r.request.resolve_comment.as_deref() %}
        — "{{ rc }}"
        {% endif %}
    </div>
    {% endif %}
    {% if r.request.status == "pending" && user.role.can_manage_mods() %}
    <div class="mt-1 resolve-form" style="display:flex;gap:0.5rem;align-items:flex-start;flex-wrap:wrap">
        <form hx-post="/api/requests/{{ r.request.id }}/resolve" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML"
              style="display:flex;gap:0.5rem;align-items:center;flex-wrap:wrap">
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
            <input type="hidden" name="action" value="approve">
            <input type="text" name="comment" placeholder="Comment (optional)" style="flex:1;min-width:150px" class="text-sm">
            <label class="text-sm" style="display:flex;align-items:center;gap:0.25rem;cursor:pointer">
                <input type="checkbox" name="install" value="true"> Install now
            </label>
            <button type="submit" class="btn btn-sm btn-success">Approve</button>
        </form>
        <form hx-post="/api/requests/{{ r.request.id }}/resolve" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML"
              style="display:inline">
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
            <input type="hidden" name="action" value="reject">
            <button type="submit" class="btn btn-sm btn-danger">Reject</button>
        </form>
    </div>
    {% endif %}
    <div id="comments-{{ r.request.id }}"></div>
</div>
</div>
{% endfor %}
{% endif %}
<script>
function toggleVoteComment(btn, requestId) {
    var el = document.getElementById('vote-comment-' + requestId);
    el.style.display = el.style.display === 'none' ? 'block' : 'none';
    if (el.style.display === 'block') el.querySelector('input').focus();
}
function updateVoteComments(input, requestId) {
    var card = document.getElementById('request-' + requestId);
    card.querySelectorAll('.vote-comment-field').forEach(function(f) { f.value = input.value; });
}
</script>
```

- [ ] **Step 3: Create the request card partial (for HTMX swap after vote/resolve)**

Create `templates/mods/partials/request_card.html`:

```html
<div id="request-{{ r.request.id }}">
{% if let Some(msg) = message.as_deref() %}
<div class="toast toast-success" style="margin-bottom:0.5rem">{{ msg }}</div>
{% endif %}
<div class="request-card card">
    <div class="flex-between">
        <div>
            {% if let Some(slug) = r.request.mod_slug.as_deref() %}
            <a href="https://forge.sp-tarkov.com/mods/{{ r.request.forge_mod_id }}-{{ slug }}" target="_blank" rel="noopener">
                <strong>{{ r.request.mod_name }}</strong> ↗
            </a>
            {% else %}
            <strong>{{ r.request.mod_name }}</strong>
            {% endif %}
            {% if r.request.fika_compatible == "compatible" %}
            <span class="badge badge-success" style="margin-left:0.5rem">Fika ✓</span>
            {% elif r.request.fika_compatible == "incompatible" %}
            <span class="badge badge-danger" style="margin-left:0.5rem">Fika ✗</span>
            {% endif %}
        </div>
        <div>
            {% if r.request.status == "pending" %}
            <span class="badge badge-muted">Pending</span>
            {% elif r.request.status == "approved" %}
            <span class="badge badge-success">Approved</span>
            {% else %}
            <span class="badge badge-danger">Rejected</span>
            {% endif %}
        </div>
    </div>
    {% if let Some(desc) = r.request.mod_description.as_deref() %}
    <p class="text-muted text-sm mt-1" style="max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{{ desc }}</p>
    {% endif %}
    {% if let Some(reason) = r.request.reason.as_deref() %}
    <p class="text-sm mt-1" style="font-style:italic;color:var(--text-muted)">"{{ reason }}"</p>
    {% endif %}
    <div class="flex-between mt-1">
        <span class="text-muted text-sm">by {{ r.requester_username }} · {{ r.request.created_at }}</span>
        <div class="vote-buttons flex gap-1" style="align-items:center;flex-wrap:wrap">
            {% if r.request.status == "pending" %}
            <form hx-post="/api/requests/{{ r.request.id }}/vote" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML" style="display:inline">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <input type="hidden" name="upvote" value="true">
                <input type="hidden" name="comment" class="vote-comment-field" value="">
                <button type="submit" class="btn btn-sm {% if r.current_user_vote == Some(true) %}btn-success{% else %}btn-outline{% endif %}">▲</button>
            </form>
            <span class="vote-score" style="font-weight:700;min-width:2ch;text-align:center">{{ r.vote_score }}</span>
            <form hx-post="/api/requests/{{ r.request.id }}/vote" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML" style="display:inline">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <input type="hidden" name="upvote" value="false">
                <input type="hidden" name="comment" class="vote-comment-field" value="">
                <button type="submit" class="btn btn-sm {% if r.current_user_vote == Some(false) %}btn-danger{% else %}btn-outline{% endif %}">▼</button>
            </form>
            <button type="button" class="btn btn-sm btn-outline" onclick="toggleVoteComment(this, {{ r.request.id }})">💬</button>
            {% else %}
            <span class="vote-score text-muted" style="font-weight:700">{{ r.vote_score }}</span>
            {% endif %}
            <span class="text-muted text-sm" style="margin-left:0.5rem">{{ r.upvote_count }}↑ {{ r.downvote_count }}↓</span>
            {% if r.comment_count > 0 %}
            <button class="btn btn-sm btn-outline" style="margin-left:0.25rem"
                    hx-get="/api/requests/{{ r.request.id }}/votes"
                    hx-target="#comments-{{ r.request.id }}"
                    hx-swap="innerHTML">{{ r.comment_count }} comment{% if r.comment_count != 1 %}s{% endif %}</button>
            {% endif %}
            <div class="vote-comment-input" id="vote-comment-{{ r.request.id }}" style="display:none;width:100%;margin-top:0.25rem">
                <input type="text" placeholder="Add a comment with your vote..." style="width:100%;font-size:0.85rem"
                       oninput="updateVoteComments(this, {{ r.request.id }})">
            </div>
        </div>
    </div>
    {% if let Some(resolver) = r.resolver_username.as_deref() %}
    <div class="text-sm text-muted mt-1">
        Resolved by {{ resolver }} · {{ r.request.resolved_at.as_deref().unwrap_or("") }}
        {% if let Some(rc) = r.request.resolve_comment.as_deref() %}
        — "{{ rc }}"
        {% endif %}
    </div>
    {% endif %}
    {% if r.request.status == "pending" && user.role.can_manage_mods() %}
    <div class="mt-1 resolve-form" style="display:flex;gap:0.5rem;align-items:flex-start;flex-wrap:wrap">
        <form hx-post="/api/requests/{{ r.request.id }}/resolve" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML"
              style="display:flex;gap:0.5rem;align-items:center;flex-wrap:wrap">
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
            <input type="hidden" name="action" value="approve">
            <input type="text" name="comment" placeholder="Comment (optional)" style="flex:1;min-width:150px" class="text-sm">
            <label class="text-sm" style="display:flex;align-items:center;gap:0.25rem;cursor:pointer">
                <input type="checkbox" name="install" value="true"> Install now
            </label>
            <button type="submit" class="btn btn-sm btn-success">Approve</button>
        </form>
        <form hx-post="/api/requests/{{ r.request.id }}/resolve" hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML"
              style="display:inline">
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
            <input type="hidden" name="action" value="reject">
            <button type="submit" class="btn btn-sm btn-danger">Reject</button>
        </form>
    </div>
    {% endif %}
    <div id="comments-{{ r.request.id }}"></div>
</div>
</div>
```

- [ ] **Step 4: Create the search results partial**

Create `templates/mods/partials/search_results.html`:

```html
{% if let Some(err) = error.as_deref() %}
<div class="alert alert-warning text-sm">{{ err }}</div>
{% endif %}
{% if results.is_empty() && error.is_none() %}
{% else %}
{% for m in &results %}
<div class="search-card card" style="padding:0.75rem;margin-bottom:0.5rem;cursor:pointer"
     data-mod-id="{{ m.id }}" data-mod-name="{{ m.name }}" data-fika="{{ m.fika_compatible }}">
    <div class="flex-between">
        <strong>{{ m.name }}</strong>
        <span class="text-muted text-sm">ID: {{ m.id }}</span>
    </div>
    {% if let Some(desc) = m.description.as_deref() %}
    <p class="text-muted text-sm" style="max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{{ desc }}</p>
    {% endif %}
    {% if m.fika_compatible == "compatible" %}
    <span class="badge badge-success">Fika ✓</span>
    {% elif m.fika_compatible == "incompatible" %}
    <span class="badge badge-danger">Fika ✗</span>
    {% endif %}
</div>
{% endfor %}
<script>
document.querySelectorAll('.search-card').forEach(function(card) {
    card.addEventListener('click', function() {
        var id = this.dataset.modId;
        var name = this.dataset.modName;
        var fika = this.dataset.fika;
        document.querySelectorAll('.search-card').forEach(function(c) { c.style.borderColor = ''; });
        this.style.borderColor = 'var(--accent)';
        document.getElementById('selected-mod-id').value = id;
        var preview = document.getElementById('selected-mod-preview');
        var badge = '';
        if (fika === 'compatible') badge = ' <span class="badge badge-success">Fika ✓</span>';
        else if (fika === 'incompatible') badge = ' <span class="badge badge-danger">Fika ✗</span>';
        preview.innerHTML = '<strong>' + name.replace(/</g, '&lt;') + '</strong> (ID: ' + id + ')' + badge;
        document.getElementById('request-create-form').style.display = 'block';
    });
});
</script>
{% endif %}
```

- [ ] **Step 5: Create the vote comments partial**

Create `templates/mods/partials/vote_comments.html`:

```html
{% if comments.is_empty() %}
<p class="text-muted text-sm mt-1">No comments.</p>
{% else %}
<div style="margin-top:0.5rem;padding-left:1rem;border-left:2px solid var(--border)">
    {% for c in &comments %}
    <div class="text-sm" style="margin-bottom:0.5rem">
        <span style="font-weight:600">{{ c.username }}</span>
        {% if c.upvote %}
        <span class="badge badge-success" style="font-size:0.7rem">▲</span>
        {% else %}
        <span class="badge badge-danger" style="font-size:0.7rem">▼</span>
        {% endif %}
        <span class="text-muted" style="margin-left:0.25rem">{{ c.created_at }}</span>
        <p style="margin:0.15rem 0 0 0">{{ c.comment }}</p>
    </div>
    {% endfor %}
</div>
{% endif %}
```

- [ ] **Step 6: Run build to verify templates compile**

Run: `cargo build 2>&1 | head -50`

Expected: Successful build (or only warnings, no errors). Askama validates templates at compile time.

- [ ] **Step 7: Run full test suite and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All tests pass, no clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add templates/mods/list.html templates/mods/partials/requests.html templates/mods/partials/request_card.html templates/mods/partials/search_results.html templates/mods/partials/vote_comments.html
git commit -m "feat: mod request & voting UI templates with HTMX tabs"
```

---

### Task 5: CSS Styles for Request Cards & Voting

**Files:**
- Modify: `src/assets/style.css` — add request card, vote button, search card, status filter styles

**Interfaces:**
- Consumes: CSS class names referenced in Task 4 templates
- Produces: Visual styling for the mod request feature

- [ ] **Step 1: Add CSS for request cards and voting**

Append to `src/assets/style.css`:

```css

/* Mod requests */
.request-card { position: relative; }
.request-card .vote-buttons .btn-sm { min-width: 2rem; justify-content: center; }
.vote-score { font-size: 1.1rem; }

.search-card { transition: border-color 0.15s; }
.search-card:hover { border-color: var(--accent); }
.search-card.selected { border-color: var(--accent); }

.status-filter { display: flex; gap: 0.25rem; }
.status-filter .btn.active { border-color: var(--accent); color: var(--text); }

.resolve-form input[type="text"] {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    padding: 0.25rem 0.5rem;
    font-size: 0.85rem;
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build && cargo clippy -- -D warnings`

Expected: Clean build.

- [ ] **Step 3: Commit**

```bash
git add src/assets/style.css
git commit -m "feat: CSS styles for mod request cards and voting UI"
```

---

### Task 6: Handler Unit Tests & Integration Verification

**Files:**
- Modify: `src/web/handlers/requests.rs` — add `#[cfg(test)] mod tests` with unit tests for `parse_forge_url`, `is_cache_stale`, `fika_compat_to_string`

**Interfaces:**
- Consumes: Helper functions from Task 3
- Produces: Test coverage for handler logic

- [ ] **Step 1: Add unit tests to the handlers file**

Add at the end of `src/web/handlers/requests.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_forge_url_numeric_id() {
        assert_eq!(parse_forge_url("2326"), Some(2326));
    }

    #[test]
    fn parse_forge_url_full_url() {
        assert_eq!(
            parse_forge_url("https://forge.sp-tarkov.com/mods/2326-some-mod"),
            Some(2326)
        );
    }

    #[test]
    fn parse_forge_url_url_with_trailing_slash() {
        assert_eq!(
            parse_forge_url("https://forge.sp-tarkov.com/mods/123-test/"),
            Some(123)
        );
    }

    #[test]
    fn parse_forge_url_plain_text() {
        assert_eq!(parse_forge_url("SAIN"), None);
    }

    #[test]
    fn parse_forge_url_empty() {
        assert_eq!(parse_forge_url(""), None);
    }

    #[test]
    fn parse_forge_url_whitespace() {
        assert_eq!(parse_forge_url("  2326  "), Some(2326));
    }

    #[test]
    fn parse_forge_url_with_query_params() {
        assert_eq!(
            parse_forge_url("https://forge.sp-tarkov.com/mods/2326-some-mod?details=true"),
            Some(2326)
        );
    }

    #[test]
    fn fika_compat_string_values() {
        use crate::forge::models::FikaCompat;
        assert_eq!(fika_compat_to_string(&Some(FikaCompat::Compatible)), "compatible");
        assert_eq!(fika_compat_to_string(&Some(FikaCompat::Incompatible)), "incompatible");
        assert_eq!(fika_compat_to_string(&Some(FikaCompat::Unknown)), "unknown");
        assert_eq!(fika_compat_to_string(&None), "unknown");
    }

    #[test]
    fn cache_stale_old_datetime() {
        assert!(is_cache_stale("2020-01-01 00:00:00", 86400));
    }

    #[test]
    fn cache_stale_recent_datetime() {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        assert!(!is_cache_stale(&now, 86400));
    }

    #[test]
    fn cache_stale_rfc3339_format() {
        assert!(is_cache_stale("2020-01-01T00:00:00+00:00", 86400));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p quartermaster -- requests`

Expected: All tests pass.

- [ ] **Step 3: Run full suite and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All tests pass, no clippy warnings.

- [ ] **Step 4: Commit**

```bash
git add src/web/handlers/requests.rs
git commit -m "test: unit tests for mod request handler helpers"
```

---

### Task 7: Migration Table Verification & Final Cleanup

**Files:**
- Modify: `src/db/tests.rs` — add migration verification test for new tables
- Modify: `TODO.md` — remove the mod request future feature entry

**Interfaces:**
- Consumes: All previous tasks
- Produces: Final verification that everything compiles, tests pass, and the feature is tracked correctly

- [ ] **Step 1: Add migration verification test**

In `src/db/tests.rs`, update the existing `create_in_memory_db` test to include the new tables:

Add these assertions to the `create_in_memory_db` test:

```rust
    assert!(tables.contains(&"mod_requests".to_string()));
    assert!(tables.contains(&"mod_request_votes".to_string()));
```

- [ ] **Step 2: Update TODO.md**

Remove the line from the Future Features section:

```
- **Player mod request/voting**: Players can suggest mods via the web UI, admin approves/rejects
```

- [ ] **Step 3: Run full test suite and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All tests pass, no warnings.

- [ ] **Step 4: Commit**

```bash
git add src/db/tests.rs TODO.md
git commit -m "chore: migration verification test and remove mod requests from TODO"
```
