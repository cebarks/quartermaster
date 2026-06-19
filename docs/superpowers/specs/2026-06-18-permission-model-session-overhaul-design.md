# Permission Model & Session Overhaul

**Sub-project 1 of 3** in the Multi-User Support initiative.

Replaces the binary admin/player permission model with a three-role system (Admin, Moderator, Player), adds account disabling, and switches to DB-validated sessions for immediate enforcement of role changes and disables.

## Roles

### Role Enum

Replace the bare `role: String` with a Rust enum in `src/db/users.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Moderator,
    Player,
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
```

`Display` is required because Askama templates render `{{ user.role }}` (e.g., in nav.html).

`#[serde(rename_all = "lowercase")]` ensures the enum serializes as `"admin"`, `"moderator"`, `"player"` — matching both the existing DB TEXT values and existing session cookie values. Without this, serde defaults to PascalCase (`"Admin"`) which would break existing sessions and require a data migration.

Implement `Role::as_str() -> &'static str` (returns `"admin"`, `"moderator"`, `"player"`) and `TryFrom<&str> for Role` for DB serialization. Unknown/corrupt values must return an error — do not silently default to `Player`, as that would mask data corruption. The middleware (which runs on every request) should log a warning and redirect to `/login` if deserialization fails.

No schema migration needed for the role column — existing `"admin"` and `"player"` values are already valid.

### Capabilities

Methods on the `Role` enum define what each role can do:

| Capability | Admin | Moderator | Player |
|------------|-------|-----------|--------|
| `can_manage_mods()` | yes | yes | no |
| `can_control_server()` | yes | yes | no |
| `can_manage_queue()` | yes | yes | no |
| `can_manage_users()` | yes | no | no |

### Struct Updates

Both `User` (in `src/db/users.rs`) and `SessionUser` (in `src/web/auth.rs`) change `role: String` to `role: Role`.

### DB Function Updates

- `insert_user()` changes signature from `role: &str` to `role: Role`. Callers pass `Role::Admin` or `Role::Player` instead of string literals.
- `row_to_user()` deserializes the TEXT column to `Role` via `TryFrom<&str>`, returning an error for unknown values.
- `admin_exists()` uses `Role::Admin.as_str()` instead of the hardcoded string `"admin"` in the SQL WHERE clause.

## Account Disabling

### Database Change

New migration:

```sql
ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0;
```

`0` = active, `1` = disabled. Disabled users cannot log in and active sessions are invalidated on their next request.

### Login Rejection

The login handler in `src/web/handlers/auth.rs` must check `disabled` after password verification and reject with "Your account has been disabled."

## Session Validation

### Current Behavior

`RequireAuth` middleware checks only that a `SessionUser` exists in the signed cookie. Role is cached at login time and never re-checked. A disabled or role-changed user remains unaffected until their 7-day session expires.

### New Behavior

On every authenticated request, `RequireAuth` performs a DB lookup:

1. Read `user_id` directly from the session via `session.get::<i64>("user_id")`. Do NOT use `get_session_user()` — it bundles user_id + username + role, and if role deserialization fails (e.g., on old sessions), the user_id is never extracted and the DB lookup never fires.
2. If `user_id` is missing → redirect to `/login`.
3. Query DB: `SELECT username, role, disabled FROM users WHERE id = ?`
4. If DB query fails (lock timeout, pool exhaustion, any error) → reject the request (redirect to `/login` or return 503). **Never fall through with cached session data** — that would let disabled/downgraded users retain old privileges during DB outages.
5. If user not found or `disabled = 1` → purge session, redirect to `/login`.
6. Construct a verified `SessionUser` from the DB result and insert it into request extensions via `req.extensions_mut().insert(verified_user)`. Update the session cookie with the current role.
7. Handlers read the verified `SessionUser` from request extensions (not from the session directly).

### Implementation Notes

The current `RequireAuth` middleware is purely synchronous (cookie-only). Adding a DB call requires async bridging:

- Extract `web::Data<AppState>` from the request via `req.app_data::<web::Data<AppState>>()`. `AppState` is already registered as app_data on the App.
- Use `web::block()` to offload the synchronous rusqlite call to a thread pool.
- Consider using `actix-web-lab`'s `from_fn` middleware (already a project dependency) as a simpler alternative to the manual `Transform`/`Service` impl.
- If `app_data` extraction fails, reject the request.

This is a single indexed primary-key lookup per request. Given the expected user count (<20 concurrent for a single SPT server), no caching layer is needed. Can be added later if it ever matters.

## Handler Authorization

### New Helper

In `src/web/auth.rs`:

```rust
pub fn require_capability(user: &SessionUser, check: fn(&Role) -> bool) -> Result<(), WebError> {
    if !check(&user.role) {
        return Err(WebError::Forbidden);
    }
    Ok(())
}
```

### Route-to-Capability Mapping

| Routes | Current Check | New Check |
|--------|---------------|-----------|
| `POST /mods/install`, `/mods/{id}/update`, `/mods/{id}/remove`, `/mods/update-all` | `require_admin()` | `can_manage_mods()` |
| `GET /mods`, `/api/mods/*` | `require_admin()` | `can_manage_mods()` |
| `POST /server/start`, `/stop`, `/restart` | `require_admin()` | `can_control_server()` |
| `POST /queue/{id}/cancel`, `/queue/apply` | `require_admin()` | `can_manage_queue()` |
| Future: `/users/*` | n/a | `can_manage_users()` |
| Dashboard, status, logs, mod detail | `require_auth()` | `require_auth()` (unchanged) |

`require_admin()` remains as a convenience alias for `can_manage_users()`.

## Template Conditional Rendering

Currently, admin-only pages are blocked entirely at the handler level. With three roles, templates need conditional rendering so moderators see the right controls.

### Changes

- **Nav bar** (`templates/partials/nav.html`): Show "Mods" link to `can_manage_mods()` roles. Show future "Users" link to `can_manage_users()` only.
- **Status page** (`templates/status.html`): Show server control buttons to `can_control_server()` roles.
- **Queue page**: Show "Apply" and "Cancel" buttons to `can_manage_queue()` roles.
- **Mod detail page**: Show update/remove buttons to `can_manage_mods()` roles.

Templates receive the `SessionUser` (with `role: Role`) as a field. Askama conditionals:

```html
{% if user.role.can_manage_mods() %}
  <!-- mod management controls -->
{% endif %}
```

Handler-level capability checks remain as a security backstop — template conditionals are purely UX.

## Files Affected

- `src/db/users.rs` — `Role` enum (with serde, Display, TryFrom), `User` struct, `insert_user()` signature change (`&str` → `Role`), `row_to_user()` deserialization, `admin_exists()` query update
- `src/db/schema.rs` — add `MIGRATION_005` const via `include_str!` and `if current_version < 5` block
- `src/web/auth.rs` — `SessionUser`, `RequireAuth` middleware (DB-validated, async bridging, request extensions), `require_capability()`, `require_admin()` update
- `src/web/handlers/auth.rs` — login disabled check, `register_submit` uses `Role::Player` instead of `"player"`
- `src/web/handlers/mods.rs` — `require_admin()` → `can_manage_mods()`
- `src/web/handlers/server.rs` — `require_admin()` → `can_control_server()`
- `src/web/handlers/queue.rs` — `require_admin()` → `can_manage_queue()`
- `src/cli/setup.rs` — `create_admin_user()` uses `Role::Admin` instead of `"admin"`
- `templates/partials/nav.html` — role-conditional nav links (also fixes existing UX bug where players see Mods link that returns Forbidden)
- `templates/dashboard.html` — `is_admin()` conditional on "Go to Mods" link → `can_manage_mods()`
- `templates/status.html` — role-conditional server controls
- `templates/queue.html` — role-conditional queue actions
- `templates/mods/*.html` — role-conditional mod management buttons
- `migrations/005_add_disabled_column.sql` — new migration

## Out of Scope

- Admin user management UI (Sub-project 2)
- Mod request/voting system (Sub-project 3)
- Password change / user profile page
- Audit logging of admin actions
- Session caching / rate limiting the DB lookup
