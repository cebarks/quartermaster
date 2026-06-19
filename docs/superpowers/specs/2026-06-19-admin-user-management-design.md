# Admin User Management UI

**Sub-project 2 of 3** in the Multi-User Support initiative.

Adds a tabbed admin panel (`/admin`) for managing users and invite codes via the web UI. Depends on Sub-project 1 (Permission Model & Session Overhaul) which provides the Role enum, capability methods, account disabling, and DB-validated sessions.

## Page Structure

### Route: `GET /admin`

A single page with two HTMX-driven tabs: **Users** (default active) and **Invites**. Requires `can_manage_users()` (Admin only). One "Admin" link appears in `nav.html`, visible only when `user.role.can_manage_users()`.

On initial page load, the Users tab content is rendered inline (no extra round-trip). Switching to the Invites tab fires `hx-get="/api/admin/invites"` and swaps into `#admin-content`.

**Tab state persistence:** Use URL hash fragments (`#users`, `#invites`) to track the active tab. On page load, JavaScript checks `location.hash` and activates the corresponding tab (firing the HTMX GET if needed). Tab clicks use `hx-push-url="#invites"` / `hx-push-url="#users"` to update the URL without a page reload. This makes tabs bookmarkable and preserves state across refreshes.

**Note on tab pattern:** The existing `templates/logs.html` uses vanilla JavaScript for tab switching (CSS `display` toggling, all content rendered inline). This admin panel uses HTMX-driven tabs instead because the Invites tab content requires a separate handler/query. This is a deliberate choice — HTMX tabs avoid rendering all tab content upfront at the cost of one network round-trip on first tab switch.

### Routing Table

**Authenticated, admin-only:** All `/admin` and `/api/admin/*` routes are wrapped in a nested actix-web scope with `from_fn` middleware that enforces `can_manage_users()` before any handler runs. This is a structural guarantee — individual handlers do not need to call `require_admin()` themselves (but the scope middleware does, preventing the "forgotten check" class of bugs).

| Method | Path | Handler | Returns |
|--------|------|---------|---------|
| `GET` | `/admin` | `admin_page` | Full page with Users tab inline |
| `GET` | `/api/admin/users` | `admin_users` | Users table partial |
| `GET` | `/api/admin/invites` | `admin_invites` | Invites table + create form partial |
| `POST` | `/api/admin/users/{id}/role` | `change_role` | Updated user row partial |
| `POST` | `/api/admin/users/{id}/disable` | `toggle_disable` | Updated user row partial |
| `POST` | `/api/admin/users/{id}/reset-password` | `create_reset_token` | Updated user row partial (with reset link) |
| `POST` | `/api/admin/invites` | `create_invite` | Updated invites table partial |

All POST handlers validate CSRF tokens via `csrf::validate_token()` before processing. All row partials include the CSRF token as a hidden input in each form, passed via `csrf::get_or_create_token(&session)` in the handler (following the existing pattern in `src/web/handlers/tasks.rs`).

**Unauthenticated (rate-limited via Governor middleware, 5/min/IP, same as login/register):**

The `/reset-password` routes MUST be wrapped with `.wrap(Governor::new(&governor_conf))` in `src/web/mod.rs`, following the same pattern as `/login` and `/register`.

| Method | Path | Handler | Returns |
|--------|------|---------|---------|
| `GET` | `/reset-password?token=` | `reset_password_page` | Reset password form |
| `POST` | `/reset-password` | `reset_password_submit` | Redirect to `/login` on success |

## Users Tab

### Table Columns

| Column | Source | Notes |
|--------|--------|-------|
| Username | `User.username` | quma account name |
| Role | `User.role` | Dropdown: Admin / Moderator / Player |
| Status | `User.disabled` | Badge: Active (green) / Disabled (red) |
| SPT Profile | Profile JSON | In-game nickname, Level, Side (BEAR/USEC) |
| SPT Stats | Profile JSON | Raid count, survival rate, kill count |
| Registered | `User.created_at` | Date or relative time |
| Actions | — | Disable/Enable toggle, Reset Password |

### SPT Profile Enrichment

The handler reads SPT profile JSON files from `{spt_dir}/SPT/user/profiles/` and matches them to users via `User.spt_profile_id` == `profile.info.id`.

**Extended profile data** parsed from profile JSON. SPT profiles have two layers: a top-level `info` block (account metadata: `id`, `username`) which the existing `list_profiles()` already reads, and `characters.pmc` which contains the in-game PMC data.

| Field | JSON Path | Type |
|-------|-----------|------|
| Username | `info.username` | String (account-level, already read) |
| Nickname | `characters.pmc.Info.Nickname` | String (in-game display name) |
| Level | `characters.pmc.Info.Level` | Integer |
| Side | `characters.pmc.Info.Side` | String ("Bear" or "Usec") |
| Experience | `characters.pmc.Info.Experience` | Integer |
| Registration | `characters.pmc.Info.RegistrationDate` | Integer (unix timestamp) |
| Raid Count | `characters.pmc.Stats.Eft.OverallCounters.Items` | Derived — sum entries where Key contains `["Sessions", "Pmc"]` |
| Survival Rate | `characters.pmc.Stats.Eft.OverallCounters.Items` | Derived — survived / total sessions |
| Kill Count | `characters.pmc.Stats.Eft.Victims` | `Vec` length |

**OverallCounters parsing:** `OverallCounters.Items` is a `Vec<{Key: Vec<String>, Value: i64}>`. Example entries:

```json
{"Key": ["Sessions", "Pmc", "Survived"], "Value": 42}
{"Key": ["Sessions", "Pmc", "Died"], "Value": 15}
{"Key": ["Sessions", "Pmc", "RunThrough"], "Value": 3}
```

- **Raid count:** Sum the `Value` of all items where `Key[0] == "Sessions"` and `Key[1] == "Pmc"` (survived + died + run-through = total raids).
- **Survival count:** The single item where `Key == ["Sessions", "Pmc", "Survived"]`.
- **Survival rate:** `survived / total_raids * 100` (display as percentage, handle division by zero as "N/A").

A new `pub struct SptProfileStats` in `src/spt/profiles.rs` holds these fields (all `Option<T>` to allow partial data). A new `pub fn load_all_profile_stats(spt_dir: &Path) -> HashMap<String, SptProfileStats>` function parses all profiles once and returns a map keyed by AID. Uses serde with `#[serde(default)]` on optional nested structures so partially corrupt profiles degrade gracefully (show available fields, `None` for missing ones).

**Profile status in templates:** The template receives `Vec<(User, ProfileStatus)>` where:

```rust
pub enum ProfileStatus {
    Found(SptProfileStats),
    NotFound,       // No profile file matches this user's spt_profile_id
    ParseError,     // File exists but PMC data is corrupt/missing
}
```

- `NotFound` renders as "No profile linked" (muted text).
- `ParseError` renders as "Profile data unavailable" (muted text with warning icon).
- `Found(stats)` renders the stats card with available fields.

**Out of scope:** Stash value (requires inventory iteration + item price data, too expensive). Noted in TODO.md as a future improvement.

### Self-Protection Rules

- Admins **cannot** change their own role (prevents accidental self-demotion with no admin left).
- Admins **cannot** disable themselves.
- The role dropdown and disable button are hidden/disabled for the current user's row.

**Server-side enforcement (not just template hiding):** The `change_role` and `toggle_disable` handlers MUST reject requests where `user_id == session_user.user_id` with a 403 response, regardless of how many other admins exist. Template hiding is UX convenience, not a security boundary — the server-side check prevents crafted POST requests from bypassing it.

### Last-Admin Guard

The last-admin check and role/disable update MUST be atomic to prevent TOCTOU races (two admins simultaneously demoting each other). Use a single SQL statement with a subquery guard:

```sql
-- For role change:
UPDATE users SET role = ?
WHERE id = ?
AND (? != 'admin' OR (SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0 AND id != ?) > 0)

-- For disable:
UPDATE users SET disabled = 1
WHERE id = ?
AND (SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0 AND id != ?) > 0
```

If the UPDATE affects 0 rows due to the guard failing, return an error message: "Cannot demote/disable the last admin." The `update_user_role` and `set_user_disabled` DB functions incorporate this guard internally and return `usize` (rows affected) — 0 means the guard blocked the operation.

### HTMX Interaction

All action forms include a hidden `<input name="csrf_token">` populated by the handler via `csrf::get_or_create_token(&session)`. All action buttons include `hx-disabled-elt="this"` to prevent double-submission during the request.

- **Role dropdown:** `<select>` with `hx-post="/api/admin/users/{id}/role"` and `hx-target="closest tr"` / `hx-swap="outerHTML"`. The selected role is sent as form data. Handler validates the role string via `Role::try_from()` — returns 400 for unknown values.
- **Disable toggle:** `<button>` with `hx-post="/api/admin/users/{id}/disable"` and `hx-target="closest tr"` / `hx-swap="outerHTML"`. No form data needed — the handler reads current state and toggles. Success message for disable: "User disabled — will be logged out on their next request."
- **Reset password:** `<button>` with `hx-post="/api/admin/users/{id}/reset-password"` and `hx-target="closest tr"` / `hx-swap="outerHTML"`. Returns the row with the reset link displayed in a read-only `<input>` with a "Copy Link" button that calls `navigator.clipboard.writeText()` and briefly changes to "Copied!" (2-second timeout). Clipboard API failure fallback: the read-only input is pre-selected so the admin can Ctrl+C manually.

### User Feedback

Admin actions return inline feedback in the swapped row partial. Success is indicated by a brief CSS highlight animation on the row. Errors (last-admin guard, invalid input, DB failures) are returned as an inline error message element above the affected row using `hx-swap="outerHTML"` on the row (the returned partial includes both an error banner and the unchanged row).

**Error responses by handler:**

| Handler | Condition | HTTP Status | Error Message |
|---------|-----------|-------------|---------------|
| `change_role` | Invalid role string | 400 | "Invalid role" |
| `change_role` | Self-modification | 403 | "Cannot change your own role" |
| `change_role` | Last admin guard | 422 | "Cannot demote the last admin" |
| `change_role` | User not found | 404 | "User not found" |
| `toggle_disable` | Self-disable | 403 | "Cannot disable your own account" |
| `toggle_disable` | Last admin guard | 422 | "Cannot disable the last admin" |
| `create_reset_token` | User not found | 404 | "User not found" |
| `create_invite` | Invalid expiry | 400 | "Invalid expiry value" |
| All | DB error | 500 | "Database error — please try again" |

## Invites Tab

### Table Columns

| Column | Source | Notes |
|--------|--------|-------|
| Code | `InviteCode.code` | The `quma-xxxx` string |
| Created By | `InviteCode.created_by` → username | Who generated it |
| Created | `InviteCode.created_at` | Relative time |
| Expires | `InviteCode.expires_at` | Relative time, or "Never" |
| Status | Derived | Badge: Available (green) / Used (blue) / Expired (red) |
| Used By | `InviteCode.used_by` → username | Username, or empty |

**Status derivation:**
- `used_by IS NOT NULL` → Used (blue)
- `expires_at IS NOT NULL AND expires_at < now` → Expired (red)
- Otherwise → Available (green)

**Display order:** Most recent first (`ORDER BY created_at DESC`). Show all codes — used, expired, and active — for full history.

### Create Invite Form

Sits above the invites table. Single field:

| Field | Type | Default | Options |
|-------|------|---------|---------|
| Expiry | Select dropdown | 7 days | 1 hour, 24 hours, 7 days, 30 days, Never |

Submit via `hx-post="/api/admin/invites"` with `hx-target="#admin-content"` to refresh the full invites table. Code is generated server-side using existing `generate_invite_code()`. The new code row is highlighted briefly (CSS flash animation) and includes a "Copy Code" button (same clipboard pattern as reset links) so the admin can copy it.

Invite codes are single-use (1:1 model). Existing schema and `use_invite()` / `create_invite()` functions are used as-is.

**Username resolution:** The `InviteCode` struct stores `created_by` and `used_by` as `Option<i64>` (user IDs). The `list_invite_codes()` DB function uses a SQL JOIN to include usernames, returning a new `InviteCodeWithUsers` struct:

```rust
pub struct InviteCodeWithUsers {
    pub invite: InviteCode,
    pub created_by_username: Option<String>,
    pub used_by_username: Option<String>,
}
```

The SQL: `SELECT ic.*, u1.username AS created_by_username, u2.username AS used_by_username FROM invite_codes ic LEFT JOIN users u1 ON ic.created_by = u1.id LEFT JOIN users u2 ON ic.used_by = u2.id ORDER BY ic.created_at DESC`.

## Password Reset

### Admin-Initiated Flow

1. Admin clicks "Reset Password" on a user's row in the Users tab.
2. Handler generates a token using a CSPRNG (`OsRng` or `getrandom`) — 32 bytes of entropy, base64url-encoded (no `=` padding). Do NOT use `rand::thread_rng()` — it is not cryptographically secure.
3. Any existing reset token for that user is deleted (one active token per user).
4. Token is stored in `password_reset_tokens` table with 24-hour expiry.
5. Handler returns the updated user row partial with the reset link in a read-only input + "Copy Link" button.
6. Admin copies the link and delivers it to the user out-of-band (Discord, in-game, etc.).

### User-Facing Reset Flow

7. User visits the reset link — `GET /reset-password?token=<token>`.
8. Handler looks up the token. If not found, expired, or for a nonexistent/disabled user, render the error page with a generic message: "This password reset link is invalid or has already been used. Please contact an administrator for a new link." Do NOT distinguish between not-found, expired, or already-used tokens (prevents information disclosure).
9. Page renders a "Set New Password" form with password + confirmation fields.
10. On submit (`POST /reset-password`): validate token again (same generic error if invalid). Server-side password validation: reject passwords < 8 or > 128 chars (400, error: "Password must be 8-128 characters"), reject mismatched confirmation (400, error: "Passwords do not match"). Use HTML5 `minlength`/`maxlength` attributes on the inputs for client-side convenience.
11. Hash the new password (Argon2), update the user's `password_hash`, delete the token, purge any active sessions for this user (set `disabled = 1` momentarily then `disabled = 0` to trigger the auth middleware's session invalidation on next request — or add a `password_changed_at` column checked by middleware). Redirect to `/login` with flash message "Password updated — please log in."

### Security

- Tokens are **single-use** — deleted after successful reset.
- **24-hour expiry**, checked server-side on both GET and POST.
- **One active token per user** — creating a new one deletes any existing token.
- Token MUST be generated with a **CSPRNG** (`OsRng` / `getrandom`), not `rand::thread_rng()`.
- Reset pages are **unauthenticated** (user is locked out, that's why they need a reset).
- **Rate limited** via Governor middleware at 5 requests/min/IP, same as login and register.
- Token lookup uses `get_reset_token()` which returns `Option<ResetToken>` — the handler then uses **constant-time comparison** via the `subtle` crate (`subtle::ConstantTimeEq`) to compare the user-provided token with the DB token. Add `subtle = "2"` to `Cargo.toml` dependencies.
- **Identical error messages** for all failure modes (not found, expired, already used, user deleted) — prevents token validity enumeration.
- No "forgot password" link on login page — this is admin-initiated only.
- **Session invalidation** after password reset: existing sessions should not remain valid. Implementation approach: add a `password_changed_at TEXT` column to the `users` table (migration 006, alongside the reset tokens table). The auth middleware checks if `password_changed_at > session_created_at` and forces re-login if so. The `set_session_user()` function stores `session_created_at` in the session cookie.

### Schema — Migration 006

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

The `password_changed_at` column is nullable (existing users have `NULL`, meaning "never changed — don't invalidate"). The auth middleware only forces re-login when `password_changed_at IS NOT NULL AND password_changed_at > session_created_at`.

## Database Layer

### New Functions (`src/db/users.rs`)

**User management:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `update_user_role` | `(user_id: i64, role: Role) -> usize` | Update role column, returns rows affected |
| `set_user_disabled` | `(user_id: i64, disabled: bool) -> usize` | Set disabled flag, returns rows affected |
| `update_user_password` | `(user_id: i64, password_hash: &str) -> usize` | Set new password hash, returns rows affected |
| `list_invite_codes` | `() -> Vec<InviteCodeWithUsers>` | All invites with JOINed usernames, ordered by `created_at DESC` |
| `count_admins` | `() -> i64` | Count of users with `role = 'admin'` and `disabled = 0` |

**Password reset tokens:**

| Function | Signature | Purpose |
|----------|-----------|---------|
| `create_reset_token` | `(user_id: i64, token: &str, expires_at: &str) -> i64` | Insert token, deleting any existing token for this user first |
| `get_reset_token` | `(token: &str) -> Option<ResetToken>` | Look up by token string |
| `delete_reset_token` | `(token: &str) -> usize` | Remove after successful use |

### New Struct

```rust
pub struct ResetToken {
    pub id: i64,
    pub user_id: i64,
    pub token: String,
    pub expires_at: String,
    pub created_at: String,
}
```

### Existing Functions Used As-Is

- `list_users()` — already returns all users ordered by username
- `get_user_by_id()` — used by role change, disable, and reset handlers
- `create_invite()` — already takes `(code, created_by, expires_at)`
- `get_invite()` — used for invite validation
- `generate_invite_code()` — in `src/cli/invite.rs`, reused for web invite creation

## Files Affected

### New Files

| File | Purpose |
|------|---------|
| `src/web/handlers/admin.rs` | Admin panel handlers (7 functions) |
| `src/invite.rs` | Shared invite utilities: `generate_invite_code()` and `parse_expiry()` (moved from CLI) |
| `templates/admin.html` | Main admin page with tab bar (extends `base.html`) |
| `templates/admin/partials/users.html` | Users table partial (HTMX swap target, no base) |
| `templates/admin/partials/user_row.html` | Single user row partial (just `<tr>`, no base) |
| `templates/admin/partials/invites.html` | Invites table + create form partial (no base) |
| `templates/reset_password.html` | Password reset page (extends `base.html`, unauthenticated) |
| `migrations/006_password_reset_tokens.sql` | Reset tokens table + `password_changed_at` column |

### Modified Files

| File | Change |
|------|--------|
| `src/main.rs` | Add `mod invite;` declaration |
| `src/web/mod.rs` | Add admin scope with `from_fn` admin middleware, reset-password routes with Governor rate limiting |
| `src/web/auth.rs` | Store `session_created_at` in session cookie via `set_session_user()`. Auth middleware checks `password_changed_at > session_created_at` to force re-login after password reset. |
| `src/web/handlers/mod.rs` | Add `pub mod admin;` |
| `src/web/handlers/auth.rs` | Add `reset_password_page` and `reset_password_submit` handlers (unauthenticated, 2 functions) |
| `src/db/users.rs` | Add `ResetToken`, `InviteCodeWithUsers` structs, 8 new DB functions |
| `src/db/schema.rs` | Add `MIGRATION_006` and `if current_version < 6` block |
| `src/spt/profiles.rs` | Add `pub struct SptProfileStats` and `pub fn load_all_profile_stats()` |
| `src/cli/invite.rs` | Remove `generate_invite_code()` and `parse_expiry()`, import from `crate::invite` instead |
| `templates/partials/nav.html` | Add "Admin" link with `can_manage_users()` guard |
| `Cargo.toml` | Add `subtle = "2"` dependency (constant-time token comparison) |
| `TODO.md` | Add stash value as future improvement |

## Out of Scope

- User deletion (disabled accounts are effectively dead)
- Self-service password change (user-initiated, not admin-initiated)
- Audit logging of admin actions
- Multi-use invite codes
- "Forgot password" link on login page
- Stash value in SPT profile cards (future TODO)
- Pagination for user/invite tables (expected scale <20 users; add if needed later)
- Profile stats caching (parse-on-load is acceptable at <20 profiles; add TTL cache if performance becomes an issue)
- Colorblind-accessible badge icons (v1 uses color + text labels; add shape/icon differentiation later)
- Admin CLI recovery command for zero-admin corruption scenarios (recoverable via direct DB manipulation for now)
- Invite code revocation (codes expire naturally or are used)
- Sub-project 3: Mod Request & Voting System
