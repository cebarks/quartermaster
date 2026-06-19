# Permission Model & Session Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the binary admin/player permission model with a three-role system (Admin, Moderator, Player), add account disabling, and switch to DB-validated sessions for immediate enforcement of role changes and disables.

**Architecture:** A `Role` enum replaces bare role strings throughout the codebase. The `RequireAuth` middleware gains a per-request DB lookup that validates the user's role and disabled status, injecting a verified `SessionUser` into request extensions. Handlers switch from `require_admin()` to capability-based checks (`can_manage_mods()`, `can_control_server()`, etc.). Templates conditionally render controls based on role capabilities.

**Tech Stack:** Rust, actix-web 4, actix-session 0.11 (CookieSessionStore), actix-web-lab 0.24, rusqlite (SQLite WAL), Askama 0.16, serde, parking_lot

## Global Constraints

- Binary name: `quma`
- Build: `just build` / `just check` / `just test` / `just clippy` / `just lint`
- Single test: `cargo test <test_name>`
- DB is SQLite via rusqlite, wrapped in `Arc<parking_lot::Mutex<Database>>`
- Migrations use `PRAGMA user_version` with `include_str!` consts in `src/db/schema.rs`
- Templates are Askama (compile-time checked) — template errors are compile errors
- All existing tests must continue passing after each task
- The `disabled` column defaults to `0` (active) — existing users are unaffected
- Role serialization must use lowercase strings (`"admin"`, `"moderator"`, `"player"`) for DB and session cookie compatibility

---

### Task 1: Role Enum and DB Layer

**Files:**
- Modify: `src/db/users.rs` — add `Role` enum, update `User` struct, update all DB functions
- Modify: `src/db/schema.rs` — add migration 005
- Create: `migrations/005_add_disabled_column.sql`
- Modify: `src/db/tests.rs` — update existing tests, add new tests
- Modify: `Cargo.toml` — no changes needed (serde already a dependency with `derive` feature)

**Interfaces:**
- Produces: `Role` enum with `can_manage_mods()`, `can_control_server()`, `can_manage_queue()`, `can_manage_users()`, `as_str() -> &'static str`, `Display`, `TryFrom<String>`, `Serialize`/`Deserialize`
- Produces: `User` struct with `role: Role` and `disabled: bool`
- Produces: `insert_user()` accepting `Role` instead of `&str`
- Produces: `get_user_by_id(id: i64) -> rusqlite::Result<Option<User>>` (new, needed by middleware)
- Produces: `row_to_user()` deserializing role TEXT to `Role` and disabled INTEGER to `bool`

- [ ] **Step 1: Create the migration file**

Create `migrations/005_add_disabled_column.sql`:

```sql
ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Register the migration in schema.rs**

In `src/db/schema.rs`, add after the `MIGRATION_004` const (line 4):

```rust
const MIGRATION_005: &str = include_str!("../../migrations/005_add_disabled_column.sql");
```

And add a new block inside `run_migrations()` after the `current_version < 4` block:

```rust
        if current_version < 5 {
            conn.execute_batch(MIGRATION_005)?;
            conn.pragma_update(None, "user_version", 5)?;
        }
```

- [ ] **Step 3: Write the Role enum with tests**

In `src/db/users.rs`, add before the `User` struct (before line 6):

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

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
```

In `src/db/tests.rs`, add Role tests:

```rust
use crate::db::users::Role;

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
```

- [ ] **Step 4: Update User struct and DB functions**

Update the `User` struct in `src/db/users.rs`:

```rust
pub struct User {
    pub id: i64,
    pub username: String,
    pub spt_profile_id: String,
    pub password_hash: Option<String>,
    pub role: Role,
    pub disabled: bool,
    pub created_at: String,
}
```

Update `row_to_user()`:

```rust
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
        created_at: row.get(6)?,
    })
}
```

Update `insert_user()` signature:

```rust
pub fn insert_user(
    &self,
    username: &str,
    spt_profile_id: &str,
    password_hash: Option<&str>,
    role: Role,
) -> rusqlite::Result<i64> {
    self.conn.execute(
        "INSERT INTO users (username, spt_profile_id, password_hash, role) VALUES (?1, ?2, ?3, ?4)",
        params![username, spt_profile_id, password_hash, role.as_str()],
    )?;
    Ok(self.conn.last_insert_rowid())
}
```

Update `get_user_by_username()` and `list_users()` SQL to include `disabled`:

```rust
pub fn get_user_by_username(&self, username: &str) -> rusqlite::Result<Option<User>> {
    self.conn
        .query_row(
            "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at
             FROM users WHERE username = ?1",
            params![username],
            row_to_user,
        )
        .optional()
}

pub fn list_users(&self) -> rusqlite::Result<Vec<User>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at
         FROM users ORDER BY username",
    )?;
    let users = stmt.query_map([], row_to_user)?;
    users.collect()
}
```

Update `admin_exists()`:

```rust
pub fn admin_exists(&self) -> rusqlite::Result<bool> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM users WHERE role = ?1",
        params![Role::Admin.as_str()],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
```

Add `get_user_by_id()`:

```rust
pub fn get_user_by_id(&self, id: i64) -> rusqlite::Result<Option<User>> {
    self.conn
        .query_row(
            "SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at
             FROM users WHERE id = ?1",
            params![id],
            row_to_user,
        )
        .optional()
}
```

- [ ] **Step 5: Update existing tests in tests.rs**

Every test that calls `insert_user` with a string role must switch to `Role`:

```rust
// Change all instances of:
db.insert_user("alice", "profile-abc", Some("hashed_pw"), "admin")
// To:
db.insert_user("alice", "profile-abc", Some("hashed_pw"), Role::Admin)

// Change all instances of:
db.insert_user("player1", "p1", Some("pw"), "player")
// To:
db.insert_user("player1", "p1", Some("pw"), Role::Player)
```

Update role assertions from string comparisons to enum:

```rust
// Change:
assert_eq!(user.role, "admin");
// To:
assert_eq!(user.role, Role::Admin);
```

Add a test for `get_user_by_id`:

```rust
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
```

Add a test for the disabled column:

```rust
#[test]
fn user_disabled_default() {
    let db = test_db();
    let id = db
        .insert_user("alice", "profile-abc", Some("hashed_pw"), Role::Player)
        .unwrap();
    let user = db.get_user_by_id(id).unwrap().expect("user should exist");
    assert!(!user.disabled);
}
```

- [ ] **Step 6: Run tests and verify**

Run: `cargo test -p quartermaster`
Expected: All tests pass, including the new `role_capabilities`, `role_serialization_roundtrip`, `role_display`, `role_serde_lowercase`, `get_user_by_id`, and `user_disabled_default` tests.

- [ ] **Step 7: Run clippy**

Run: `just clippy`
Expected: No warnings.

- [ ] **Step 8: Commit**

```bash
git add src/db/users.rs src/db/schema.rs src/db/tests.rs migrations/005_add_disabled_column.sql
git commit -m "feat: add Role enum, disabled column, and get_user_by_id"
```

---

### Task 2: Update CLI Callers

**Files:**
- Modify: `src/cli/setup.rs:498` — change `"admin"` to `Role::Admin`
- Modify: `src/web/handlers/auth.rs:339` — change `"player"` to `Role::Player`

**Interfaces:**
- Consumes: `Role::Admin`, `Role::Player` from `src/db/users.rs`
- Produces: no new interfaces — just fixes compilation after Task 1

- [ ] **Step 1: Update setup.rs**

In `src/cli/setup.rs`, add the import at the top of the file:

```rust
use crate::db::users::Role;
```

At line 498, change the `insert_user` call:

```rust
// Change:
db.insert_user(
    &profile.username,
    &profile.aid,
    Some(&password_hash),
    "admin",
)
// To:
db.insert_user(
    &profile.username,
    &profile.aid,
    Some(&password_hash),
    Role::Admin,
)
```

- [ ] **Step 2: Update auth.rs register_submit**

In `src/web/handlers/auth.rs`, add the import at the top:

```rust
use crate::db::users::Role;
```

At line 339, change:

```rust
// Change:
let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), "player")?;
// To:
let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), Role::Player)?;
```

- [ ] **Step 3: Update login_submit to check disabled**

In the `login_submit` handler in `src/web/handlers/auth.rs`, after the password verification succeeds (around line 145, after `if !password_valid`), add a disabled check:

```rust
if user.disabled {
    flash.set("Your account has been disabled.");
    return Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish());
}
```

- [ ] **Step 4: Update SessionUser construction in login_submit**

In `login_submit` around line 155, the `SessionUser` is constructed from `user.role`. Since `user.role` is now `Role` (not String), and `set_session_user` still stores individual session keys, update the session storage to use `role.as_str()`. This will be handled more completely in Task 3 when we update `auth.rs`, but for now ensure `login_submit` compiles:

The `SessionUser` construction at line 155 uses `role: user.role` — since `User.role` is now `Role` and `SessionUser.role` is still `String` at this point, temporarily use `role: user.role.as_str().to_string()`. This will be cleaned up in Task 3.

- [ ] **Step 5: Build and test**

Run: `just check` then `cargo test -p quartermaster`
Expected: Compiles and all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/cli/setup.rs src/web/handlers/auth.rs
git commit -m "feat: use Role enum in setup and registration, check disabled on login"
```

---

### Task 3: Session Validation and Auth Overhaul

**Files:**
- Modify: `src/web/auth.rs` — update `SessionUser` to use `Role`, rewrite `RequireAuth` middleware with DB validation, add `require_capability()`, update `require_admin()`
- Modify: `src/web/mod.rs` — no route changes needed, but middleware wiring may need adjustment

**Interfaces:**
- Consumes: `Role` enum from `src/db/users.rs`, `get_user_by_id()` from `src/db/users.rs`, `AppState` from `src/web/state.rs`
- Produces: `SessionUser` with `role: Role`
- Produces: `require_capability(user: &SessionUser, check: fn(&Role) -> bool) -> Result<(), WebError>`
- Produces: `require_admin(user: &SessionUser) -> Result<(), WebError>` (updated signature — no longer takes `&Session`)
- Produces: Verified `SessionUser` injected into request extensions by middleware

- [ ] **Step 1: Update SessionUser**

In `src/web/auth.rs`, update `SessionUser`:

```rust
use crate::db::users::Role;

pub struct SessionUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
}
```

Remove `is_admin()` from `SessionUser` — callers will use `role.can_manage_users()` instead.

- [ ] **Step 2: Update set_session_user and get_session_user**

Update `set_session_user` to store role as its lowercase string form:

```rust
pub fn set_session_user(session: &Session, user: &SessionUser) -> Result<()> {
    session.insert("user_id", user.user_id)?;
    session.insert("username", &user.username)?;
    session.insert("role", user.role.as_str())?;
    Ok(())
}
```

Update `get_session_user` — this is now only used as a fallback. The primary path is the middleware reading `user_id` directly:

```rust
pub fn get_session_user(session: &Session) -> Option<SessionUser> {
    let user_id = session.get::<i64>("user_id").ok()??;
    let username = session.get::<String>("username").ok()??;
    let role_str = session.get::<String>("role").ok()??;
    let role = Role::try_from(role_str).ok()?;
    Some(SessionUser {
        user_id,
        username,
        role,
    })
}
```

- [ ] **Step 3: Add require_capability and update require_admin**

Replace the existing `require_auth` and `require_admin` functions:

```rust
pub fn require_auth(req: &actix_web::HttpRequest) -> std::result::Result<SessionUser, WebError> {
    req.extensions()
        .get::<SessionUser>()
        .cloned()
        .ok_or(WebError::Forbidden)
}

pub fn require_capability(
    user: &SessionUser,
    check: fn(&Role) -> bool,
) -> std::result::Result<(), WebError> {
    if !check(&user.role) {
        return Err(WebError::Forbidden);
    }
    Ok(())
}

pub fn require_admin(user: &SessionUser) -> std::result::Result<(), WebError> {
    require_capability(user, Role::can_manage_users)
}
```

Note: `require_auth` now takes `&HttpRequest` instead of `&Session`, reading from request extensions. `require_admin` now takes `&SessionUser` instead of `&Session` and returns `Result<(), WebError>` instead of `Result<SessionUser, WebError>`.

Add `Clone` derive to `SessionUser`:

```rust
#[derive(Clone)]
pub struct SessionUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
}
```

- [ ] **Step 4: Rewrite RequireAuth middleware with DB validation**

Replace the entire `RequireAuth` middleware implementation. Use `actix_web_lab::middleware::from_fn` for simplicity since it's already a dependency.

Create a standalone async function for the middleware logic, then wrap it with `from_fn`:

```rust
use actix_session::SessionExt;
use actix_web::{body::BoxBody, dev::ServiceRequest, dev::ServiceResponse, web, HttpResponse};
use actix_web_lab::middleware::{from_fn, Next};

async fn auth_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let session = req.get_session();
    let user_id: Option<i64> = session.get("user_id").unwrap_or(None);

    let Some(user_id) = user_id else {
        let response = HttpResponse::SeeOther()
            .insert_header(("Location", "/login"))
            .finish();
        return Ok(req.into_response(response).map_into_boxed_body());
    };

    let state = req
        .app_data::<web::Data<crate::web::state::AppState>>()
        .expect("AppState not found")
        .clone();

    let verified_user = web::block(move || {
        let db = state.db.lock();
        db.get_user_by_id(user_id)
    })
    .await;

    let verified_user = match verified_user {
        Ok(Ok(Some(user))) if !user.disabled => user,
        _ => {
            session.purge();
            let response = HttpResponse::SeeOther()
                .insert_header(("Location", "/login"))
                .finish();
            return Ok(req.into_response(response).map_into_boxed_body());
        }
    };

    let session_user = SessionUser {
        user_id: verified_user.id,
        username: verified_user.username.clone(),
        role: verified_user.role,
    };

    let _ = set_session_user(&session, &session_user);
    req.extensions_mut().insert(session_user);
    next.call(req).await
}
```

Remove the old `RequireAuth` struct, `Transform` impl, and `RequireAuthMiddleware` struct+impl.

- [ ] **Step 5: Update middleware wiring in mod.rs**

In `src/web/mod.rs`, the `RequireAuth` middleware is used in two places for the `/api` scope and the root page scope. Replace with `from_fn(auth_middleware)`:

The old pattern:
```rust
web::scope("/api").wrap(RequireAuth)
```

The new pattern:
```rust
use actix_web_lab::middleware::from_fn;
use crate::web::auth::auth_middleware;

web::scope("/api").wrap(from_fn(auth_middleware))
// ...
web::scope("").wrap(from_fn(auth_middleware))
```

Remove the `use crate::web::auth::RequireAuth;` import.

- [ ] **Step 6: Build and fix any remaining compilation errors**

Run: `just check`

At this point, all handlers that call `require_admin(&session)?` or `require_auth(&session)?` will have compilation errors. These are expected and will be fixed in Task 4. First verify the auth module itself compiles by checking for errors only in auth.rs.

- [ ] **Step 7: Commit the auth overhaul (may not compile yet — handler updates in Task 4)**

```bash
git add src/web/auth.rs src/web/mod.rs
git commit -m "feat: DB-validated sessions with Role-based auth middleware"
```

---

### Task 4: Update All Handlers

**Files:**
- Modify: `src/web/handlers/mods.rs` — 9 handlers: switch from `require_admin(&session)?` to `require_auth(&req)` + `require_capability()`
- Modify: `src/web/handlers/server.rs` — 3 handlers
- Modify: `src/web/handlers/queue.rs` — 2 handlers (cancel_op, apply_queue) + queue_page
- Modify: `src/web/handlers/dashboard.rs` — dashboard handler
- Modify: `src/web/handlers/status.rs` — status_page handler
- Modify: `src/web/handlers/logs.rs` — logs_page handler
- Modify: `src/web/handlers/tasks.rs` — task handlers

**Interfaces:**
- Consumes: `require_auth(req: &HttpRequest) -> Result<SessionUser, WebError>`, `require_capability(user: &SessionUser, check: fn(&Role) -> bool) -> Result<(), WebError>` from `src/web/auth.rs`
- Consumes: `Role::can_manage_mods`, `Role::can_control_server`, `Role::can_manage_queue` from `src/db/users.rs`
- Produces: All handlers compile and use the new auth pattern

The migration pattern for each handler type:

**Old pattern (admin-only handlers):**
```rust
pub async fn some_handler(state: Data<AppState>, session: Session) -> Result<...> {
    let user = require_admin(&session)?;
    // ...
}
```

**New pattern (capability-checked handlers):**
```rust
pub async fn some_handler(state: Data<AppState>, req: HttpRequest) -> Result<...> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    // ...
}
```

**Old pattern (auth-only handlers):**
```rust
pub async fn some_handler(state: Data<AppState>, session: Session) -> Result<...> {
    let user = require_auth(&session)?;
    // ...
}
```

**New pattern (auth-only handlers):**
```rust
pub async fn some_handler(state: Data<AppState>, req: HttpRequest) -> Result<...> {
    let user = require_auth(&req)?;
    // ...
}
```

Note: Handlers that still need `Session` for CSRF or flash messages keep the `session: Session` parameter in addition to `req: HttpRequest`. The `require_auth` call switches from `&session` to `&req`.

- [ ] **Step 1: Update mods.rs handlers**

In `src/web/handlers/mods.rs`, add imports:

```rust
use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability};
use actix_web::HttpRequest;
```

Update each handler. There are 9 handlers calling `require_admin` and 1 calling `require_auth`:

1. `list_mods` (line 108) — `let user = require_admin(&session)?;` → `let user = require_auth(&req)?; require_capability(&user, Role::can_manage_mods)?;`
2. `mod_detail` (line 150) — `let user = require_auth(&session)?;` → `let user = require_auth(&req)?;`
3. `check_updates_partial` (line 192) — `require_admin(&session)?;` → `let user = require_auth(&req)?; require_capability(&user, Role::can_manage_mods)?;`
4. `update_status_partial` (line 266) — same pattern as above
5. `dep_tree_partial` (line 357) — same pattern
6. `install_mod` (line 376) — `require_admin(&session)?;` → `let user = require_auth(&req)?; require_capability(&user, Role::can_manage_mods)?;`
7. `update_mod` (line 580) — same pattern
8. `remove_mod` (line 742) — same pattern
9. `update_all_mods` (line 806) — same pattern
10. `list_body_partial` (line 988) — `let user = require_admin(&session)?;` → `let user = require_auth(&req)?; require_capability(&user, Role::can_manage_mods)?;`

Add `req: HttpRequest` parameter to each handler signature that doesn't already have it.

- [ ] **Step 2: Update server.rs handlers**

In `src/web/handlers/server.rs`, add imports and update 3 handlers:

1. `start_server` (line 16) — `require_admin(&session)?;` → `let user = require_auth(&req)?; require_capability(&user, Role::can_control_server)?;`
2. `stop_server` (line 53) — same
3. `restart_server` (line 90) — same

- [ ] **Step 3: Update queue.rs handlers**

In `src/web/handlers/queue.rs`, update 3 handlers:

1. `queue_page` (line 22) — `let user = require_auth(&session)?;` → `let user = require_auth(&req)?;`
2. `cancel_op` (line 50) — `require_admin(&session)?;` → `let user = require_auth(&req)?; require_capability(&user, Role::can_manage_queue)?;`
3. `apply_queue` (line 76) — same as cancel_op

- [ ] **Step 4: Update remaining handlers**

Update `dashboard.rs`, `status.rs`, `logs.rs`, `tasks.rs` — all use `require_auth(&session)?` which becomes `require_auth(&req)?`:

- `dashboard::dashboard` — `require_auth(&session)?` → `require_auth(&req)?`
- `dashboard::server_status_partial` — add `require_auth(&req)?` (currently has no auth check)
- `status::status_page` — `require_auth(&session)?` → `require_auth(&req)?`
- `status::server_partial` — add `require_auth(&req)?` (currently has no auth check)
- `status::mods_partial` — `require_auth(&session)?` → `require_auth(&req)?`
- `status::integrity_partial` — `require_auth(&session)?` → `require_auth(&req)?`
- `logs::logs_page` — `require_auth(&session)?` → `require_auth(&req)?`
- `logs::app_logs_json`, `app_logs_stream`, `server_logs_json`, `server_logs_stream` — `require_auth(&session)?` → `require_auth(&req)?`
- `tasks::task_status_partial`, `dismiss_task` — `require_auth(&session)?` → `require_auth(&req)?`

- [ ] **Step 5: Build and test**

Run: `just check` then `cargo test -p quartermaster`
Expected: Compiles with no errors and all tests pass.

- [ ] **Step 6: Run clippy**

Run: `just clippy`
Expected: No warnings.

- [ ] **Step 7: Commit**

```bash
git add src/web/handlers/
git commit -m "feat: switch all handlers to capability-based auth"
```

---

### Task 5: Template Conditional Rendering

**Files:**
- Modify: `templates/partials/nav.html` — conditional Mods link, role display
- Modify: `templates/dashboard.html` — `is_admin()` → `can_manage_mods()`
- Modify: `templates/status.html` — `is_admin()` → `can_control_server()`
- Modify: `templates/queue.html` — `is_admin()` → `can_manage_queue()`
- Modify: `templates/mods/list.html` — `is_admin()` → `can_manage_mods()`
- Modify: `templates/mods/partials/list_body.html` — `is_admin()` → `can_manage_mods()`
- Modify: `templates/mods/detail.html` — `is_admin()` → `can_manage_mods()`

**Interfaces:**
- Consumes: `SessionUser` with `role: Role` passed as template field `user`
- Consumes: `Role::can_manage_mods()`, `can_control_server()`, `can_manage_queue()`, `can_manage_users()` called directly in Askama templates

- [ ] **Step 1: Update nav.html**

Replace all `is_admin()` references. The nav currently shows all links unconditionally. Add conditional rendering for the Mods link and update the role display:

Change the Mods link from unconditional to:
```html
{% if user.role.can_manage_mods() %}
<a href="/mods" ...>Mods</a>
{% endif %}
```

The `{{ user.role }}` display (line 11 in nav.html) will now call `Display` on the `Role` enum, showing "Admin", "Moderator", or "Player" instead of the raw lowercase string.

- [ ] **Step 2: Update dashboard.html**

At line 47, change:
```html
{% if user.is_admin() %}
```
to:
```html
{% if user.role.can_manage_mods() %}
```

- [ ] **Step 3: Update status.html**

At lines 9 and 26, change:
```html
{% if user.is_admin() %}
```
to:
```html
{% if user.role.can_control_server() %}
```

- [ ] **Step 4: Update queue.html**

At all `is_admin()` checks (lines 9, 34, 50), change to:
```html
{% if user.role.can_manage_queue() %}
```

- [ ] **Step 5: Update mods/list.html**

At all `is_admin()` checks (lines 10, 35, 48, 62, 81, 91), change to:
```html
{% if user.role.can_manage_mods() %}
```

And for the colspan calculation:
```html
<td colspan="{% if user.role.can_manage_mods() %}6{% else %}5{% endif %}">
```

- [ ] **Step 6: Update mods/partials/list_body.html**

At `is_admin()` checks (lines 9, 28), change to:
```html
{% if user.role.can_manage_mods() %}
```

- [ ] **Step 7: Update mods/detail.html**

At `is_admin()` check (line 26), change to:
```html
{% if user.role.can_manage_mods() %}
```

- [ ] **Step 8: Build and verify**

Run: `just check`
Expected: Compiles. Askama templates are compile-time checked, so a successful build means the Role method calls are valid.

Run: `cargo test -p quartermaster`
Expected: All tests pass.

- [ ] **Step 9: Commit**

```bash
git add templates/
git commit -m "feat: role-conditional template rendering for three-role system"
```

---

### Task 6: Final Verification

**Files:** None — verification only.

- [ ] **Step 1: Full lint check**

Run: `just lint` (runs `just fmt` + `just clippy`)
Expected: No warnings, no formatting changes.

- [ ] **Step 2: Full test suite**

Run: `just test`
Expected: All tests pass.

- [ ] **Step 3: Manual smoke test**

Start the server with `just serve`. Verify:

1. Login as admin works — can see Mods, Queue, Status, Logs links in nav
2. Admin can start/stop/restart server (if container available)
3. Admin sees all mod management controls (install, update, remove)
4. Register a new user via invite code — they get `Player` role
5. Login as the player — Mods link is hidden in nav, no server controls on status page, no queue actions
6. Role displays as "Admin" or "Player" in the nav bar (not "admin"/"player")

- [ ] **Step 4: Commit any fixes from smoke testing**

If any issues found during smoke testing, fix and commit.
