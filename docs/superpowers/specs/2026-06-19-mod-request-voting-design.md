# Mod Request & Voting System — Design Spec

**Sub-project 3** of the Multi-User Support initiative.

**Goal:** Let players request mods from SPT Forge via the web UI, vote on each other's requests with optional comments, and let admins/moderators approve or reject requests with optional auto-install.

**Prerequisites:** Sub-project 1 (Permission Model & Session Overhaul) and Sub-project 2 (Admin User Management UI) are complete. The Role enum (Admin/Moderator/Player) with capability methods, account management, invite codes, and SPT profile stats are all in place.

---

## 1. Data Model

### 1.1 `mod_requests` table (migration 007)

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | INTEGER | PRIMARY KEY AUTOINCREMENT | |
| user_id | INTEGER | NOT NULL, FK → users(id) | requester |
| forge_mod_id | INTEGER | NOT NULL | Forge mod ID |
| mod_name | TEXT | NOT NULL | cached from Forge at creation |
| mod_slug | TEXT | | cached slug (nullable, some mods lack slugs) |
| mod_description | TEXT | | cached description |
| fika_compatible | TEXT | NOT NULL DEFAULT 'unknown' | cached as `compatible`, `incompatible`, or `unknown` (matches `FikaCompat` enum) |
| reason | TEXT | | optional player comment |
| status | TEXT | NOT NULL DEFAULT 'pending' | `pending`, `approved`, `rejected` |
| resolved_by | INTEGER | FK → users(id) | admin/mod who resolved |
| resolved_at | TEXT | | RFC3339 timestamp |
| resolve_comment | TEXT | | admin's reason |
| created_at | TEXT | NOT NULL DEFAULT (datetime('now')) | |
| forge_cached_at | TEXT | NOT NULL DEFAULT (datetime('now')) | last Forge metadata refresh |

**Constraints:**
- `UNIQUE(forge_mod_id)` where `status = 'pending'` — enforced in application code (SQLite partial unique indexes require specific syntax). Prevents duplicate pending requests for the same mod.

### 1.2 `mod_request_votes` table (migration 007)

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | INTEGER | PRIMARY KEY AUTOINCREMENT | |
| request_id | INTEGER | NOT NULL, FK → mod_requests(id) ON DELETE CASCADE | |
| user_id | INTEGER | NOT NULL, FK → users(id) | voter |
| upvote | INTEGER | NOT NULL | 1 = upvote, 0 = downvote |
| comment | TEXT | | optional |
| created_at | TEXT | NOT NULL DEFAULT (datetime('now')) | |

**Constraints:**
- `UNIQUE(request_id, user_id)` — one vote per user per request. Changing vote uses `INSERT OR REPLACE`.

### 1.3 Duplicate prevention

Before creating a request, the server checks:
1. **Already installed:** Query `installed_mods` for matching `forge_mod_id`. If found, reject with message "This mod is already installed."
2. **Pending request exists:** Query `mod_requests` for matching `forge_mod_id` with `status = 'pending'`. If found, reject with message "A pending request for this mod already exists." (Link to the existing request.)

Approved/rejected requests do NOT block new requests for the same mod — a previously rejected mod can be re-requested.

---

## 2. Request Creation Flow

### 2.1 Search input (HTMX live search)

A text input with HTMX attributes:
- `hx-get="/api/requests/search?q=..."` triggered on `input` event with `delay:300ms` (debounce)
- `hx-target` points to a results container below the input
- `hx-indicator` shows a loading spinner during search

The server endpoint proxies to `ForgeClient::search_mods(query)` and returns an HTML partial with selectable mod cards.

### 2.2 Direct URL/ID input

The same input field accepts:
- **Numeric ID:** Detected by the server if the query is purely numeric. Calls `ForgeClient::get_mod(id, false)` directly.
- **Forge URL:** Pattern `https://forge.sp-tarkov.com/mods/{id}-{slug}` or similar. Server extracts the numeric ID prefix from the path segment after `/mods/`. Calls `get_mod(id, false)`.

Both cases return the same search result partial, showing a single mod card.

### 2.3 Mod card selection

Each search result card is a clickable element that:
- Highlights as selected
- Populates a hidden `forge_mod_id` field in the form
- Shows the selected mod's name, description snippet, and Fika badge above the reason textarea

### 2.4 Form submission

`POST /api/requests` with:
- `forge_mod_id` (required, from card selection)
- `reason` (optional textarea)
- `csrf_token`

Server validates:
1. CSRF token
2. `forge_mod_id` exists on Forge (re-fetch to get fresh metadata for caching)
3. Mod not already installed
4. No existing pending request for this mod
5. User is authenticated (any role)

On success: caches Forge metadata in `mod_requests`, sets flash message, returns updated request list partial.

### 2.5 Rate limiting

The search endpoint (`GET /api/requests/search`) is rate-limited via Governor at 10 requests/min/IP to avoid hammering the Forge API.

---

## 3. Requests List & Filtering

### 3.1 Layout

The Requests tab shows request cards sorted by **net vote score** (upvotes - downvotes, descending), with secondary sort by `created_at` descending for equal scores.

### 3.2 Status filters

Simple filter links/buttons at the top of the tab:
- **Pending** (default) — shows only `status = 'pending'`
- **Approved** — shows `status = 'approved'`
- **Rejected** — shows `status = 'rejected'`
- **All** — shows everything

Filter selection uses HTMX `hx-get` with a `status` query parameter, swapping the card list. Active filter is visually highlighted.

### 3.3 Request card contents

Each card displays:
- **Mod name** (linked to Forge page: `https://forge.sp-tarkov.com/mods/{forge_mod_id}-{slug}`)
- **Fika compatibility badge** (compatible/incompatible/unknown)
- **Description snippet** (first ~150 characters of cached description, truncated with ellipsis)
- **Requester** username + relative time ("requested 3 days ago")
- **Vote score** (net: upvotes - downvotes) displayed prominently
- **Vote breakdown** (e.g., "5 up / 2 down")
- **Upvote / downvote buttons** — highlighted if the current user has voted, toggleable
- **Comment count** badge (number of vote comments)
- **Status badge** (Pending = blue, Approved = green, Rejected = red)
- **Requester's reason** if present (below description, italicized or in a quote block)

### 3.4 Admin/moderator actions

Visible only to users with `can_manage_mods()`, on pending requests only:
- **Approve** and **Reject** buttons
- Clicking either expands an inline form with:
  - Textarea for resolve comment (optional)
  - For Approve only: checkbox "Install mod now" (default unchecked)
  - Submit button

---

## 4. Voting

### 4.1 Vote mechanics

- Any authenticated user can vote on any pending request (including their own)
- One vote per user per request
- Clicking upvote when you haven't voted: creates upvote
- Clicking downvote when you haven't voted: creates downvote
- Clicking upvote when you've already upvoted: removes your vote (toggle off)
- Clicking downvote when you've already downvoted: removes your vote (toggle off)
- Clicking upvote when you've downvoted: changes to upvote (and vice versa)

### 4.2 Vote with comment

When casting a vote, an optional comment textarea is available. The textarea expands on click (not always visible to keep the UI compact). Comments are attached to the vote record.

### 4.3 HTMX interaction

`POST /api/requests/{id}/vote` with:
- `upvote` (boolean)
- `comment` (optional)
- `csrf_token`

Returns an HTML partial that replaces the vote buttons + score section of the card. This keeps the interaction snappy without a full page reload.

### 4.4 Removing a vote

When a user toggles off their vote, the server deletes the vote row from `mod_request_votes`. The HTMX response re-renders the vote section with no highlight on either button.

### 4.5 Vote comments list

A "View comments" link (with comment count) on each card triggers `GET /api/requests/{id}/votes` via `hx-get`, which returns an expandable HTML partial showing:
- Voter username
- Up/down indicator icon
- Comment text (if present; votes without comments are omitted from this list)
- Relative timestamp

---

## 5. Request Resolution (Approve / Reject)

### 5.1 Resolution flow

`POST /api/requests/{id}/resolve` with:
- `action` (`approve` or `reject`)
- `comment` (optional)
- `install` (boolean, only meaningful for approve)
- `csrf_token`

Server validates:
1. CSRF token
2. User has `can_manage_mods()` capability
3. Request exists and has `status = 'pending'`

On resolve:
- Sets `status`, `resolved_by`, `resolved_at` (RFC3339 via chrono), `resolve_comment`
- If `action = approve` and `install = true`: triggers the existing mod install flow (`ops::install_mod_from_archive`) or queues it via `PendingOperation` if the server is running

### 5.2 Install-on-approve

When the admin checks "Install mod now" on approval:
1. Fetch the latest compatible version from Forge (`get_versions(mod_id, spt_version)`)
2. If server is running: create a `PendingOperation` with `action = "install"`, flash message "Mod approved and queued for install"
3. If server is stopped: download and install immediately via the existing install flow, flash message "Mod approved and installed"
4. If no compatible version exists: approve the request but skip install, flash message "Mod approved but no compatible version found for SPT {version}"

### 5.3 Post-resolution display

Resolved requests show:
- Status badge (Approved/Rejected)
- "Resolved by {username} {relative_time}" line
- Admin's comment if present
- Voting buttons disabled (no voting on resolved requests)

---

## 6. Forge Metadata Cache Refresh

### 6.1 Staleness check

When rendering a request card, compare `forge_cached_at` against `now - forge_cache_ttl`. If `forge_cached_at` is older than the TTL, the metadata is stale.

### 6.2 Lazy per-item refresh

When a stale request is rendered:
1. Serve the current cached data immediately (no page load delay)
2. Spawn a `tokio::spawn` background task that:
   - Calls `ForgeClient::get_mod(forge_mod_id, false)`
   - On success: updates `mod_name`, `mod_slug`, `mod_description`, `fika_compatible` (as string: `compatible`/`incompatible`/`unknown`), `forge_cached_at` in the database
   - On failure: logs a `tracing::warn!` and leaves the stale cache in place
3. The next page load shows the refreshed data

### 6.3 Configuration

New field in `Config` (`src/config.rs`):
```
forge_cache_ttl: Option<u64>  // seconds, default 86400 (24 hours)
```

Settable in `quartermaster.toml`:
```toml
forge_cache_ttl = 86400
```

Or via environment variable: `QUMA_FORGE_CACHE_TTL=86400`

---

## 7. Routing & Handler Structure

### 7.1 Routes

All routes added to `src/web/mod.rs`:

**Authenticated scope (any user):**
- `GET /api/requests/search?q=...` — Forge search proxy (Governor: 10/min/IP)
- `GET /api/mods/requests?status=...` — Requests tab content (HTMX partial)
- `POST /api/requests` — create request
- `POST /api/requests/{id}/vote` — cast/change/remove vote
- `GET /api/requests/{id}/votes` — vote comments (HTMX partial)

**Mod-management (inline `can_manage_mods()` check):**
- `POST /api/requests/{id}/resolve` — approve/reject

### 7.2 Files

- **Handler:** `src/web/handlers/requests.rs` — all request/vote/resolve handlers
- **DB module:** `src/db/requests.rs` — CRUD for `mod_requests` and `mod_request_votes`
- **Migration:** `migrations/007_mod_requests.sql`

### 7.3 Middleware

No new middleware. The resolve endpoint checks `can_manage_mods()` inline via `require_capability()`, consistent with admin.rs pattern for single endpoints outside a scoped middleware group.

---

## 8. Mods Page Tab Integration

### 8.1 Tab bar

Add a tab bar to `templates/mods.html` with two tabs:
- **Installed** (default) — existing mod list content, unchanged
- **Requests** — loads `/api/mods/requests` via `hx-get` on tab click

Tab state persisted via URL hash (`#installed` / `#requests`), same JavaScript pattern as the admin page's tab bar.

### 8.2 Templates

| Template | Purpose |
|----------|---------|
| `templates/mods.html` | Modified: add tab bar |
| `templates/mods/partials/requests.html` | Request list with status filters and "Request a Mod" button |
| `templates/mods/partials/request_form.html` | Search + create form (HTMX inline) |
| `templates/mods/partials/request_card.html` | Single request card (vote buttons, status, admin actions) |
| `templates/mods/partials/search_results.html` | Forge search result cards (selectable) |
| `templates/mods/partials/vote_comments.html` | Expandable vote comment list |

### 8.3 CSS

New classes added to `src/assets/style.css`:
- `.request-card` — card layout for requests
- `.vote-buttons` — upvote/downvote button group
- `.vote-active` — highlighted state for user's current vote
- `.vote-score` — prominent score display
- `.search-results` — search result container
- `.search-card` — selectable Forge mod card
- `.search-card.selected` — selected state
- `.status-filter` — filter button group
- `.resolve-form` — inline approve/reject form
- `.fika-badge` — Fika compatibility indicator (may already exist)

---

## 9. Capability & Access Control

No new Role capabilities are introduced. Access control uses existing capabilities:

| Action | Who | Enforced by |
|--------|-----|-------------|
| View requests tab | Any authenticated user | Auth middleware on `/api/mods/requests` |
| Create a request | Any authenticated user | Auth middleware on `POST /api/requests` |
| Vote on a request | Any authenticated user | Auth middleware on `POST /api/requests/{id}/vote` |
| Search Forge mods | Any authenticated user | Auth middleware + Governor on `/api/requests/search` |
| Approve/reject | Admin or Moderator | Inline `can_manage_mods()` check in resolve handler |

Disabled users are already blocked at the auth middleware level and cannot perform any actions.

---

## 10. Edge Cases & Error Handling

| Scenario | Behavior |
|----------|----------|
| Forge API unreachable during search | Return empty results with a message: "Could not reach SPT Forge. Try again later." |
| Forge API unreachable during request creation | Reject with error: "Could not verify mod on SPT Forge." |
| Forge API unreachable during cache refresh | Log warning, serve stale cache |
| Mod removed from Forge after request created | Cached data persists. Cache refresh fails silently. Card shows cached info. |
| User tries to request an installed mod | Block with message: "This mod is already installed." |
| User tries to request a mod with pending request | Block with message: "A pending request for this mod already exists." |
| Admin approves but no compatible SPT version exists | Approve the request, skip install, flash: "Approved but no compatible version found for SPT {version}." |
| Vote on a resolved request | Reject: voting is only allowed on pending requests |
| Admin tries to resolve an already-resolved request | Reject with error: "This request has already been resolved." |
| Request creator votes on their own request | Allowed — no restriction |
| Search query too short (< 2 chars) | Return empty results, no Forge API call |
