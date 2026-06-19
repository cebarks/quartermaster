# Mod UI Features — Design Spec

**Date:** 2026-06-18
**Scope:** Mod management UX improvements — sizes, update visibility, Forge link, SSE live updates

## 1. Human-Readable File Sizes

### Askama Custom Filter

Add filter functions in a new `src/web/template_filters.rs` module.

**Two filter functions** (both decorated with `#[askama::filter_fn]`):
- `fn format_size(bytes: &Option<i64>, _env: &dyn askama::Values) -> askama::Result<String>` — for `InstalledFile.file_size` (Option). Returns `"-"` for `None`.
- `fn format_size_i64(bytes: &i64, _env: &dyn askama::Values) -> askama::Result<String>` — for computed totals (bare i64).

Both delegate to a shared `format_bytes(n: i64) -> String` internal function.

**Behavior:**
- `0` → `"0 B"`
- Otherwise, format with binary units: B, KiB, MiB, GiB, TiB
- One decimal place for KiB+, no decimal for B (e.g., `1.2 MiB`, `512 B`)

**Registration:** Askama 0.16 discovers custom filters via a `mod filters` block in scope of each template struct. The filter functions live in `src/web/template_filters.rs` (named `template_filters` to avoid collision with the Askama-expected `filters` module name). Each handler file that has template structs using the filters adds a re-export block:

```rust
mod filters {
    pub use crate::web::template_filters::*;
}
```

This makes the filters available to all template structs in that file. No `askama.toml` needed.

**Template usage:** Replace the `{% if let Some(size) = f.file_size %}{{ size }}{% else %}-{% endif %}` pattern in `detail.html` (both archive and runtime file tables) with `{{ f.file_size|format_size }}`.

## 2. Size Columns on Mod List Page

### DB Query Change

Extend `list_mods_with_file_counts` to also return total size per mod. Return type changes from `Vec<(InstalledMod, usize)>` to `Vec<(InstalledMod, usize, i64)>`. The single existing caller (`handlers/mods.rs` `list_mods` handler) is updated in the same change:

```sql
SELECT m.id, m.forge_mod_id, m.forge_version_id, m.name, m.slug, m.version,
       m.installed_at, m.updated_at,
       COUNT(f.id) as file_count,
       COALESCE(SUM(f.file_size), 0) as total_size
FROM installed_mods m
LEFT JOIN installed_files f ON f.mod_id = m.id
GROUP BY m.id
ORDER BY m.name
```

### View Model Change

Extend `ModListEntry` in `src/web/handlers/mods.rs`:

```rust
struct ModListEntry {
    mod_info: InstalledMod,
    file_count: usize,
    total_size: i64,
}
```

### Template Changes (list.html)

- Add "Size" column header between "Files" and "Installed"
- Add size cell: `<td class="text-muted">{{ m.total_size|format_size_i64 }}</td>` (needs a non-Option variant of the filter, or wrap in `Some()`)
- Add footer row: `<tfoot><tr><td colspan="...">Total: {{ mods.len() }} mods — {{ grand_total_size|format_size_i64 }}</td></tr></tfoot>`

### Handler Change

Compute `grand_total_size: i64` by summing `total_size` across all entries. Pass to `ModListTemplate`.

## 3. Move Buttons & Forge Link on Mod Detail Page

### Button Relocation

Move the admin action buttons (Update, Remove) from below the files tables (currently lines 79-92 of `detail.html`) to immediately after the metadata card (after line 21, before the dependencies card).

### Forge Link

Add a row to the metadata `<table>` in `detail.html`:

```html
{% if let Some(slug) = &mod_info.slug %}
<tr>
    <th>Forge</th>
    <td><a href="https://forge.sp-tarkov.com/mod/{{ mod_info.forge_mod_id }}/{{ slug }}" target="_blank" rel="noopener">View on Forge ↗</a></td>
</tr>
{% endif %}
```

Only rendered when `slug` is `Some` — mods without slugs lack a canonical Forge URL.

## 4. Update Status — Cached Check, Version Badges, Disabled Buttons

### 4a. Config: Update Check Interval

Add to `Config` in `src/config.rs`:

```rust
#[serde(default = "default_update_check_interval")]
pub update_check_interval: u64,
```

Default: `300` (5 minutes). Add `QUMA_UPDATE_CHECK_INTERVAL` env var override in `apply_env_overrides`.

### 4b. Update Cache

New struct in `src/web/update_cache.rs`:

```rust
pub struct UpdateCache {
    inner: Arc<Mutex<Option<(Instant, UpdatesResponseData)>>>,
    ttl: Duration,
}

impl UpdateCache {
    pub fn new(ttl_secs: u64) -> Self { ... }
    pub fn get(&self) -> Option<UpdatesResponseData> { ... }
    pub fn set(&self, data: UpdatesResponseData) { ... }
    pub fn invalidate(&self) { ... }
}
```

- `get()` returns `Some` if cached and within TTL, `None` otherwise
- `set()` stores data with current timestamp
- `invalidate()` clears the cache (called after install/update/remove completes)

Add `update_cache: UpdateCache` to `AppState`.

### 4c. Update Status HTMX Partial

**Endpoint:** `GET /api/mods/update-status`

**Handler:** Check cache → if miss, call `forge.check_updates()` and cache the result → build OOB swap HTML. On Forge API error, return OOB elements with all update buttons disabled and no version arrows (fail gracefully, same pattern as the existing `check_updates_partial` which returns 0 updates on error).

**Coexistence with existing `/api/mods/check-updates`:** The existing endpoint returns a simple badge count for the dashboard nav. It continues to work as-is (the dashboard uses `hx-trigger="load, every 60s"`). The new endpoint returns richer per-mod OOB data for the list page. Both share the same `UpdateCache` — whichever endpoint runs first populates the cache, the other benefits from it. The existing `check_updates_partial` handler should be updated to read from the cache too.

**Response:** Returns multiple `hx-swap-oob="true"` elements:

For each mod in the list, if an update exists:
```html
<td id="mod-version-{id}" hx-swap-oob="true">
    {installed_version} <span style="color: var(--success)">→ {new_version}</span>
</td>
<td id="mod-update-{id}" hx-swap-oob="true">
    <form method="post" action="/mods/{id}/update" style="display:inline">
        <input type="hidden" name="csrf_token" value="...">
        <button type="submit" class="btn btn-sm btn-outline">Update</button>
    </form>
</td>
```

For mods with no update available:
```html
<td id="mod-update-{id}" hx-swap-oob="true">
    <form method="post" action="/mods/{id}/update" style="display:inline">
        <input type="hidden" name="csrf_token" value="...">
        <button type="submit" class="btn btn-sm btn-outline" disabled>Update</button>
    </form>
</td>
```

### 4d. Template Changes (list.html)

- Version cells get IDs: `<td id="mod-version-{{ m.mod_info.id }}">`
- Update button cells get IDs: `<td id="mod-update-{{ m.mod_info.id }}">`
- One trigger element fetches update status on page load:
  ```html
  <span hx-get="/api/mods/update-status" hx-trigger="load, sse:modsChanged from:#sse-source" hx-swap="none"></span>
  ```
  This fires on initial load and again whenever mods change (via SSE).

### 4e. Data Flow for Update Check

To map Forge update results (keyed by `forge_mod_id`) back to template element IDs (keyed by DB `id`), the handler needs the mapping. Options:
- Query `list_mods()` from DB to build a `forge_mod_id → db_id` map
- Pass both IDs through the template (already available: `m.mod_info.id` and `m.mod_info.forge_mod_id`)

The handler will query `list_mods()` to get the mapping, then cross-reference with the update check results.

## 5. SSE-Driven Live Updates

### 5a. Broadcast Channel

Add to `AppState`:

```rust
pub events: tokio::sync::broadcast::Sender<ServerEvent>,
```

```rust
#[derive(Clone, Debug)]
pub enum ServerEvent {
    TaskChanged,
    ModsChanged,
}
```

Initialize with `tokio::sync::broadcast::channel(64)` (small buffer, events are lightweight signals).

### 5b. TaskTracker Integration

Modify `TaskTracker` to accept a `broadcast::Sender<ServerEvent>`:
- `start()` → sends `TaskChanged`
- `complete()` → sends `TaskChanged` + `ModsChanged`
- `fail()` → sends `TaskChanged`
- `update_message()` → sends `TaskChanged`

The sender is passed at construction: `TaskTracker::new(events_tx: broadcast::Sender<ServerEvent>)`.

### 5c. SSE Endpoint

**Route:** `GET /api/events`
**Auth:** Requires authentication (via `require_auth`)

**Handler:** Returns `HttpResponse` with `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`.

Uses `futures_util::stream::unfold` (already a dependency) to bridge `broadcast::Receiver` into an Actix-web streaming response:

```rust
let stream = futures_util::stream::unfold(rx, |mut rx| async move {
    loop {
        match rx.recv().await {
            Ok(event) => {
                let msg = match event {
                    ServerEvent::TaskChanged => "event: taskChanged\ndata: \n\n",
                    ServerEvent::ModsChanged => "event: modsChanged\ndata: \n\n",
                };
                return Some((Ok::<_, actix_web::Error>(web::Bytes::from(msg)), rx));
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
});
HttpResponse::Ok()
    .content_type("text/event-stream")
    .insert_header(("Cache-Control", "no-cache"))
    .streaming(stream)
```

**Error handling:** `RecvError::Lagged` (slow consumer missed events) is silently skipped — the next event will trigger a refresh anyway. `RecvError::Closed` (sender dropped) ends the stream cleanly.

The stream ends when the client disconnects (receiver is dropped) or the broadcast sender is dropped.

### 5d. HTMX SSE Extension & Connection in Base Template

**HTMX SSE extension:** The SSE extension is NOT bundled with htmx 2.x core — it must be loaded separately. Download `sse.js` from the [htmx extensions repo](https://github.com/bigskysoftware/htmx-ext-sse) and add it to `src/assets/sse.js` (embedded via the existing static asset serving). Add a `<script src="/assets/sse.js"></script>` tag in `base.html` after the htmx script.

**SSE connection element:** Add to `base.html`, inside `<body>` but outside `<main>` (so it's not affected by content swaps):

```html
<div id="sse-source" hx-ext="sse" sse-connect="/api/events" style="display:none"></div>
```

This provides the SSE connection on all authenticated pages. Individual elements subscribe to events via `hx-trigger="sse:eventName from:#sse-source"`.

**Reconnection:** The HTMX SSE extension has built-in reconnection with exponential backoff. If the connection drops, it automatically reconnects. Events missed during disconnect are acceptable — they're lightweight signals, and the next event triggers a fresh data fetch.

**Connection scope:** Every authenticated page maintains an SSE connection. This is a deliberate tradeoff — it simplifies the implementation and future-proofs for server status updates, container stats, etc. The cost is one long-lived HTTP connection per open browser tab.

### 5e. Task Status Template Changes

In `partials/task_status.html`, replace the polling trigger:

**Before:**
```html
{% if has_active %}hx-get="/api/tasks/status" hx-trigger="every 2s" hx-swap="outerHTML"{% endif %}
```

**After:**
```html
hx-get="/api/tasks/status" hx-trigger="sse:taskChanged from:#sse-source" hx-swap="outerHTML"
```

Remove the `has_active` conditional on the trigger — SSE events only fire when something actually changes, so there's no cost to always listening.

**Initial load:** The initial `<div id="task-status">` in page templates (e.g., `list.html` line 7) retains `hx-trigger="load"` so the task status partial is fetched on page load. The SSE trigger then handles subsequent updates. The initial load is necessary because SSE won't fire a retroactive event just because the page loaded.

### 5f. Mod List Auto-Refresh

New partial endpoint: `GET /api/mods/list`

**Handler:** Queries `list_mods_with_file_counts` (the extended version with sizes) and renders just the `<tbody>` rows + `<tfoot>`.

**Template:** `templates/mods/partials/list_body.html` — the table body extracted from `list.html`.

**list.html changes:**
- The `<tbody>` gets: `id="mod-list-body" hx-get="/api/mods/list" hx-trigger="sse:modsChanged from:#sse-source" hx-swap="innerHTML"`
- On `modsChanged` event, the table body refreshes in place

### 5g. Cache Invalidation on Mod Changes

When a mod is installed, updated, or removed, call `update_cache.invalidate()` so the next update-status fetch gets fresh data. This happens in the async task completion paths in the install/update/remove handlers.

## Files Affected

### New Files
- `src/web/template_filters.rs` — `format_size` / `format_size_i64` Askama filter functions (with `#[askama::filter_fn]`)
- `src/web/update_cache.rs` — `UpdateCache` struct
- `src/web/sse.rs` — `ServerEvent` enum, SSE endpoint handler
- `src/assets/sse.js` — HTMX SSE extension (downloaded from htmx-ext-sse repo)
- `templates/mods/partials/list_body.html` — mod list tbody partial
- `templates/mods/partials/update_status.html` — OOB swap partial for version badges

### Modified Files
- `src/config.rs` — add `update_check_interval` field + default + env override
- `src/web/state.rs` — add `update_cache: UpdateCache` and `events: broadcast::Sender<ServerEvent>`
- `src/web/mod.rs` — register new routes (`/api/events`, `/api/mods/list`, `/api/mods/update-status`), add modules
- `src/web/tasks.rs` — `TaskTracker` takes broadcast sender, sends events on state changes
- `src/web/handlers/mods.rs` — extend `ModListEntry` with `total_size`, add `grand_total_size` to template, add update-status + list-body handlers, invalidate cache on install/update/remove completion, add `mod filters { pub use crate::web::template_filters::*; }` re-export
- `src/db/mods.rs` — extend `list_mods_with_file_counts` query to include `SUM(file_size)`
- `templates/base.html` — add `<script src="/assets/sse.js">` after htmx script, add `#sse-source` div inside `<body>` outside `<main>`
- `templates/mods/list.html` — size column, footer row, element IDs for OOB swaps, SSE triggers, update-status fetch trigger
- `templates/mods/detail.html` — move buttons above files, add Forge link, use `format_size` filter
- `templates/partials/task_status.html` — replace polling with SSE trigger

### Dependencies
- `tokio` — already included with `features = ["full"]`; `broadcast` channel is in `tokio::sync`
- `futures-util` — already included; used for `stream::unfold` to bridge broadcast receiver to SSE stream
- No new crate dependencies required
- One new vendored JS file: `sse.js` (HTMX SSE extension, ~4KB)

## Out of Scope

- Configurable update check interval UI (settings page) — config file/env var only for now
- Push notifications for update availability (SSE only signals task/mod changes, not proactive update alerts)
- Mod search/browse from Forge within the UI
- Batch select/update from the mod list
