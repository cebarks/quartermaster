# Mod UI Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add human-readable file sizes, mod size columns, Forge links, cached update-status badges, and SSE-driven live updates to the Quartermaster web UI.

**Architecture:** Server-rendered HTML (Askama 0.16) + HTMX 2.x with SSE extension for push updates. Update checks are cached server-side with a configurable TTL. SSE replaces polling for task status and mod list refresh. All new endpoints are HTMX partials returning HTML fragments.

**Tech Stack:** Rust, Actix-web 4, Askama 0.16, HTMX 2.x + SSE extension, SQLite (rusqlite), tokio broadcast channels, futures-util

## Global Constraints

- No new crate dependencies — use existing `futures-util`, `tokio` (full features), `parking_lot`
- One new vendored JS file: HTMX SSE extension (`sse.js`, ~4KB)
- Askama 0.16 custom filters require `#[askama::filter_fn]` attribute and `&dyn askama::Values` parameter
- Askama discovers filters via `mod filters` in scope of the template struct — use re-export pattern
- All new API endpoints go under `/api` scope with `RequireAuth` middleware
- Run `cargo test` after each task; run `cargo build` to verify template compilation
- Spec: `docs/superpowers/specs/2026-06-18-mod-ui-features-design.md`

---

### Task 1: Human-Readable File Size Filter + Detail Page Sizes

Creates the `format_size` Askama filter and applies it to the mod detail page. This is the foundation — later tasks use the same filter.

**Files:**
- Create: `src/web/template_filters.rs`
- Modify: `src/web/mod.rs:1-7` (add module declaration)
- Modify: `src/web/handlers/mods.rs:1-12` (add `mod filters` re-export)
- Modify: `templates/mods/detail.html:54,71` (use filter in both file tables)

**Interfaces:**
- Produces: `crate::web::template_filters::format_size` — `fn(bytes: &Option<i64>, _env: &dyn askama::Values) -> askama::Result<String>`
- Produces: `crate::web::template_filters::format_size_i64` — `fn(bytes: &i64, _env: &dyn askama::Values) -> askama::Result<String>`
- Produces: `crate::web::template_filters::format_bytes` — `fn(n: i64) -> String` (internal helper, also unit-tested)

- [ ] **Step 1: Write failing tests for `format_bytes`**

Create `src/web/template_filters.rs` with the test module first:

```rust
fn format_bytes(n: i64) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_bytes_kib() {
        assert_eq!(format_bytes(1536), "1.5 KiB");
    }

    #[test]
    fn format_bytes_mib() {
        assert_eq!(format_bytes(2_621_440), "2.5 MiB");
    }

    #[test]
    fn format_bytes_gib() {
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }

    #[test]
    fn format_bytes_exact_kib() {
        assert_eq!(format_bytes(1024), "1.0 KiB");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib web::template_filters -- --nocapture`
Expected: all 6 tests fail with `not yet implemented`

- [ ] **Step 3: Implement `format_bytes` and filter functions**

Replace the `todo!()` stub and add filter functions in `src/web/template_filters.rs`:

```rust
fn format_bytes(n: i64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    if n == 0 {
        return "0 B".to_string();
    }
    let n = n as f64;
    let i = (n.log2() / 10.0).floor() as usize;
    let i = i.min(UNITS.len() - 1);
    let val = n / (1u64 << (i * 10)) as f64;
    if i == 0 {
        format!("{} B", val as i64)
    } else {
        format!("{:.1} {}", val, UNITS[i])
    }
}

#[askama::filter_fn]
pub fn format_size(bytes: &Option<i64>, _env: &dyn askama::Values) -> askama::Result<String> {
    match bytes {
        Some(n) => Ok(format_bytes(*n)),
        None => Ok("-".to_string()),
    }
}

#[askama::filter_fn]
pub fn format_size_i64(bytes: &i64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format_bytes(*bytes))
}
```

- [ ] **Step 4: Register the module in `src/web/mod.rs`**

Add `pub mod template_filters;` to the module declarations at the top of `src/web/mod.rs` (after line 7, alongside the other `pub mod` statements):

```rust
pub mod auth;
pub mod csrf;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod state;
pub mod tasks;
pub mod template_filters;
```

- [ ] **Step 5: Add filter re-export in `src/web/handlers/mods.rs`**

Add this block after the existing `use` statements (after line 11) and before the `// -- View models --` comment:

```rust
mod filters {
    pub use crate::web::template_filters::*;
}
```

- [ ] **Step 6: Update `detail.html` to use the filter**

In `templates/mods/detail.html`, replace line 54:
```
<td class="text-muted text-sm">{% if let Some(size) = f.file_size %}{{ size }}{% else %}-{% endif %}</td>
```
with:
```
<td class="text-muted text-sm">{{ f.file_size|format_size }}</td>
```

Do the same replacement at line 71 (runtime files table — identical pattern).

- [ ] **Step 7: Run tests and build**

Run: `cargo test --lib web::template_filters -- --nocapture`
Expected: all 6 tests pass

Run: `cargo build`
Expected: compiles successfully (verifies Askama template compilation with the new filter)

- [ ] **Step 8: Commit**

```bash
git add src/web/template_filters.rs src/web/mod.rs src/web/handlers/mods.rs templates/mods/detail.html
git commit -m "feat: add human-readable file size filter and apply to mod detail page"
```

---

### Task 2: Size Columns on Mod List Page

Extends the DB query to return per-mod total size, adds a Size column and total footer to the installed mods list.

**Files:**
- Modify: `src/db/mods.rs:109-124` (extend `list_mods_with_file_counts` query and return type)
- Modify: `src/web/handlers/mods.rs:15-18,27-34,77-106` (extend `ModListEntry`, `ModListTemplate`, `list_mods` handler)
- Modify: `templates/mods/list.html:30-65` (add Size column header, cell, footer row)

**Interfaces:**
- Consumes: `crate::web::template_filters::format_size_i64` from Task 1
- Produces: `Database::list_mods_with_file_counts` — returns `Vec<(InstalledMod, usize, i64)>` (mod, file_count, total_size)

- [ ] **Step 1: Write failing test for the extended DB query**

Add a test in `src/db/mods.rs` at the bottom of the file, inside a new `#[cfg(test)] mod tests` block (the module doesn't have one yet — `src/db/tests.rs` exists separately, but inline tests work too):

```rust
#[cfg(test)]
mod tests {
    use crate::db::Database;

    #[test]
    fn list_mods_with_file_counts_includes_total_size() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0").unwrap();
        db.insert_file(mod_id, "file1.dll", None, Some(1024)).unwrap();
        db.insert_file(mod_id, "file2.dll", None, Some(2048)).unwrap();

        let results = db.list_mods_with_file_counts().unwrap();
        assert_eq!(results.len(), 1);
        let (m, count, size) = &results[0];
        assert_eq!(m.name, "TestMod");
        assert_eq!(*count, 2);
        assert_eq!(*size, 3072);
    }

    #[test]
    fn list_mods_with_file_counts_zero_size_when_no_files() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(100, 200, "EmptyMod", None, "1.0.0").unwrap();

        let results = db.list_mods_with_file_counts().unwrap();
        assert_eq!(results.len(), 1);
        let (_, count, size) = &results[0];
        assert_eq!(*count, 0);
        assert_eq!(*size, 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib db::mods::tests -- --nocapture`
Expected: compilation error — `list_mods_with_file_counts` returns `Vec<(InstalledMod, usize)>`, not a 3-tuple

- [ ] **Step 3: Extend the DB query**

In `src/db/mods.rs`, modify `list_mods_with_file_counts` (lines 109-124). Change the return type and query:

```rust
pub fn list_mods_with_file_counts(&self) -> rusqlite::Result<Vec<(InstalledMod, usize, i64)>> {
    let mut stmt = self.conn.prepare(
        "SELECT m.id, m.forge_mod_id, m.forge_version_id, m.name, m.slug, m.version,
                m.installed_at, m.updated_at, COUNT(f.id) as file_count,
                COALESCE(SUM(f.file_size), 0) as total_size
         FROM installed_mods m
         LEFT JOIN installed_files f ON f.mod_id = m.id
         GROUP BY m.id
         ORDER BY m.name",
    )?;
    let rows = stmt.query_map([], |row| {
        let m = row_to_installed_mod(row)?;
        let count: usize = row.get(8)?;
        let size: i64 = row.get(9)?;
        Ok((m, count, size))
    })?;
    rows.collect()
}
```

- [ ] **Step 4: Update the handler and view model**

In `src/web/handlers/mods.rs`, update `ModListEntry` (around line 15):

```rust
struct ModListEntry {
    mod_info: InstalledMod,
    file_count: usize,
    total_size: i64,
}
```

Update `ModListTemplate` (around line 29) to add `grand_total_size`:

```rust
#[derive(Template)]
#[template(path = "mods/list.html")]
struct ModListTemplate {
    user: SessionUser,
    mods: Vec<ModListEntry>,
    grand_total_size: i64,
    flash: Option<FlashMessage>,
    csrf_token: String,
}
```

Update the `list_mods` handler (around line 83) to destructure the 3-tuple:

```rust
let mods = web::block(move || {
    let db = db.lock();
    let mods_with_counts = db.list_mods_with_file_counts()?;
    let entries: Vec<ModListEntry> = mods_with_counts
        .into_iter()
        .map(|(mod_info, file_count, total_size)| ModListEntry {
            mod_info,
            file_count,
            total_size,
        })
        .collect();
    Ok::<_, anyhow::Error>(entries)
})
.await
.map_err(WebError::from)?
.map_err(WebError::from)?;

let grand_total_size: i64 = mods.iter().map(|m| m.total_size).sum();
```

And pass `grand_total_size` to the template struct.

- [ ] **Step 5: Update `list.html` template**

In `templates/mods/list.html`, add the Size column header (after the "Files" `<th>` at line 35):

```html
<th>Size</th>
```

Add the size cell in the `<tbody>` row (after the file count cell at line 45):

```html
<td class="text-muted">{{ m.total_size|format_size_i64 }}</td>
```

Add a footer after the `</tbody>` (before `</table>`):

```html
<tfoot>
    <tr>
        <td colspan="{% if user.is_admin() %}6{% else %}5{% endif %}" class="text-muted text-sm">
            Total: {{ mods.len() }} mods — {{ grand_total_size|format_size_i64 }}
        </td>
    </tr>
</tfoot>
```

Note: the colspan adjusts for the Actions column (admin-only). The column count is now: Name, Version, Files, Size, Installed, [Actions] = 5 or 6.

- [ ] **Step 6: Run tests and build**

Run: `cargo test --lib db::mods::tests -- --nocapture`
Expected: both new tests pass

Run: `cargo build`
Expected: compiles (verifies template + handler alignment)

- [ ] **Step 7: Commit**

```bash
git add src/db/mods.rs src/web/handlers/mods.rs templates/mods/list.html
git commit -m "feat: add size column and total footer to installed mods list"
```

---

### Task 3: Move Buttons & Forge Link on Mod Detail Page

Moves admin action buttons above the files tables and adds a Forge link to the metadata card.

**Files:**
- Modify: `templates/mods/detail.html:7-93` (reorder buttons, add Forge link row)

**Interfaces:**
- Consumes: `mod_info.forge_mod_id` (i64), `mod_info.slug` (Option<String>) — already in template context from `ModDetailTemplate`

- [ ] **Step 1: Add the Forge link row to the metadata table**

In `templates/mods/detail.html`, add after the updated_at row (after line 19, before the closing `</table>`):

```html
{% if let Some(slug) = &mod_info.slug %}
<tr><th>Forge</th><td><a href="https://forge.sp-tarkov.com/mod/{{ mod_info.forge_mod_id }}/{{ slug }}" target="_blank" rel="noopener">View on Forge ↗</a></td></tr>
{% endif %}
```

- [ ] **Step 2: Move the action buttons above the files tables**

Cut the admin button block (lines 79-92):
```html
{% if user.is_admin() %}
<div class="flex gap-1 mt-2">
    ...buttons...
</div>
{% endif %}
```

Paste it immediately after `</div>` that closes the metadata card (after line 21). Change `mt-2` to `mb-2` so it has spacing below instead of above:

```html
</div>

{% if user.is_admin() %}
<div class="flex gap-1 mb-2">
    <form method="post" action="/mods/{{ mod_info.id }}/update">
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
        <button type="submit" class="btn">{% call icons::refresh() %}{% endcall %} Update</button>
    </form>
    <form method="post" action="/mods/{{ mod_info.id }}/remove"
          data-name="{{ mod_info.name }}"
          onsubmit="return confirm('Remove ' + this.dataset.name + '?')">
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
        <button type="submit" class="btn btn-danger">{% call icons::trash() %}{% endcall %} Remove</button>
    </form>
</div>
{% endif %}

{% if !dependencies.is_empty() %}
```

Remove the old button block from the bottom of the file (it should no longer exist there).

- [ ] **Step 3: Build to verify template compiles**

Run: `cargo build`
Expected: compiles successfully

- [ ] **Step 4: Commit**

```bash
git add templates/mods/detail.html
git commit -m "feat: move action buttons above files table and add Forge link"
```

---

### Task 4: Update Check Config + Cache

Adds the `update_check_interval` config field and the `UpdateCache` struct. No template changes — this is pure backend infrastructure for Task 5.

**Files:**
- Modify: `src/config.rs:12-26,28-59,61-76,127-153` (add field, default fn, Default impl, env override)
- Create: `src/web/update_cache.rs`
- Modify: `src/web/mod.rs` (add module declaration)
- Modify: `src/web/state.rs` (add `update_cache` field)
- Modify: `src/web/mod.rs:54-61` (construct `UpdateCache` in `start_server`)

**Interfaces:**
- Produces: `Config::update_check_interval` — `u64` (seconds), default 300
- Produces: `UpdateCache::new(ttl_secs: u64) -> Self`
- Produces: `UpdateCache::get(&self) -> Option<UpdatesResponseData>`
- Produces: `UpdateCache::set(&self, data: UpdatesResponseData)`
- Produces: `UpdateCache::invalidate(&self)`

- [ ] **Step 1: Write failing test for config field**

Add to the existing `tests` module in `src/config.rs`:

```rust
#[test]
fn update_check_interval_default() {
    let config: Config = toml::from_str("").expect("should parse empty TOML");
    assert_eq!(config.update_check_interval, 300);
}

#[test]
fn update_check_interval_custom() {
    let config: Config = toml::from_str("update_check_interval = 60").expect("should parse");
    assert_eq!(config.update_check_interval, 60);
}

#[test]
fn update_check_interval_env_override() {
    temp_env::with_vars([("QUMA_UPDATE_CHECK_INTERVAL", Some("120"))], || {
        let mut config = Config::default();
        config.apply_env_overrides();
        assert_eq!(config.update_check_interval, 120);
    });
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib config::tests::update_check_interval -- --nocapture`
Expected: compilation error — `update_check_interval` field doesn't exist

- [ ] **Step 3: Add `update_check_interval` to `Config`**

In `src/config.rs`, add the default function (after line 24, near the other defaults):

```rust
fn default_update_check_interval() -> u64 {
    300
}
```

Add the field to the `Config` struct (after `web_port` at line 58):

```rust
#[serde(default = "default_update_check_interval")]
pub update_check_interval: u64,
```

Add to the `Default` impl (after `web_port: 9190,` at line 74):

```rust
update_check_interval: 300,
```

Add to `apply_env_overrides` (after the `QUMA_SERVER_PORT` block at line 152):

```rust
if let Ok(val) = std::env::var("QUMA_UPDATE_CHECK_INTERVAL") {
    if let Ok(secs) = val.parse::<u64>() {
        self.update_check_interval = secs;
    }
}
```

Update the existing `deserialize_full_config` test to include the new field in the TOML string and assertion. Add `update_check_interval = 600` to the TOML and `assert_eq!(config.update_check_interval, 600);` to the assertions.

Update the existing `deserialize_minimal_config` test to assert the default: `assert_eq!(config.update_check_interval, 300);`

Update the `save_and_reload` test — add `config.update_check_interval = 120;` before save.

- [ ] **Step 4: Run config tests**

Run: `cargo test --lib config -- --nocapture`
Expected: all config tests pass (existing + 3 new)

- [ ] **Step 5: Create `UpdateCache`**

Create `src/web/update_cache.rs`:

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::forge::models::UpdatesResponseData;

#[derive(Clone)]
pub struct UpdateCache {
    inner: Arc<Mutex<Option<(Instant, UpdatesResponseData)>>>,
    ttl: Duration,
}

impl UpdateCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self) -> Option<UpdatesResponseData> {
        let guard = self.inner.lock();
        guard.as_ref().and_then(|(ts, data)| {
            if ts.elapsed() < self.ttl {
                Some(data.clone())
            } else {
                None
            }
        })
    }

    pub fn set(&self, data: UpdatesResponseData) {
        let mut guard = self.inner.lock();
        *guard = Some((Instant::now(), data));
    }

    pub fn invalidate(&self) {
        let mut guard = self.inner.lock();
        *guard = None;
    }
}
```

- [ ] **Step 6: Write tests for `UpdateCache`**

Add to the bottom of `src/web/update_cache.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::models::UpdatesResponseData;

    fn empty_response() -> UpdatesResponseData {
        UpdatesResponseData {
            spt_version: "4.0.0".to_string(),
            updates: vec![],
            blocked_updates: vec![],
            up_to_date: vec![],
            incompatible_with_spt: vec![],
        }
    }

    #[test]
    fn cache_miss_when_empty() {
        let cache = UpdateCache::new(300);
        assert!(cache.get().is_none());
    }

    #[test]
    fn cache_hit_after_set() {
        let cache = UpdateCache::new(300);
        cache.set(empty_response());
        let result = cache.get();
        assert!(result.is_some());
        assert_eq!(result.unwrap().spt_version, "4.0.0");
    }

    #[test]
    fn cache_miss_after_invalidate() {
        let cache = UpdateCache::new(300);
        cache.set(empty_response());
        cache.invalidate();
        assert!(cache.get().is_none());
    }
}
```

- [ ] **Step 7: Register module and wire into `AppState`**

Add `pub mod update_cache;` to `src/web/mod.rs` (alongside the other modules).

Update `src/web/state.rs` to add the field:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::SptInfo;
use crate::web::tasks::TaskTracker;
use crate::web::update_cache::UpdateCache;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
    pub update_cache: UpdateCache,
}
```

Update `start_server` in `src/web/mod.rs` (around line 54) to construct the cache:

```rust
let app_state = web::Data::new(AppState {
    db,
    forge,
    config: config.clone(),
    spt_dir,
    spt_info,
    tasks: crate::web::tasks::TaskTracker::new(),
    update_cache: crate::web::update_cache::UpdateCache::new(config.update_check_interval),
});
```

- [ ] **Step 8: Run all tests and build**

Run: `cargo test -- --nocapture`
Expected: all tests pass

Run: `cargo build`
Expected: compiles

- [ ] **Step 9: Commit**

```bash
git add src/config.rs src/web/update_cache.rs src/web/mod.rs src/web/state.rs
git commit -m "feat: add update check interval config and server-side update cache"
```

---

### Task 5: SSE Infrastructure (Broadcast + Endpoint + Extension)

Adds the SSE broadcast channel, integrates it with `TaskTracker`, creates the `/api/events` endpoint, and loads the HTMX SSE extension. Also updates `task_status.html` to use SSE instead of polling.

**Files:**
- Create: `src/web/sse.rs`
- Create: `src/assets/sse.js` (download from htmx-ext-sse)
- Modify: `src/web/tasks.rs:1-165` (add broadcast sender, send events on state changes)
- Modify: `src/web/mod.rs` (add module, route, construct broadcast channel)
- Modify: `src/web/state.rs` (add `events` field)
- Modify: `templates/base.html` (add SSE script + connection div)
- Modify: `templates/partials/task_status.html` (replace polling with SSE trigger)

**Interfaces:**
- Produces: `ServerEvent` enum — `TaskChanged`, `ModsChanged`
- Produces: `sse::events_stream` handler — `GET /api/events` returning `text/event-stream`
- Modifies: `TaskTracker::new(events_tx: broadcast::Sender<ServerEvent>)` — now requires broadcast sender

- [ ] **Step 1: Create `ServerEvent` and SSE handler**

Create `src/web/sse.rs`:

```rust
use actix_session::Session;
use actix_web::web::{self, Data};
use actix_web::HttpResponse;
use futures_util::stream::unfold;
use tokio::sync::broadcast;

use crate::web::auth::require_auth;
use crate::web::state::AppState;

#[derive(Clone, Debug)]
pub enum ServerEvent {
    TaskChanged,
    ModsChanged,
}

pub async fn events_stream(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;

    let rx = state.events.subscribe();

    let stream = unfold(rx, |mut rx| async move {
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

    Ok(HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .streaming(stream))
}
```

- [ ] **Step 2: Modify `TaskTracker` to send events**

In `src/web/tasks.rs`, add the broadcast import and modify the struct:

Add to the top of the file:

```rust
use tokio::sync::broadcast;
use crate::web::sse::ServerEvent;
```

Change `TrackerInner` to include the sender:

```rust
struct TrackerInner {
    tasks: HashMap<u64, TaskInfo>,
    next_id: u64,
    events_tx: broadcast::Sender<ServerEvent>,
}
```

Change `TaskTracker::new` to accept the sender:

```rust
pub fn new(events_tx: broadcast::Sender<ServerEvent>) -> Self {
    Self {
        inner: Arc::new(Mutex::new(TrackerInner {
            tasks: HashMap::new(),
            next_id: 1,
            events_tx,
        })),
    }
}
```

Add a helper to send events (ignoring errors — no receivers is fine):

```rust
fn send_event(inner: &TrackerInner, event: ServerEvent) {
    let _ = inner.events_tx.send(event);
}
```

Update `start()` — add at the end before returning `id`:
```rust
Self::send_event(&inner, ServerEvent::TaskChanged);
```

Update `complete()` — add after the status change:
```rust
Self::send_event(&inner, ServerEvent::TaskChanged);
Self::send_event(&inner, ServerEvent::ModsChanged);
```

Update `fail()` — add after the status change:
```rust
Self::send_event(&inner, ServerEvent::TaskChanged);
```

Update `update_message()` — add after the status change:
```rust
Self::send_event(&inner, ServerEvent::TaskChanged);
```

Note: the `send_event` calls go inside the lock scope (before `Self::prune_old` where applicable), since `inner` is the `MutexGuard`.

- [ ] **Step 3: Register module, add `events` to `AppState`, wire up**

Add `pub mod sse;` to `src/web/mod.rs`.

Update `src/web/state.rs` to add the events field:

```rust
use tokio::sync::broadcast;
use crate::web::sse::ServerEvent;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
    pub update_cache: UpdateCache,
    pub events: broadcast::Sender<ServerEvent>,
}
```

Update `start_server` in `src/web/mod.rs` to create the channel and pass it:

```rust
let (events_tx, _) = tokio::sync::broadcast::channel::<crate::web::sse::ServerEvent>(64);

let app_state = web::Data::new(AppState {
    db,
    forge,
    config: config.clone(),
    spt_dir,
    spt_info,
    tasks: crate::web::tasks::TaskTracker::new(events_tx.clone()),
    update_cache: crate::web::update_cache::UpdateCache::new(config.update_check_interval),
    events: events_tx,
});
```

Add the SSE route in the `/api` scope (alongside the existing routes):

```rust
.route("/events", web::get().to(crate::web::sse::events_stream))
```

- [ ] **Step 4: Download and add the HTMX SSE extension**

Run:
```bash
curl -sL https://unpkg.com/htmx-ext-sse@2.2.2/sse.js -o src/assets/sse.js
```

Verify the file was downloaded (should be ~4-8KB of JavaScript).

- [ ] **Step 5: Update `base.html`**

In `templates/base.html`, add the SSE script after the htmx script (after line 8):

```html
<script src="/assets/sse.js"></script>
```

Add the SSE connection element inside `<body>`, after `</nav>` and before `<main>` (after line 14):

```html
<div id="sse-source" hx-ext="sse" sse-connect="/api/events" style="display:none"></div>
```

- [ ] **Step 6: Update `task_status.html` to use SSE**

In `templates/partials/task_status.html`, change line 3 from:

```html
     {% if has_active %}hx-get="/api/tasks/status" hx-trigger="every 2s" hx-swap="outerHTML"{% endif %}>
```

to:

```html
     hx-get="/api/tasks/status" hx-trigger="sse:taskChanged from:#sse-source" hx-swap="outerHTML">
```

The `has_active` conditional is removed — SSE events only fire when something changes, so always listening is fine.

- [ ] **Step 7: Build and test**

Run: `cargo build`
Expected: compiles successfully

Run: `cargo test -- --nocapture`
Expected: all tests pass (note: SSE handler can't be unit-tested easily, it needs integration testing via the running app)

- [ ] **Step 8: Commit**

```bash
git add src/web/sse.rs src/web/tasks.rs src/web/mod.rs src/web/state.rs src/assets/sse.js templates/base.html templates/partials/task_status.html
git commit -m "feat: add SSE infrastructure and replace task status polling"
```

---

### Task 6: Update Status Endpoint + Version Badges + Disabled Buttons

Adds the `/api/mods/update-status` endpoint that returns OOB swaps for version badges and button states. Updates the existing `check_updates_partial` to use the cache. Updates `list.html` to have the right element IDs and trigger.

**Files:**
- Create: `templates/mods/partials/update_status.html`
- Modify: `src/web/handlers/mods.rs` (add `update_status_partial` handler, update `check_updates_partial` to use cache)
- Modify: `src/web/mod.rs` (register new route)
- Modify: `templates/mods/list.html` (add element IDs, update-status trigger)

**Interfaces:**
- Consumes: `UpdateCache::get/set` from Task 4
- Consumes: `Database::list_mods()` — existing, returns `Vec<InstalledMod>`
- Consumes: `ForgeClient::check_updates()` — existing
- Produces: `handlers::mods::update_status_partial` — handler for `GET /api/mods/update-status`

- [ ] **Step 1: Add template struct and handler for update-status**

In `src/web/handlers/mods.rs`, add the template struct (near the other template structs):

```rust
struct UpdateStatusEntry {
    db_id: i64,
    installed_version: String,
    new_version: Option<String>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/partials/update_status.html")]
struct UpdateStatusTemplate {
    entries: Vec<UpdateStatusEntry>,
}
```

Add the handler:

```rust
pub async fn update_status_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&session)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if installed.is_empty() {
        let tmpl = UpdateStatusTemplate { entries: vec![] };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let updates_data = if let Some(cached) = state.update_cache.get() {
        cached
    } else {
        let check_list: Vec<(i64, String)> = installed
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();
        match state
            .forge
            .check_updates(&check_list, &state.spt_info.spt_version)
            .await
        {
            Ok(data) => {
                state.update_cache.set(data.clone());
                data
            }
            Err(_) => {
                let entries = installed
                    .iter()
                    .map(|m| UpdateStatusEntry {
                        db_id: m.id,
                        installed_version: m.version.clone(),
                        new_version: None,
                        csrf_token: csrf_token.clone(),
                    })
                    .collect();
                let tmpl = UpdateStatusTemplate { entries };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
        }
    };

    let entries: Vec<UpdateStatusEntry> = installed
        .iter()
        .map(|m| {
            let new_version = updates_data
                .updates
                .iter()
                .find(|u| u.current_version.mod_id == m.forge_mod_id)
                .map(|u| u.recommended_version.version.clone());
            UpdateStatusEntry {
                db_id: m.id,
                installed_version: m.version.clone(),
                new_version,
                csrf_token: csrf_token.clone(),
            }
        })
        .collect();

    let tmpl = UpdateStatusTemplate { entries };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 2: Create the OOB swap template**

Create `templates/mods/partials/update_status.html`:

```html
{% for e in &entries %}
{% if let Some(new_ver) = &e.new_version %}
<td id="mod-version-{{ e.db_id }}" hx-swap-oob="true">
    {{ e.installed_version }} <span style="color: var(--success)">→ {{ new_ver }}</span>
</td>
<td id="mod-update-{{ e.db_id }}" hx-swap-oob="true">
    <form method="post" action="/mods/{{ e.db_id }}/update" style="display:inline">
        <input type="hidden" name="csrf_token" value="{{ e.csrf_token }}">
        <button type="submit" class="btn btn-sm btn-outline">{% call icons::refresh() %}{% endcall %} Update</button>
    </form>
</td>
{% else %}
<td id="mod-update-{{ e.db_id }}" hx-swap-oob="true">
    <form method="post" action="/mods/{{ e.db_id }}/update" style="display:inline">
        <input type="hidden" name="csrf_token" value="{{ e.csrf_token }}">
        <button type="submit" class="btn btn-sm btn-outline" disabled>{% call icons::refresh() %}{% endcall %} Update</button>
    </form>
</td>
{% endif %}
{% endfor %}
```

Note: This template needs the icons import. Add at the top:

```html
{% import "partials/icons.html" as icons %}
```

- [ ] **Step 3: Update `check_updates_partial` to use cache**

In `src/web/handlers/mods.rs`, modify the existing `check_updates_partial` handler (around line 151). Replace the direct Forge API call with a cache-aware version:

```rust
pub async fn check_updates_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    require_admin(&session)?;
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let updates_available = if !installed.is_empty() {
        if let Some(cached) = state.update_cache.get() {
            cached.updates.len()
        } else {
            let check_list: Vec<(i64, String)> = installed
                .iter()
                .map(|m| (m.forge_mod_id, m.version.clone()))
                .collect();
            match state
                .forge
                .check_updates(&check_list, &state.spt_info.spt_version)
                .await
            {
                Ok(data) => {
                    let count = data.updates.len();
                    state.update_cache.set(data);
                    count
                }
                Err(_) => 0,
            }
        }
    } else {
        0
    };

    let tmpl = UpdateBadgesTemplate { updates_available };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 4: Register the route**

In `src/web/mod.rs`, add to the `/api` scope:

```rust
.route(
    "/mods/update-status",
    web::get().to(handlers::mods::update_status_partial),
)
```

- [ ] **Step 5: Update `list.html` with element IDs and trigger**

In `templates/mods/list.html`, update the version cell (around line 44) to add an ID:

```html
<td id="mod-version-{{ m.mod_info.id }}">{{ m.mod_info.version }}</td>
```

Update the Actions `<td>` for the update button to add an ID (around line 48). Wrap the update form in an identified cell:

Change:
```html
<td>
    <form method="post" action="/mods/{{ m.mod_info.id }}/update" style="display:inline">
```

To:
```html
<td id="mod-update-{{ m.mod_info.id }}">
    <form method="post" action="/mods/{{ m.mod_info.id }}/update" style="display:inline">
```

But we need to separate the Update and Remove into their own cells, or keep the Remove in the same cell without the ID colliding. Since OOB replaces the entire element, and Remove is in the same `<td>`, we need to split the Actions column into two cells — or put only the Update form in the OOB-targeted cell and leave Remove separate.

Simpler approach: put the Update button `<form>` and Remove button `<form>` in the same `<td>` but wrap the Update form in a `<span>` with the ID:

```html
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
```

And update the OOB template to use `<span>` instead of `<td>`:

Update `templates/mods/partials/update_status.html` — change all `<td id="mod-update-...">` to `<span id="mod-update-...">` and `</td>` to `</span>`.

Add the trigger element just before the closing `</div>` of the card (after `</table>`, before `</div>`):

```html
<span hx-get="/api/mods/update-status" hx-trigger="load, sse:modsChanged from:#sse-source" hx-swap="none"></span>
```

- [ ] **Step 6: Build and test**

Run: `cargo build`
Expected: compiles (verifies all templates, OOB partial, handler alignment)

Run: `cargo test -- --nocapture`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/web/handlers/mods.rs src/web/mod.rs templates/mods/partials/update_status.html templates/mods/list.html
git commit -m "feat: add cached update-status endpoint with version badges and disabled buttons"
```

---

### Task 7: Mod List Auto-Refresh via SSE + Cache Invalidation

Adds the `/api/mods/list` partial endpoint for SSE-triggered table refresh and wires cache invalidation into the install/update/remove completion paths.

**Files:**
- Create: `templates/mods/partials/list_body.html`
- Modify: `src/web/handlers/mods.rs` (add `list_body_partial` handler, add cache invalidation to install/update/remove)
- Modify: `src/web/mod.rs` (register route)
- Modify: `templates/mods/list.html` (add SSE trigger on tbody)

**Interfaces:**
- Consumes: `Database::list_mods_with_file_counts` from Task 2
- Consumes: `UpdateCache::invalidate` from Task 4
- Consumes: SSE `modsChanged` event from Task 5
- Produces: `handlers::mods::list_body_partial` — handler for `GET /api/mods/list`

- [ ] **Step 1: Create the list body partial template**

Create `templates/mods/partials/list_body.html`. This renders just the `<tr>` rows and `<tfoot>` that go inside the `<tbody>`:

```html
{% import "partials/icons.html" as icons %}
{% for m in &mods %}
<tr>
    <td><a href="/mods/{{ m.mod_info.id }}">{{ m.mod_info.name }}</a></td>
    <td id="mod-version-{{ m.mod_info.id }}">{{ m.mod_info.version }}</td>
    <td class="text-muted">{{ m.file_count }}</td>
    <td class="text-muted">{{ m.total_size|format_size_i64 }}</td>
    <td class="text-muted text-sm">{{ m.mod_info.installed_at }}</td>
    {% if user.is_admin() %}
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
    <td colspan="{% if user.is_admin() %}6{% else %}5{% endif %}" class="text-muted text-sm">
        Total: {{ mods.len() }} mods — {{ grand_total_size|format_size_i64 }}
    </td>
</tr>
```

- [ ] **Step 2: Add the handler and template struct**

In `src/web/handlers/mods.rs`, add the template struct:

```rust
#[derive(Template)]
#[template(path = "mods/partials/list_body.html")]
struct ListBodyTemplate {
    user: SessionUser,
    mods: Vec<ModListEntry>,
    grand_total_size: i64,
    csrf_token: String,
}
```

Add the handler:

```rust
pub async fn list_body_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();

    let mods = web::block(move || {
        let db = db.lock();
        let mods_with_counts = db.list_mods_with_file_counts()?;
        let entries: Vec<ModListEntry> = mods_with_counts
            .into_iter()
            .map(|(mod_info, file_count, total_size)| ModListEntry {
                mod_info,
                file_count,
                total_size,
            })
            .collect();
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let grand_total_size: i64 = mods.iter().map(|m| m.total_size).sum();

    let tmpl = ListBodyTemplate {
        user,
        mods,
        grand_total_size,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 3: Register the route**

In `src/web/mod.rs`, add to the `/api` scope:

```rust
.route(
    "/mods/list",
    web::get().to(handlers::mods::list_body_partial),
)
```

- [ ] **Step 4: Update `list.html` to use SSE-triggered tbody**

In `templates/mods/list.html`, change the `<tbody>` tag (around line 40) to:

```html
<tbody id="mod-list-body"
       hx-get="/api/mods/list"
       hx-trigger="sse:modsChanged from:#sse-source"
       hx-swap="innerHTML">
```

Move the `<tfoot>` content out of the main template into `list_body.html` (done in Step 1). The `<tfoot>` in `list.html` is removed — it's now part of the partial's response. The partial renders `<tr>` rows directly inside the `<tbody>`.

Note: The footer row uses `class="tfoot-row"` instead of a `<tfoot>` element because `<tfoot>` can't be nested inside `<tbody>`. Add a minimal CSS rule in `src/assets/style.css`:

```css
.tfoot-row td {
    border-top: 1px solid var(--border);
    font-style: italic;
}
```

- [ ] **Step 5: Add cache invalidation to install/update/remove handlers**

In the `install_mod` handler's `tokio::spawn` block (around line 392), add cache invalidation when the task completes successfully:

```rust
Ok(()) => {
    tracing::info!(mod_id, "mod installed successfully");
    tasks.complete(task_id, "Mod installed successfully".to_string());
    update_cache.invalidate();
}
```

To make `update_cache` accessible inside the spawned task, clone it before the `tokio::spawn` (cheap — `inner` is behind `Arc`).

Clone `update_cache` before the spawn in `install_mod`, `update_mod`, and `update_all_mods`:

For `install_mod` (before `tokio::spawn` around line 341):
```rust
let update_cache = state.update_cache.clone();
```

Then in the success branch:
```rust
Ok(()) => {
    tracing::info!(mod_id, "mod installed successfully");
    tasks.complete(task_id, "Mod installed successfully".to_string());
    update_cache.invalidate();
}
```

Do the same for `update_mod` (clone before spawn around line 493, invalidate in success around line 551).

For `update_all_mods` (clone before spawn around line 714, invalidate after the loop completes, before the final task status update around line 796):
```rust
update_cache.invalidate();
```

For `remove_mod` — this one doesn't use `tokio::spawn`, it runs synchronously. Add invalidation after the successful removal (around line 623, before `set_flash`):
```rust
state.update_cache.invalidate();
```

- [ ] **Step 6: Build and test**

Run: `cargo build`
Expected: compiles

Run: `cargo test -- --nocapture`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/web/update_cache.rs src/web/handlers/mods.rs src/web/mod.rs templates/mods/list.html templates/mods/partials/list_body.html src/assets/style.css
git commit -m "feat: add SSE-driven mod list auto-refresh and cache invalidation"
```

---

### Task 8: Manual Integration Test

Verify all features work together in the running application.

**Files:** None (testing only)

- [ ] **Step 1: Start the application**

Run: `cargo run -- serve` (or however the dev server is started — check `src/main.rs` or `src/cli/` for the serve command)

Navigate to the web UI in a browser.

- [ ] **Step 2: Verify file size formatting**

Go to `/mods/{id}` for any installed mod. Verify:
- File sizes in the archive files table show human-readable units (e.g., `1.2 MiB`, `512 B`)
- File sizes in the runtime files table also show human-readable units
- Files with no size data show `-`

- [ ] **Step 3: Verify mod list page**

Go to `/mods`. Verify:
- "Size" column exists between "Files" and "Installed"
- Each mod row shows a human-readable total size
- Footer row shows total mod count and combined size
- Update buttons are disabled for mods that are up-to-date
- Mods with updates show `1.2.0 → 1.3.0` style version badges in green

- [ ] **Step 4: Verify mod detail page**

Go to `/mods/{id}`. Verify:
- Update and Remove buttons are above the files tables (not below)
- "Forge" row exists in the metadata table with a working link to `forge.sp-tarkov.com`
- Clicking the Forge link opens in a new tab

- [ ] **Step 5: Verify SSE live updates**

Open the browser dev tools Network tab and verify:
- An SSE connection is established to `/api/events` (EventStream type)
- Install a mod (or trigger an update) and verify:
  - The task status banner updates without polling (check no `every 2s` requests)
  - The mod list table refreshes automatically when the install completes
  - The version badges re-fetch after the mod list updates

- [ ] **Step 6: Verify update cache**

Check server logs or add temporary debug logging:
- First visit to `/mods` triggers a Forge API call for update check
- Subsequent page loads within the TTL (default 5 minutes) use cached data
- After installing/updating/removing a mod, the cache is invalidated and the next check hits the API

- [ ] **Step 7: Clean up and final commit (if any fixes were needed)**

If any issues were found and fixed during testing, commit the fixes:

```bash
git add -A
git commit -m "fix: integration test fixes for mod UI features"
```
