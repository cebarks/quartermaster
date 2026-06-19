# Web UI Bugfixes & Improvements Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix six issues found during manual testing: unstyled error pages, missing logging, no Fika compatibility warning in web install, blocking mod operations, slow status page, and broken server controls.

**Architecture:** Error pages get a proper HTML template. Structured logging via `tracing` replaces `println!/eprintln!`. Mod install/update operations become async background jobs with HTMX polling for progress. Status page sections load as independent HTMX partials in parallel. Server control errors surface via flash messages instead of raw error responses. Fika compatibility warnings are added to the install flow when Fika is detected as installed.

**Tech Stack:** Rust, actix-web 4, askama templates, HTMX, tracing + tracing-subscriber + tracing-actix-web

## Global Constraints

- All web error responses must render using the base HTML template (not plain text)
- All `println!`/`eprintln!` calls in `src/web/` must be replaced with `tracing` macros
- No new JavaScript frameworks — use HTMX for async UI updates
- Existing tests must continue to pass
- Follow existing code patterns (askama templates, `web::block` for DB, `WebError` for handler errors)
- All POST endpoints must validate CSRF tokens (per project security policy, commit `46640a6`)
- Status page partials must preserve the existing visual design from `templates/partials/status_detail.html` (hero-card, status-grid, colored borders, badge styling, table layouts)

---

### Task 1: Add structured logging with `tracing`

Currently the app uses `println!`/`eprintln!` with no log levels, timestamps, or filtering. Add `tracing` as the logging framework.

**Files:**
- Modify: `Cargo.toml` (add dependencies)
- Modify: `src/main.rs` (init subscriber)
- Modify: `src/web/mod.rs` (replace println, add request logging middleware)
- Modify: `src/web/error.rs` (replace eprintln with tracing::error)
- Modify: `src/web/csrf.rs` (replace eprintln with tracing::warn)
- Modify: `src/web/handlers/queue.rs` (replace eprintln with tracing::error)

**Interfaces:**
- Produces: tracing subscriber initialized in main, all web code uses tracing macros. Later tasks (especially Task 4) depend on tracing being available.

- [ ] **Step 1: Add tracing dependencies to Cargo.toml**

Add these to `[dependencies]`:
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-actix-web = "0.7"
```

- [ ] **Step 2: Initialize tracing subscriber in main.rs**

Add at the top of `main()`, before CLI parsing:

```rust
use tracing_subscriber::EnvFilter;

tracing_subscriber::fmt()
    .with_env_filter(
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,quartermaster=debug")),
    )
    .init();
```

- [ ] **Step 3: Add request logging middleware to the actix app**

In `src/web/mod.rs`, add `tracing_actix_web::TracingLogger` as a middleware wrap on the App (after `NormalizePath`):

```rust
.wrap(tracing_actix_web::TracingLogger::default())
```

Replace the `println!` on server start with:

```rust
tracing::info!("Quartermaster web UI starting on http://{bind_addr}");
```

- [ ] **Step 4: Replace all eprintln!/println! in web code with tracing macros**

In `src/web/error.rs`:
```rust
// Before:
eprintln!("internal error: {e:#}");
// After:
tracing::error!(error = %e, "internal server error");
```

In `src/web/csrf.rs`:
```rust
// Before:
eprintln!("failed to insert CSRF token into session: {e}");
// After:
tracing::warn!(error = %e, "failed to insert CSRF token into session");
```

In `src/web/handlers/queue.rs`:
```rust
// Before:
eprintln!("queue apply failed for {} '{}': {e}", op.action, op.mod_name);
// After:
tracing::error!(action = %op.action, mod_name = %op.mod_name, error = %e, "queue apply failed");
```

- [ ] **Step 5: Add logging to key web handlers**

Add `tracing::info!` calls to mod install, update, remove, and server control handlers at the point of action (not on every request — just the mutating ones). Examples:

In `src/web/handlers/mods.rs` `install_mod`:
```rust
tracing::info!(mod_id, mod_name = %mod_info.name, version = %version.version, "installing mod");
```

In `src/web/handlers/server.rs` `start_server`:
```rust
tracing::info!(container, "starting server");
```

- [ ] **Step 6: Build and verify**

Run: `cargo build`
Run: `cargo test`

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/web/mod.rs src/web/error.rs src/web/csrf.rs src/web/handlers/queue.rs src/web/handlers/mods.rs src/web/handlers/server.rs
git commit -m "feat: add structured logging with tracing"
```

---

### Task 2: Style error pages with HTML template

Error responses currently return plain text strings. Add an error template that extends `base.html` so errors get nav, CSS, and a styled error message. The error template includes a minimal nav bar (just the brand link) since `error_response()` has no access to the user session.

**Files:**
- Create: `templates/error.html`
- Modify: `src/web/error.rs` (render error template instead of plain text)

**Interfaces:**
- Consumes: `templates/base.html` (existing)
- Produces: All `WebError` responses now render styled HTML

- [ ] **Step 1: Create the error template**

Create `templates/error.html`. Note: error pages cannot show the full nav bar because `error_response()` has no access to the user session. The base template's `{% block nav %}` defaults to empty, so error pages show just the "Quartermaster" brand text — which links home — plus a "Back to Dashboard" button in the body.

```html
{% extends "base.html" %}
{% block title %}{{ title }} — Quartermaster{% endblock %}
{% block content %}
<div class="card">
    <h1>{{ title }}</h1>
    <p>{{ message }}</p>
    <a href="/" class="btn">Back to Dashboard</a>
</div>
{% endblock %}
```

- [ ] **Step 2: Update WebError to render the template**

In `src/web/error.rs`, add the template struct and update `error_response()`:

```rust
use askama::Template;

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
    title: String,
    message: String,
}

impl ResponseError for WebError {
    fn status_code(&self) -> StatusCode {
        match self {
            WebError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebError::NotFound => StatusCode::NOT_FOUND,
            WebError::Forbidden => StatusCode::FORBIDDEN,
            WebError::BadRequest(_) => StatusCode::BAD_REQUEST,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let WebError::Internal(e) = self {
            tracing::error!(error = %e, "internal server error");
        }

        let (title, message) = match self {
            WebError::Internal(_) => (
                "Internal Server Error".to_string(),
                "An unexpected error occurred. Please try again.".to_string(),
            ),
            WebError::NotFound => (
                "Not Found".to_string(),
                "The page you're looking for doesn't exist.".to_string(),
            ),
            WebError::Forbidden => (
                "Forbidden".to_string(),
                "You don't have permission to access this page.".to_string(),
            ),
            WebError::BadRequest(msg) => (
                "Bad Request".to_string(),
                msg.clone(),
            ),
        };

        let tmpl = ErrorTemplate { title: title.clone(), message };
        match tmpl.render() {
            Ok(body) => HttpResponse::build(self.status_code())
                .content_type("text/html")
                .body(body),
            Err(_) => HttpResponse::build(self.status_code())
                .body(title),
        }
    }
}
```

- [ ] **Step 3: Build and manually test**

Run: `cargo build`

Test by triggering a BadRequest error — e.g. POST to `/mods/install` with an empty mod_ref (after logging in). Verify the error page has the brand nav, CSS, and the error message.

- [ ] **Step 4: Commit**

```bash
git add templates/error.html src/web/error.rs
git commit -m "fix: render error pages with styled HTML template"
```

---

### Task 3: Add Fika compatibility warning to web mod install

The CLI already has `check_fika_compat()` in `src/cli/install.rs`. The web install handler has no equivalent — it installs Fika-incompatible mods silently. Add a warning when Fika is installed and the target mod is marked incompatible or unknown.

**Files:**
- Modify: `src/web/handlers/mods.rs` (add Fika check after version lookup, show flash warning)

**Interfaces:**
- Consumes: `ForgeVersion.fika_compatibility` field, `db.get_mod_by_forge_id()` to check if Fika is installed (mod ID 2326)
- Produces: Flash warning on install when mod is Fika-incompatible

- [ ] **Step 1: Add Fika compatibility check to `install_mod` handler**

In `src/web/handlers/mods.rs`, after the version lookup (after the `get_versions` and `first()` calls, before the `should_queue` check), add a Fika compatibility warning. The check should only fire when Fika is installed (forge_mod_id 2326 in the DB):

```rust
const FIKA_FORGE_MOD_ID: i64 = 2326;

// Check Fika compatibility if Fika is installed
{
    let db = state.db.clone();
    let fika_installed = web::block(move || {
        let db = db.lock();
        Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(FIKA_FORGE_MOD_ID)?.is_some())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if fika_installed {
        use crate::forge::models::FikaCompat;
        match &version.fika_compatibility {
            Some(FikaCompat::Incompatible) => {
                set_flash(
                    &session,
                    &format!(
                        "Warning: {} v{} is marked as Fika INCOMPATIBLE. It may cause issues with multiplayer.",
                        mod_info.name, version.version
                    ),
                    "warning",
                );
            }
            Some(FikaCompat::Unknown) => {
                set_flash(
                    &session,
                    &format!(
                        "Note: Fika compatibility for {} v{} is unknown.",
                        mod_info.name, version.version
                    ),
                    "warning",
                );
            }
            _ => {}
        }
    }
}
```

Note: Unlike the CLI which blocks and prompts for confirmation, the web version shows a warning flash but proceeds with installation. This is deliberate — the web UI has no interactive confirmation flow and the mod is still installable.

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Run: `cargo test`

- [ ] **Step 3: Commit**

```bash
git add src/web/handlers/mods.rs
git commit -m "feat: warn about Fika incompatibility during web mod install"
```

---

### Task 4: Make mod install/update operations non-blocking with progress feedback

This is the biggest change. Currently `install_mod`, `update_mod`, and `update_all_mods` handlers download and extract archives synchronously in the HTTP request, blocking the entire server. Convert these to background operations with HTMX progress polling.

**Architecture:** When a mod operation is triggered, it spawns a `tokio::spawn` task and immediately redirects to the mods page. A new `TaskTracker` in AppState tracks in-flight operations. The mods page shows a progress banner for active operations via HTMX polling. When the task completes, the banner updates with success/failure.

**Prerequisites:**
- Task 1 must be completed first (this task uses `tracing` macros in the spawned tasks)
- `ForgeClient` must be made `Clone` (it currently has no `#[derive(Clone)]` — add it; `reqwest::Client` internally uses `Arc` so cloning is cheap)

**Files:**
- Modify: `src/forge/client.rs` (add `#[derive(Clone)]` to `ForgeClient`)
- Create: `src/web/tasks.rs` (background task tracker)
- Modify: `src/web/state.rs` (add TaskTracker to AppState)
- Modify: `src/web/mod.rs` (register task status endpoint, add `mod tasks`)
- Modify: `src/web/handlers/mod.rs` (add `pub mod tasks;`)
- Create: `src/web/handlers/tasks.rs` (task status + dismiss handlers)
- Modify: `src/web/handlers/mods.rs` (spawn operations as background tasks)
- Modify: `src/assets/style.css` (add `.alert-info` CSS class)
- Create: `templates/partials/task_status.html` (HTMX polling partial)
- Modify: `templates/mods/list.html` (add task status banner)

**Interfaces:**
- Consumes: AppState, ForgeClient (must be Clone), Database (all shared via Arc)
- Produces: `GET /api/tasks/status` endpoint for HTMX polling, `POST /api/tasks/{id}/dismiss` for dismissal, `TaskTracker` struct in AppState

- [ ] **Step 1: Add `#[derive(Clone)]` to `ForgeClient`**

In `src/forge/client.rs`, add `Clone` to the derives on `ForgeClient`:

```rust
#[derive(Clone)]
pub struct ForgeClient {
    // ...
}
```

`reqwest::Client` uses `Arc` internally, so cloning is cheap (just an Arc bump).

- [ ] **Step 2: Add `.alert-info` CSS class to stylesheet**

In `src/assets/style.css`, add after the existing alert classes (line ~129):

```css
.alert-info { background: rgba(100,149,237,0.15); border: 1px solid var(--accent); color: var(--accent); }
```

- [ ] **Step 3: Create the task tracker module**

Create `src/web/tasks.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Running { message: String },
    Completed { message: String },
    Failed { message: String },
}

impl TaskStatus {
    pub fn css_class(&self) -> &'static str {
        match self {
            TaskStatus::Running { .. } => "alert-info loading-pulse",
            TaskStatus::Completed { .. } => "alert-success",
            TaskStatus::Failed { .. } => "alert-error",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            TaskStatus::Running { message }
            | TaskStatus::Completed { message }
            | TaskStatus::Failed { message } => message,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, TaskStatus::Running { .. })
    }
}

#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub status: TaskStatus,
    pub mod_name: String,
    pub action: String,
    pub forge_mod_id: i64,
}

/// View-model for templates — pre-computed CSS class and message to avoid
/// askama match blocks with full crate paths.
pub struct TaskView {
    pub id: u64,
    pub css_class: String,
    pub message: String,
    pub is_running: bool,
}

struct TrackerInner {
    tasks: HashMap<u64, TaskInfo>,
    next_id: u64,
}

#[derive(Clone)]
pub struct TaskTracker {
    inner: Arc<Mutex<TrackerInner>>,
}

impl TaskTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TrackerInner {
                tasks: HashMap::new(),
                next_id: 1,
            })),
        }
    }

    pub fn has_running_for_mod(&self, forge_mod_id: i64) -> bool {
        let inner = self.inner.lock();
        inner.tasks.values().any(|t| {
            t.forge_mod_id == forge_mod_id && t.status.is_running()
        })
    }

    pub fn start(&self, action: &str, mod_name: &str, forge_mod_id: i64) -> u64 {
        let mut inner = self.inner.lock();
        let id = inner.next_id;
        inner.next_id += 1;

        let info = TaskInfo {
            status: TaskStatus::Running {
                message: format!("{}ing {}...", action, mod_name),
            },
            mod_name: mod_name.to_string(),
            action: action.to_string(),
            forge_mod_id,
        };
        inner.tasks.insert(id, info);
        id
    }

    pub fn complete(&self, id: u64, message: String) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(&id) {
            task.status = TaskStatus::Completed { message };
        }
        Self::prune_old(&mut inner);
    }

    pub fn fail(&self, id: u64, message: String) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(&id) {
            task.status = TaskStatus::Failed { message };
        }
        Self::prune_old(&mut inner);
    }

    pub fn task_views(&self) -> Vec<TaskView> {
        let inner = self.inner.lock();
        inner
            .tasks
            .iter()
            .map(|(id, info)| TaskView {
                id: *id,
                css_class: info.status.css_class().to_string(),
                message: info.status.message().to_string(),
                is_running: info.status.is_running(),
            })
            .collect()
    }

    pub fn dismiss(&self, id: u64) {
        self.inner.lock().tasks.remove(&id);
    }

    pub fn has_active(&self) -> bool {
        self.inner.lock().tasks.values().any(|t| t.status.is_running())
    }

    /// Remove completed/failed tasks that exceed a cap (keep at most 20 finished tasks).
    fn prune_old(inner: &mut TrackerInner) {
        let finished: Vec<u64> = inner
            .tasks
            .iter()
            .filter(|(_, t)| !t.status.is_running())
            .map(|(id, _)| *id)
            .collect();
        if finished.len() > 20 {
            let mut to_remove: Vec<u64> = finished;
            to_remove.sort();
            for id in &to_remove[..to_remove.len() - 20] {
                inner.tasks.remove(id);
            }
        }
    }
}
```

Key design decisions vs. the original:
- **Single Mutex** instead of two (simpler, no lock-ordering risk)
- **`has_running_for_mod()`** prevents duplicate installs of the same mod
- **`forge_mod_id`** tracked per task for the duplicate guard
- **`TaskView`** pre-computes CSS class and message strings for templates (avoids fragile askama `{% match %}` blocks with full crate paths)
- **`prune_old()`** auto-removes finished tasks when they exceed 20 (prevents unbounded memory growth)

- [ ] **Step 4: Add TaskTracker to AppState**

In `src/web/state.rs`:

```rust
use crate::web::tasks::TaskTracker;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
}
```

Update `src/web/mod.rs` to add `pub mod tasks;` and initialize the tracker in `start_server`:

```rust
let app_state = web::Data::new(AppState {
    db,
    forge,
    config: config.clone(),
    spt_dir,
    spt_info,
    tasks: crate::web::tasks::TaskTracker::new(),
});
```

- [ ] **Step 5: Add task status API endpoint**

Create `src/web/handlers/tasks.rs`:

```rust
use actix_session::Session;
use actix_web::web::{Data, Form, Html, Path};
use actix_web::HttpResponse;
use askama::Template;

use crate::web::auth::require_auth;
use crate::web::error::WebError;
use crate::web::state::AppState;
use crate::web::tasks::TaskView;

#[derive(Template)]
#[template(path = "partials/task_status.html")]
struct TaskStatusTemplate {
    tasks: Vec<TaskView>,
    has_active: bool,
}

pub async fn task_status_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&session)?;
    let tasks = state.tasks.task_views();
    let has_active = state.tasks.has_active();
    let tmpl = TaskStatusTemplate { tasks, has_active };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dismiss_task(
    state: Data<AppState>,
    path: Path<u64>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    state.tasks.dismiss(path.into_inner());
    Ok(HttpResponse::Ok().body(""))
}
```

Register in `src/web/mod.rs` — add the routes to the `/api` scope:

```rust
.route("/tasks/status", web::get().to(handlers::tasks::task_status_partial))
.route("/tasks/{id}/dismiss", web::post().to(handlers::tasks::dismiss_task))
```

Add `pub mod tasks;` to `src/web/handlers/mod.rs`.

- [ ] **Step 6: Create task status HTMX partial template**

Create `templates/partials/task_status.html`. Uses pre-computed `TaskView` fields instead of match blocks:

```html
{% if !tasks.is_empty() %}
<div id="task-status"
     {% if has_active %}hx-get="/api/tasks/status" hx-trigger="every 2s" hx-swap="outerHTML"{% endif %}>
    {% for task in &tasks %}
    <div class="alert {{ task.css_class }}">
        <span>{{ task.message }}</span>
        {% if !task.is_running %}
            <form method="post" action="/api/tasks/{{ task.id }}/dismiss" style="display:inline"
                  hx-post="/api/tasks/{{ task.id }}/dismiss" hx-target="closest .alert" hx-swap="delete">
                <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
                <button type="submit" class="btn btn-sm">Dismiss</button>
            </form>
        {% endif %}
    </div>
    {% endfor %}
</div>
{% else %}
<div id="task-status"></div>
{% endif %}
```

Note: The dismiss button uses a form with CSRF token (consistent with project security policy). The `hx-post` attribute provides the HTMX progressive enhancement, while the form provides a fallback.

- [ ] **Step 7: Update the template struct to include csrf_token**

The `TaskStatusTemplate` needs a `csrf_token` field for the dismiss form. Update the struct and handler:

```rust
#[derive(Template)]
#[template(path = "partials/task_status.html")]
struct TaskStatusTemplate {
    tasks: Vec<TaskView>,
    has_active: bool,
    csrf_token: String,
}

pub async fn task_status_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&session)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let tasks = state.tasks.task_views();
    let has_active = state.tasks.has_active();
    let tmpl = TaskStatusTemplate { tasks, has_active, csrf_token };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 8: Add task status banner to mods list page**

In `templates/mods/list.html`, add right after the opening `{% block content %}` (before the `<div class="flex-between">` heading):

```html
<div hx-get="/api/tasks/status" hx-trigger="load" hx-swap="outerHTML" id="task-status"></div>
```

- [ ] **Step 9: Convert `install_mod` handler to background task**

In `src/web/handlers/mods.rs`, replace the synchronous download+extract block in `install_mod` (everything after the `should_queue` check's queue path, through to the end of the function) with a spawned task:

```rust
// ... (after should_queue check and queue path) ...

// Prevent duplicate installs
if state.tasks.has_running_for_mod(mod_id) {
    set_flash(&session, "This mod is already being installed", "warning");
    return Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish());
}

let task_id = state.tasks.start("Install", &mod_info.name, mod_id);
let tasks = state.tasks.clone();
let forge = state.forge.clone();
let spt_dir = state.spt_dir.clone();
let db = state.db.clone();
let version = version.clone();
let mod_name = mod_info.name.clone();
let mod_slug = mod_info.slug.clone();

tokio::spawn(async move {
    let result = async {
        let link = version.link.as_deref()
            .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
        let tmp_dir = tempfile::tempdir()?;
        let archive_path = tmp_dir.path().join("mod.zip");
        forge.download_file(link, &archive_path).await?;

        let version_id = version.id;
        let version_str = version.version.clone();
        actix_web::web::block(move || {
            let db = db.lock();
            crate::ops::install_mod_from_archive(
                &db, &spt_dir, mod_id, version_id,
                &mod_name, mod_slug.as_deref(), &version_str, &archive_path,
            )
        }).await??;
        Ok::<_, anyhow::Error>(())
    }.await;

    match result {
        Ok(()) => {
            tracing::info!(mod_id, "mod installed successfully");
            tasks.complete(task_id, "Mod installed successfully".to_string());
        }
        Err(e) => {
            tracing::error!(mod_id, error = %e, "mod install failed");
            tasks.fail(task_id, format!("Install failed: {e}"));
        }
    }
});

Ok(HttpResponse::SeeOther()
    .insert_header(("Location", "/mods"))
    .finish())
```

Note: The handler no longer sets flash messages for success/failure. The `TaskTracker` + HTMX polling replaces flash as the feedback mechanism for async operations. The Fika warning flash from Task 3 still works because it's set *before* the async spawn.

- [ ] **Step 10: Convert `update_mod` handler to background task**

Apply the same pattern as install_mod — check for duplicate with `has_running_for_mod`, spawn a tokio task, track via TaskTracker, redirect immediately. The spawned task downloads, extracts, and updates the mod via `crate::ops::update_mod_from_archive`.

- [ ] **Step 11: Convert `update_all_mods` handler to background task**

Apply the same pattern. The spawned task iterates over all updates, downloading and applying each one. Use a forge_mod_id of `0` for the duplicate guard (or skip the guard since this is a bulk operation). Track with a single task entry that updates its message as each mod completes:

```rust
tasks.inner.lock().tasks.get_mut(&task_id).unwrap().status = TaskStatus::Running {
    message: format!("Updating mod {} of {}...", i + 1, total),
};
```

Actually, since `inner` is private, expose a method for this:

```rust
// Add to TaskTracker:
pub fn update_message(&self, id: u64, message: String) {
    let mut inner = self.inner.lock();
    if let Some(task) = inner.tasks.get_mut(&id) {
        if task.status.is_running() {
            task.status = TaskStatus::Running { message };
        }
    }
}
```

- [ ] **Step 12: Build and test**

Run: `cargo build`
Run: `cargo test`

Manually test: install a mod via the web UI, verify the page redirects immediately and the progress banner appears, then shows success. Try double-clicking install and verify the duplicate guard works.

- [ ] **Step 13: Commit**

```bash
git add src/forge/client.rs src/assets/style.css src/web/tasks.rs src/web/state.rs src/web/mod.rs src/web/handlers/tasks.rs src/web/handlers/mod.rs src/web/handlers/mods.rs templates/partials/task_status.html templates/mods/list.html
git commit -m "feat: async mod install/update with HTMX progress tracking"
```

---

### Task 5: Make status page load incrementally

The status page blocks on `build_health_report()` which sequentially runs: server ping (up to 10s timeout), loaded mods query, update check against Forge API, and file integrity hashing. Convert to parallel HTMX partials so each section loads independently.

**Important:** The new partials must preserve the existing visual design from `templates/partials/status_detail.html` — hero-card for server status, status-grid layout, colored card borders, badge styling, and table layouts.

**Files:**
- Modify: `src/web/handlers/status.rs` (split into per-section partials)
- Modify: `src/web/mod.rs` (add new partial routes)
- Modify: `templates/status.html` (use HTMX to load each section)
- Create: `templates/partials/status_server.html`
- Create: `templates/partials/status_mods.html`
- Create: `templates/partials/status_integrity.html`
- Remove: `templates/partials/status_detail.html` (replaced by the three partials)

**Interfaces:**
- Consumes: `health::check_server`, `health::check_mods_health`, `health::check_integrity_from`
- Produces: `GET /api/status/server`, `GET /api/status/mods`, `GET /api/status/integrity` endpoints

- [ ] **Step 1: Create per-section partial templates preserving existing design**

Create `templates/partials/status_server.html` — matches the hero-card design from `status_detail.html`:
```html
<div class="hero-card {% if report.reachable %}up{% else %}down{% endif %}">
    <div class="hero-status">
        {% if report.reachable %}
        <span class="status-dot up glow"></span> Running
        {% if let Some(ms) = report.latency_ms %}<span class="text-muted" style="font-size: 0.85rem; font-weight: 400;"> — {{ ms }}ms</span>{% endif %}
        {% else %}
        <span class="status-dot down"></span> Down
        {% if let Some(err) = &report.error %}<span class="text-muted" style="font-size: 0.85rem; font-weight: 400;"> — {{ err }}</span>{% endif %}
        {% endif %}
    </div>
    <div class="hero-meta">
        {{ report.address }}
        {% if let Some(v) = &report.version %}
         · v{{ v }}
            {% if let Some(matches) = report.version_matches %}
                {% if matches %}<span class="badge badge-success">matches</span>
                {% else %}<span class="badge badge-danger">mismatch</span>
                {% endif %}
            {% endif %}
        {% endif %}
    </div>
</div>
```

Create `templates/partials/status_mods.html` — matches the card + table design, including `untracked_loaded`:
```html
<div class="card" style="border-left: 3px solid {% if report.updates_available > 0 %}var(--warning){% else %}var(--success){% endif %}">
    <h2>Mods</h2>
    <table>
        <tr><th style="width:140px">Installed</th><td><strong>{{ report.installed_count }}</strong></td></tr>
        {% if let Some(loaded) = report.loaded_count %}
        <tr><th>Loaded</th><td>{{ loaded }}</td></tr>
        {% endif %}
        {% if !report.load_failures.is_empty() %}
        <tr><th>Load Failures</th><td>
            {% for name in &report.load_failures %}
            <span class="badge badge-danger">{{ name }}</span>
            {% endfor %}
        </td></tr>
        {% endif %}
        {% if !report.untracked_loaded.is_empty() %}
        <tr><th>Untracked</th><td>
            {% for name in &report.untracked_loaded %}
            <span class="badge badge-warning">{{ name }}</span>
            {% endfor %}
        </td></tr>
        {% endif %}
        <tr><th>Updates</th><td>
            {% if report.updates_available > 0 %}
            <span class="badge badge-warning">{{ report.updates_available }} available</span>
            {% else %}
            <span class="text-muted">All up to date</span>
            {% endif %}
        </td></tr>
        {% if !report.incompatible_mods.is_empty() %}
        <tr><th>Incompatible</th><td>
            {% for name in &report.incompatible_mods %}
            <span class="badge badge-danger">{{ name }}</span>
            {% endfor %}
        </td></tr>
        {% endif %}
    </table>
</div>
```

Create `templates/partials/status_integrity.html` — matches the card + table design:
```html
<div class="card" style="border-left: 3px solid {% if report.missing_files.is_empty() && report.modified_files.is_empty() %}var(--success){% else %}var(--danger){% endif %}">
    <h2>Integrity</h2>
    {% if report.missing_files.is_empty() && report.modified_files.is_empty() && report.untracked_dirs.is_empty() %}
    <p class="text-muted">{{ report.tracked_files }} tracked files — all present, hashes match.</p>
    {% else %}
    <table>
        <tr><th style="width:140px">Tracked</th><td>{{ report.tracked_files }} files</td></tr>
        {% if !report.missing_files.is_empty() %}
        <tr><th>Missing</th><td><span class="badge badge-danger">{{ report.missing_files.len() }}</span></td></tr>
        {% endif %}
        {% if !report.modified_files.is_empty() %}
        <tr><th>Modified</th><td><span class="badge badge-warning">{{ report.modified_files.len() }}</span></td></tr>
        {% endif %}
        {% if !report.untracked_dirs.is_empty() %}
        <tr><th>Untracked</th><td class="text-muted">{{ report.untracked_dirs.len() }} director{% if report.untracked_dirs.len() != 1 %}ies{% else %}y{% endif %}</td></tr>
        {% endif %}
    </table>
    {% endif %}
</div>
```

- [ ] **Step 2: Add per-section handler functions**

In `src/web/handlers/status.rs`, split `build_health_report()` into three independent handlers:

```rust
use crate::health::{self, ServerHealth, ModsHealth, IntegrityHealth};

#[derive(Template)]
#[template(path = "partials/status_server.html")]
struct StatusServerTemplate {
    report: ServerHealth,
}

#[derive(Template)]
#[template(path = "partials/status_mods.html")]
struct StatusModsTemplate {
    report: ModsHealth,
}

#[derive(Template)]
#[template(path = "partials/status_integrity.html")]
struct StatusIntegrityTemplate {
    report: IntegrityHealth,
}

pub async fn server_partial(state: Data<AppState>) -> actix_web::Result<Html> {
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port).map_err(WebError::from)?;
    let address = spt_client.base_url().to_string();
    let report = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;
    let tmpl = StatusServerTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn mods_partial(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    require_auth(&session)?;
    let db = state.db.clone();
    let installed_mods = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let loaded_mods = if let Ok(spt_client) = crate::spt::server::SptClient::new(&host, port) {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let report = health::check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &state.forge,
        &state.spt_info.spt_version,
    )
    .await;
    let tmpl = StatusModsTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn integrity_partial(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    require_auth(&session)?;
    let db = state.db.clone();
    let tracked_files = web::block(move || {
        let db = db.lock();
        db.get_all_tracked_files()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let report = web::block(move || health::check_integrity_from(&tracked_files, &spt_dir))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    let tmpl = StatusIntegrityTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

Remove the old `build_health_report()` function and `status_partial` handler. Remove the old `StatusDetailTemplate` struct.

- [ ] **Step 3: Register new routes and remove old one**

In `src/web/mod.rs`, in the `/api` scope:
- Remove: `.route("/status", web::get().to(handlers::status::status_partial))`
- Add:
```rust
.route("/status/server", web::get().to(handlers::status::server_partial))
.route("/status/mods", web::get().to(handlers::status::mods_partial))
.route("/status/integrity", web::get().to(handlers::status::integrity_partial))
```

- [ ] **Step 4: Update status.html to load sections independently**

Replace the single `hx-get="/api/status"` div with three independent sections. Wrap mods + integrity in a `status-grid` div to preserve the two-column layout:

```html
<div hx-get="/api/status/server" hx-trigger="load, every 30s" hx-swap="innerHTML">
    <div class="hero-card"><div class="hero-status"><span class="text-muted loading-pulse">Loading server status...</span></div></div>
</div>

<div class="status-grid">
    <div hx-get="/api/status/mods" hx-trigger="load, every 60s" hx-swap="innerHTML">
        <div class="card"><h2>Mods</h2><p class="text-muted loading-pulse">Loading...</p></div>
    </div>
    <div hx-get="/api/status/integrity" hx-trigger="load, every 120s" hx-swap="innerHTML">
        <div class="card"><h2>Integrity</h2><p class="text-muted loading-pulse">Checking...</p></div>
    </div>
</div>
```

- [ ] **Step 5: Delete `templates/partials/status_detail.html`**

It's fully replaced by the three new partials.

- [ ] **Step 6: Build and test**

Run: `cargo build`
Run: `cargo test`

Manually test: open the status page, verify each section loads independently with its own loading skeleton, and that the final design matches the previous look.

- [ ] **Step 7: Commit**

```bash
git add templates/partials/status_server.html templates/partials/status_mods.html templates/partials/status_integrity.html templates/status.html src/web/handlers/status.rs src/web/mod.rs
git rm templates/partials/status_detail.html
git commit -m "feat: load status page sections in parallel via HTMX"
```

---

### Task 6: Fix server control error handling

Server start/stop/restart buttons fail silently or show unstyled errors when the podman command fails (e.g. container not found, podman not installed). The handlers should catch errors and redirect back with a flash message instead of returning a raw error response.

**Files:**
- Modify: `src/web/handlers/server.rs` (catch podman errors, redirect with flash)

**Interfaces:**
- Consumes: `PodmanClient.start()/.stop()`, `set_flash()`
- Produces: Flash error messages on server control failures

- [ ] **Step 1: Refactor start_server to flash on error**

Replace the current pattern that propagates errors as WebError. Instead, catch the error and redirect with a flash message:

```rust
pub async fn start_server(
    state: Data<AppState>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let container = match state.config.server_container.as_deref() {
        Some(c) => c,
        None => {
            set_flash(
                &session,
                "No server_container configured. Set it in quartermaster.toml.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/status"))
                .finish());
        }
    };

    let podman = PodmanClient::new(container);
    if let Err(e) = podman.start().await {
        tracing::error!(container, error = %e, "failed to start server");
        set_flash(&session, &format!("Failed to start server: {e}"), "error");
    } else {
        tracing::info!(container, "server started");
        set_flash(&session, "Server starting", "success");
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}
```

- [ ] **Step 2: Apply same pattern to stop_server**

Same structure: catch the podman error, set a flash error message, redirect to /status.

```rust
pub async fn stop_server(
    state: Data<AppState>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let container = match state.config.server_container.as_deref() {
        Some(c) => c,
        None => {
            set_flash(
                &session,
                "No server_container configured. Set it in quartermaster.toml.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/status"))
                .finish());
        }
    };

    let podman = PodmanClient::new(container);
    if let Err(e) = podman.stop().await {
        tracing::error!(container, error = %e, "failed to stop server");
        set_flash(&session, &format!("Failed to stop server: {e}"), "error");
    } else {
        tracing::info!(container, "server stopped");
        set_flash(&session, "Server stopped", "success");
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}
```

- [ ] **Step 3: Apply same pattern to restart_server**

Handle both stop and start errors independently. If stop fails, flash the error and don't attempt start or drain:

```rust
pub async fn restart_server(
    state: Data<AppState>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let container = match state.config.server_container.as_deref() {
        Some(c) => c,
        None => {
            set_flash(
                &session,
                "No server_container configured. Set it in quartermaster.toml.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/status"))
                .finish());
        }
    };

    let podman = PodmanClient::new(container);

    // Stop first
    if let Err(e) = podman.stop().await {
        tracing::error!(container, error = %e, "failed to stop server for restart");
        set_flash(&session, &format!("Failed to stop server: {e}"), "error");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/status"))
            .finish());
    }

    // Drain queue if configured (existing logic preserved)
    if state.config.auto_drain_on_lifecycle {
        let db = state.db.clone();
        let ops = web::block(move || {
            let db = db.lock();
            db.list_pending_ops()
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        for op in &ops {
            let result = match op.action.as_str() {
                "install" => super::queue::apply_install(op, &state).await,
                "update" => super::queue::apply_update(op, &state).await,
                "remove" => super::queue::apply_remove(op, &state).await,
                _ => Ok(()),
            };
            if result.is_ok() {
                let db = state.db.clone();
                let op_id = op.id;
                let _ = web::block(move || {
                    let db = db.lock();
                    db.delete_pending_op(op_id)
                })
                .await;
            }
        }
    }

    // Start
    if let Err(e) = podman.start().await {
        tracing::error!(container, error = %e, "failed to start server after restart");
        set_flash(&session, &format!("Server stopped but failed to start: {e}"), "error");
    } else {
        tracing::info!(container, "server restarted");
        set_flash(&session, "Server restarting", "success");
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}
```

- [ ] **Step 4: Build and test**

Run: `cargo build`
Run: `cargo test`

Manually test: with no podman container configured (or a wrong name), click start/stop/restart and verify a styled flash error appears on the status page.

- [ ] **Step 5: Commit**

```bash
git add src/web/handlers/server.rs
git commit -m "fix: show flash errors for server control failures instead of raw errors"
```
