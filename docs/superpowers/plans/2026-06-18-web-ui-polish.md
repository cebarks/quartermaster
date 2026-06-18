# Web UI Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish the Quartermaster web UI with visual refinements (cards, badges, typography, icons) and UX improvements (flash toasts, confirm dialogs, dashboard stats, status hero card).

**Architecture:** CSS-first polish with no new dependencies. All changes use the existing Actix-web + Askama + HTMX stack. New Rust code is limited to a flash message helper module and one new HTMX partial endpoint. SVG icons are embedded inline via Askama macros.

**Tech Stack:** Rust (Actix-web 4, Askama 0.16, actix-session), HTMX 2.0.4, custom CSS

## Global Constraints

- No new crate dependencies — everything uses existing deps (actix-session, askama, actix-web)
- No external CDN, no CSS framework, no build step
- `--radius` CSS variable stays at `6px` — card border-radius uses a card-specific override
- All SVG icons are `16x16`, `fill="none" stroke="currentColor" stroke-width="2"` (Lucide-style line icons)
- Flash toasts do NOT apply to login/register pages (they use their own `error: Option<String>` pattern)
- Desktop-only layout — no mobile breakpoints
- Askama templates are compile-time structs: the child template struct's fields are accessible in all blocks of the base template

---

### Task 1: Global CSS Polish

**Files:**
- Modify: `src/assets/style.css`

**Interfaces:**
- Consumes: nothing
- Produces: CSS classes used by all subsequent tasks: `.stat-card`, `.stat-card-grid`, `.empty-state`, `.toast`, `.toast-success`, `.toast-error`, `.toast-warning`, `.hero-card`, `.hero-card.up`, `.hero-card.down`, `.status-grid`, `.status-dot.glow`, `.icon`

- [ ] **Step 1: Write the updated style.css**

Update `src/assets/style.css` with the following modifications to existing rules, then add new classes at the end. The `:root` variables and overall structure stay the same. Changes:

**Cards:** `box-shadow: 0 2px 8px rgba(0,0,0,0.2)`, `border-radius: 8px`, `padding: 1.5rem`

**Buttons:** transition `background, border-color 0.15s`, `:focus-visible` ring (`outline: 2px solid var(--accent); outline-offset: 2px`), `.btn-success`/`.btn-warning` use `color: #0d1117`

**Tables:** row padding `0.65rem 0.75rem`, border `rgba(42,42,74,0.6)`, hover `rgba(255,255,255,0.05)`

**Typography:** `h1` gets `border-bottom: 1px solid var(--border); padding-bottom: 0.5rem; margin-bottom: 1.25rem`

**Badges:** semi-transparent fills, `border-radius: 10px` pill, each variant gets `background: rgba(color, 0.15); color: <color>; border: 1px solid rgba(color, 0.3)`

**Links:** `a:hover { text-decoration: underline; }`

**New classes:**

```css
/* Stat cards (dashboard, status) */
.stat-card-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 1rem;
    margin-bottom: 1.25rem;
}
.stat-card {
    display: block;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    border-left: 3px solid var(--border);
    padding: 1rem 1.25rem;
    color: var(--text);
    text-decoration: none;
    transition: border-color 0.15s;
    box-shadow: 0 2px 8px rgba(0,0,0,0.2);
}
.stat-card:hover { border-color: var(--accent); color: var(--text); text-decoration: none; }
.stat-label { font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.5px; }
.stat-value { font-size: 1.5rem; font-weight: 700; margin-top: 0.15rem; }
.stat-detail { font-size: 0.8rem; color: var(--text-muted); margin-top: 0.15rem; }

/* Hero card (status page) */
.hero-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.5rem;
    margin-bottom: 1rem;
    box-shadow: 0 2px 8px rgba(0,0,0,0.2);
}
.hero-card.up { background: rgba(78,204,163,0.06); border-color: rgba(78,204,163,0.3); }
.hero-card.down { background: rgba(233,69,96,0.06); border-color: rgba(233,69,96,0.3); }
.hero-status { font-size: 1.25rem; font-weight: 700; display: flex; align-items: center; gap: 0.5rem; }
.hero-meta { font-size: 0.85rem; color: var(--text-muted); margin-top: 0.5rem; }

/* Status grid (2-column) */
.status-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin-bottom: 1rem;
}

/* Status dot glow */
.status-dot.glow { box-shadow: 0 0 6px rgba(78,204,163,0.5); }

/* Empty states */
.empty-state {
    text-align: center;
    padding: 2rem;
    color: var(--text-muted);
}
.empty-state .icon { margin-bottom: 0.5rem; }

/* Toast notifications */
.toast {
    padding: 0.75rem 1rem;
    border-radius: 8px;
    margin-bottom: 1rem;
    font-size: 0.9rem;
    animation: toast-in 0.3s ease, toast-out 0.3s ease 3s forwards;
}
.toast-success { background: rgba(78,204,163,0.15); border: 1px solid rgba(78,204,163,0.3); color: var(--success); }
.toast-error { background: rgba(233,69,96,0.15); border: 1px solid rgba(233,69,96,0.3); color: var(--danger); }
.toast-warning { background: rgba(240,192,64,0.15); border: 1px solid rgba(240,192,64,0.3); color: var(--warning); }
@keyframes toast-in { from { opacity: 0; transform: translateY(-10px); } to { opacity: 1; transform: translateY(0); } }
@keyframes toast-out { from { opacity: 1; } to { opacity: 0; } }

/* Auth page centering */
.auth-center { max-width: 400px; margin: 4rem auto; }

/* Inline SVG icon sizing */
.icon { display: inline-block; width: 16px; height: 16px; vertical-align: middle; flex-shrink: 0; }
.icon-sm { width: 14px; height: 14px; }

/* HTMX loading indicator */
.htmx-indicator { display: none; }
.htmx-request .htmx-indicator { display: inline-block; }
.htmx-request .btn { pointer-events: none; opacity: 0.6; }

/* Loading pulse */
@keyframes pulse { 0%, 100% { opacity: 0.4; } 50% { opacity: 1; } }
.loading-pulse { animation: pulse 1.5s ease-in-out infinite; }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` with no errors (CSS is embedded via rust-embed, so compilation validates the file exists)

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass (CSS changes don't affect test outcomes)

- [ ] **Step 4: Commit**

```bash
git add src/assets/style.css
git commit -m "style: global CSS polish — cards, badges, buttons, typography, new component classes"
```

---

### Task 2: SVG Icon Macros

**Files:**
- Create: `templates/partials/icons.html`

**Interfaces:**
- Consumes: `.icon` CSS class from Task 1
- Produces: Askama macros callable as `{% call icons::home() %}`, `{% call icons::package() %}`, etc. from any template that imports `partials/icons.html`

- [ ] **Step 1: Create the icons template**

Create `templates/partials/icons.html` with Askama macros for each icon. All icons are inline SVGs, 16x16, stroke-based (Lucide-style).

```html
{% macro home() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>{% endmacro %}

{% macro package() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="16.5" y1="9.4" x2="7.5" y2="4.21"/><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/><polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/></svg>{% endmacro %}

{% macro list() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>{% endmacro %}

{% macro activity() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>{% endmacro %}

{% macro play() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>{% endmacro %}

{% macro refresh() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>{% endmacro %}

{% macro stop() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/></svg>{% endmacro %}

{% macro download() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>{% endmacro %}

{% macro trash() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>{% endmacro %}

{% macro x() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>{% endmacro %}

{% macro log_out() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>{% endmacro %}

{% macro check() %}<svg class="icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>{% endmacro %}
```

- [ ] **Step 2: Verify it compiles**

Askama validates templates at compile time. Any syntax error in the macros will cause a build failure.

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` (the icons file is not imported yet, so it won't be checked — but we verify no parse errors in the next task when templates import it)

- [ ] **Step 3: Commit**

```bash
git add templates/partials/icons.html
git commit -m "feat: add inline SVG icon macros (Lucide-style, 12 icons)"
```

---

### Task 3: Flash Message Module

**Files:**
- Create: `src/web/flash.rs`
- Modify: `src/web/mod.rs` (add `pub mod flash;`)

**Interfaces:**
- Consumes: `actix_session::Session`
- Produces:
  - `FlashMessage { message: String, flash_type: String }` — struct passed to template structs
  - `set_flash(session: &Session, message: &str, flash_type: &str)` — stores flash in session
  - `take_flash(session: &Session) -> Option<FlashMessage>` — reads and clears flash from session

- [ ] **Step 1: Write the failing test**

Create `src/web/flash.rs`:

```rust
use actix_session::Session;

#[derive(Debug, Clone)]
pub struct FlashMessage {
    pub message: String,
    pub flash_type: String,
}

pub fn set_flash(session: &Session, message: &str, flash_type: &str) {
    let _ = session.insert("flash_message", message);
    let _ = session.insert("flash_type", flash_type);
}

pub fn take_flash(session: &Session) -> Option<FlashMessage> {
    let message = session.remove("flash_message").ok()??;
    let flash_type = session.remove("flash_type").ok()?.unwrap_or_else(|| "success".to_string());
    Some(FlashMessage { message, flash_type })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_message_struct() {
        let flash = FlashMessage {
            message: "Mod installed".to_string(),
            flash_type: "success".to_string(),
        };
        assert_eq!(flash.message, "Mod installed");
        assert_eq!(flash.flash_type, "success");
    }
}
```

Note: `set_flash` and `take_flash` require a live `Session` (backed by actix middleware) so we can't unit test them without an integration test harness. The struct test verifies the data model. The real test is that `cargo build` succeeds with correct types and the flash renders correctly in the UI.

- [ ] **Step 2: Register the module**

In `src/web/mod.rs`, add `pub mod flash;` after the existing module declarations:

```rust
pub mod auth;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod state;
```

- [ ] **Step 3: Verify it compiles and tests pass**

Run: `cargo test web::flash 2>&1 | tail -5`
Expected: `test web::flash::tests::flash_message_struct ... ok`

- [ ] **Step 4: Commit**

```bash
git add src/web/flash.rs src/web/mod.rs
git commit -m "feat: add flash message module (set_flash, take_flash, FlashMessage)"
```

---

### Task 4: Flash Integration — Base Template + All Handlers

**Files:**
- Modify: `templates/base.html` — add toast rendering in flash block
- Modify: `src/web/handlers/dashboard.rs` — add `flash` field to `DashboardTemplate`, read flash in handler
- Modify: `src/web/handlers/mods.rs` — add `flash` field to `ModListTemplate` and `ModDetailTemplate`, read flash in GET handlers, set flash in POST handlers
- Modify: `src/web/handlers/queue.rs` — add `flash` field to `QueueTemplate`, read flash in GET handler, set flash in POST handlers
- Modify: `src/web/handlers/status.rs` — add `flash` field to `StatusPageTemplate`, read flash in GET handler
- Modify: `src/web/handlers/server.rs` — set flash before redirects

**Interfaces:**
- Consumes: `FlashMessage`, `set_flash()`, `take_flash()` from Task 3
- Produces: All page templates now render toast notifications when flash is present

- [ ] **Step 1: Update base.html flash block**

Replace the `{% block flash %}{% endblock %}` line in `templates/base.html` with:

```html
{% block flash %}
{% if let Some(flash) = flash %}
<div class="toast toast-{{ flash.flash_type }}">{{ flash.message }}</div>
{% endif %}
{% endblock %}
```

The complete `templates/base.html` becomes:

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{% block title %}Quartermaster{% endblock %}</title>
    <link rel="stylesheet" href="/assets/style.css">
    <script src="/assets/htmx.min.js"></script>
</head>
<body>
    <nav>
        <span class="brand">Quartermaster</span>
        {% block nav %}{% endblock %}
    </nav>
    <main>
        {% block flash %}
        {% if let Some(flash) = flash %}
        <div class="toast toast-{{ flash.flash_type }}">{{ flash.message }}</div>
        {% endif %}
        {% endblock %}
        {% block content %}{% endblock %}
    </main>
</body>
</html>
```

- [ ] **Step 2: Update DashboardTemplate and handler**

In `src/web/handlers/dashboard.rs`:

Add `use crate::web::flash::{take_flash, FlashMessage};` to imports.

Add `flash: Option<FlashMessage>` field to `DashboardTemplate`:

```rust
#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    user: SessionUser,
    mods: Vec<InstalledMod>,
    pending_count: usize,
    unmanaged_dirs: Vec<(String, usize)>,
    flash: Option<FlashMessage>,
}
```

In the `dashboard` handler, read flash before building template:

```rust
pub async fn dashboard(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let flash = take_flash(&session);

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();

    let (mods, pending_count, unmanaged_dirs) = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let pending = db.list_pending_ops()?;
        let (dirs, _total) = find_unmanaged_mod_dirs(&spt_dir, &db)?;
        let dirs_vec: Vec<(String, usize)> = dirs.into_iter().collect();
        Ok::<_, anyhow::Error>((mods, pending.len(), dirs_vec))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = DashboardTemplate {
        user,
        mods,
        pending_count,
        unmanaged_dirs,
        flash,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 3: Update ModListTemplate, ModDetailTemplate, and mods handlers**

In `src/web/handlers/mods.rs`:

Add `use crate::web::flash::{set_flash, take_flash, FlashMessage};` to imports.

Add `flash: Option<FlashMessage>` to `ModListTemplate` and `ModDetailTemplate`:

```rust
#[derive(Template)]
#[template(path = "mods/list.html")]
struct ModListTemplate {
    user: SessionUser,
    mods: Vec<ModListEntry>,
    flash: Option<FlashMessage>,
}

#[derive(Template)]
#[template(path = "mods/detail.html")]
struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
    flash: Option<FlashMessage>,
}
```

In `list_mods`, add `let flash = take_flash(&session);` after `require_admin` and pass `flash` to template.

In `mod_detail`, add `let flash = take_flash(&session);` after `require_auth` and pass `flash` to template.

In `install_mod`, add `set_flash(&session, "Mod queued for install", "success");` before each `Ok(HttpResponse::SeeOther()...)` redirect (both the queue path and the direct install path).

In `update_mod`, add `set_flash(&session, "Update queued", "success");` before redirects (both queue and direct paths). For the "already up to date" early return, use `set_flash(&session, "Already up to date", "warning");`.

In `remove_mod`, add `set_flash(&session, "Mod queued for removal", "success");` before redirects.

In `update_all_mods`, add `set_flash(&session, "All updates queued", "success");` before the queue redirect, and `set_flash(&session, "All mods updated", "success");` before the direct redirect.

- [ ] **Step 4: Update QueueTemplate and queue handlers**

In `src/web/handlers/queue.rs`:

Add `use crate::web::flash::{set_flash, take_flash, FlashMessage};` to imports.

Add `flash: Option<FlashMessage>` to `QueueTemplate`:

```rust
#[derive(Template)]
#[template(path = "queue.html")]
struct QueueTemplate {
    user: SessionUser,
    ops: Vec<PendingOperation>,
    flash: Option<FlashMessage>,
}
```

In `queue_page`, add `let flash = take_flash(&session);` after `require_auth` and pass to template.

In `cancel_op`, add `set_flash(&session, "Operation cancelled", "success");` before the redirect.

In `apply_queue`, change the server-running early return from `HttpResponse::BadRequest().body(...)` to a flash redirect:

```rust
if server_running {
    set_flash(&session, "Cannot apply queue while server is running. Stop the server first.", "error");
    return Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish());
}
```

For the success path, add `set_flash(&session, "Queue applied successfully", "success");` before the redirect. For the failure path, change the `HttpResponse::InternalServerError().body(msg)` to a flash redirect:

```rust
if !failures.is_empty() {
    let msg = format!("{} operation(s) failed", failures.len());
    set_flash(&session, &msg, "error");
    return Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish());
}
```

- [ ] **Step 5: Update StatusPageTemplate and status handler**

In `src/web/handlers/status.rs`:

Add `use crate::web::flash::{take_flash, FlashMessage};` to imports.

Add `flash: Option<FlashMessage>` to `StatusPageTemplate`:

```rust
#[derive(Template)]
#[template(path = "status.html")]
struct StatusPageTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
}
```

In `status_page`, add `let flash = take_flash(&session);` after `require_auth` and pass to template.

- [ ] **Step 6: Update server handlers to set flash**

In `src/web/handlers/server.rs`:

Add `use crate::web::flash::set_flash;` to imports.

In `start_server`, add `set_flash(&session, "Server starting", "success");` before the redirect.

In `stop_server`, add `set_flash(&session, "Server stopped", "success");` before the redirect.

In `restart_server`, add `set_flash(&session, "Server restarting", "success");` before the redirect.

- [ ] **Step 7: Verify it compiles and tests pass**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` with no errors

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add templates/base.html src/web/handlers/
git commit -m "feat: wire flash toast notifications through all handlers and templates"
```

---

### Task 5: Nav + Auth Pages Polish

**Files:**
- Modify: `templates/partials/nav.html` — add icons before link text, icon on logout button
- Modify: `templates/login.html` — minor polish (remove inline style, use auth-card class)
- Modify: `templates/register.html` — minor polish (same)

**Interfaces:**
- Consumes: Icon macros from Task 2, CSS from Task 1
- Produces: Polished nav bar with icons, polished auth pages

- [ ] **Step 1: Update nav.html with icons**

Replace `templates/partials/nav.html` with:

```html
{% import "partials/icons.html" as icons %}
{% macro nav(active, user) %}
<div class="links">
    <a href="/"{% if active == "dashboard" %} class="active"{% endif %}>{% call icons::home() %} Dashboard</a>
    <a href="/mods"{% if active == "mods" %} class="active"{% endif %}>{% call icons::package() %} Mods</a>
    <a href="/queue"{% if active == "queue" %} class="active"{% endif %}>{% call icons::list() %} Queue</a>
    <a href="/status"{% if active == "status" %} class="active"{% endif %}>{% call icons::activity() %} Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">{% call icons::log_out() %} Logout</button>
    </form>
</div>
{% endmacro %}
```

- [ ] **Step 2: Update login.html**

Replace `templates/login.html` — move the inline centering style to the `.auth-center` CSS class (added in Task 1):

```html
{% extends "base.html" %}
{% block title %}Login — Quartermaster{% endblock %}
{% block flash %}{% endblock %}
{% block content %}
<div class="card auth-card auth-center">
    <h2>Login</h2>
    {% if let Some(error) = error %}
    <div class="alert alert-error">{{ error }}</div>
    {% endif %}
    <form method="post" action="/login">
        <label for="username">Username</label>
        <input type="text" id="username" name="username" required autofocus>
        <label for="password">Password</label>
        <input type="password" id="password" name="password" required>
        <button type="submit" class="btn">Login</button>
    </form>
</div>
{% endblock %}
```

Note: `{% block flash %}{% endblock %}` is overridden to be empty because login/register don't have a `flash` field and don't use the toast system.

- [ ] **Step 3: Update register.html**

```html
{% extends "base.html" %}
{% block title %}Register — Quartermaster{% endblock %}
{% block flash %}{% endblock %}
{% block content %}
<div class="card auth-card auth-center">
    <h2>Register</h2>
    {% if let Some(error) = error %}
    <div class="alert alert-error">{{ error }}</div>
    {% endif %}
    <form method="post" action="/register">
        <input type="hidden" name="code" value="{{ code }}">
        <label for="profile">SPT Profile</label>
        <select id="profile" name="profile_id" required>
            <option value="">— Select your profile —</option>
            {% for p in profiles %}
            <option value="{{ p.aid }}">{{ p.username }}</option>
            {% endfor %}
        </select>
        <label for="password">Password</label>
        <input type="password" id="password" name="password" required minlength="4">
        <label for="password_confirm">Confirm Password</label>
        <input type="password" id="password_confirm" name="password_confirm" required minlength="4">
        <button type="submit" class="btn">Register</button>
    </form>
</div>
{% endblock %}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` — Askama will validate that all icon macros resolve correctly

- [ ] **Step 5: Commit**

```bash
git add templates/partials/nav.html templates/login.html templates/register.html
git commit -m "style: add icons to nav, polish auth pages"
```

---

### Task 6: Dashboard Redesign

**Files:**
- Modify: `templates/dashboard.html` — stat row, empty states, polished mod table
- Create: `templates/partials/dashboard_server_status.html` — HTMX partial for server stat card
- Modify: `src/web/handlers/dashboard.rs` — add `server_status_partial` handler
- Modify: `src/web/handlers/status.rs` — make `build_health_report` helper logic reusable (extract `check_server_reachability`)
- Modify: `src/web/mod.rs` — register `/api/dashboard/server-status` route

**Interfaces:**
- Consumes: `.stat-card-grid`, `.stat-card`, `.stat-label`, `.stat-value`, `.stat-detail`, `.empty-state` CSS from Task 1; `FlashMessage` from Task 3; `check_server` from `crate::health`
- Produces: Dashboard with stat row (mods/queue/server), polished mod table, actionable empty states, new HTMX endpoint

- [ ] **Step 1: Add a lightweight server reachability check to dashboard handler**

Rather than extracting `build_health_report` (which does a full health check including mods and integrity), the dashboard just needs a quick server ping. The `health::check_server` function is already public and returns `ServerHealth`. Use it directly in a new handler.

In `src/web/handlers/dashboard.rs`, add the server status partial handler. Add these imports:

```rust
use crate::health;
use crate::server_detect::resolve_server_addr;
use crate::spt::server::SptClient;
```

Add a new template struct and handler:

```rust
#[derive(Template)]
#[template(path = "partials/dashboard_server_status.html")]
struct DashboardServerStatusTemplate {
    reachable: bool,
    latency_ms: Option<u64>,
}

pub async fn server_status_partial(state: Data<AppState>) -> actix_web::Result<Html> {
    let (host, port) = resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = SptClient::new(&host, port).map_err(WebError::from)?;
    let address = spt_client.base_url().to_string();

    let server = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;

    let tmpl = DashboardServerStatusTemplate {
        reachable: server.reachable,
        latency_ms: server.latency_ms,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 2: Create the server status partial template**

Create `templates/partials/dashboard_server_status.html`:

```html
{% if reachable %}
<div class="stat-value" style="font-size: 1rem; display: flex; align-items: center; gap: 0.4rem;">
    <span class="status-dot up glow"></span> Running
</div>
<div class="stat-detail">{% if let Some(ms) = latency_ms %}{{ ms }}ms latency{% endif %}</div>
{% else %}
<div class="stat-value" style="font-size: 1rem; display: flex; align-items: center; gap: 0.4rem;">
    <span class="status-dot down"></span> Down
</div>
{% endif %}
```

- [ ] **Step 3: Register the route**

In `src/web/mod.rs`, add the new route inside the `/api` scope, after the existing routes:

```rust
.service(
    web::scope("/api")
        .wrap(auth::RequireAuth)
        .route(
            "/mods/check-updates",
            web::get().to(handlers::mods::check_updates_partial),
        )
        .route(
            "/mods/dep-tree",
            web::get().to(handlers::mods::dep_tree_partial),
        )
        .route("/status", web::get().to(handlers::status::status_partial))
        .route(
            "/dashboard/server-status",
            web::get().to(handlers::dashboard::server_status_partial),
        ),
)
```

- [ ] **Step 4: Update dashboard.html template**

Replace `templates/dashboard.html` with:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% block title %}Dashboard — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("dashboard", user) %}{% endcall %}{% endblock %}
{% block content %}
<h1>Dashboard</h1>

<div class="stat-card-grid">
    <a href="/mods" class="stat-card" style="border-left-color: var(--success)">
        <div class="stat-label">Mods</div>
        <div class="stat-value">{{ mods.len() }}</div>
        <div class="stat-detail">installed</div>
    </a>
    <a href="/queue" class="stat-card" style="border-left-color: {% if pending_count > 0 %}var(--warning){% else %}var(--success){% endif %}">
        <div class="stat-label">Queue</div>
        <div class="stat-value">{{ pending_count }}</div>
        <div class="stat-detail">{% if pending_count > 0 %}pending{% else %}all clear{% endif %}</div>
    </a>
    <a href="/status" class="stat-card" style="border-left-color: var(--border)">
        <div class="stat-label">Server</div>
        <div hx-get="/api/dashboard/server-status" hx-trigger="load" hx-swap="innerHTML">
            <span class="text-muted text-sm loading-pulse">Checking...</span>
        </div>
    </a>
</div>

<div class="card">
    <div class="flex-between mb-1">
        <h2>Installed Mods ({{ mods.len() }})</h2>
        <span hx-get="/api/mods/check-updates" hx-trigger="load, every 60s" hx-swap="innerHTML">
            <span class="text-muted text-sm">Checking for updates...</span>
        </span>
    </div>
    {% if mods.is_empty() %}
    <div class="empty-state">
        <p>No mods installed.</p>
        {% if user.is_admin() %}<p class="mt-1"><a href="/mods">Go to Mods</a> to install your first mod.</p>{% endif %}
    </div>
    {% else %}
    <table>
        <thead>
            <tr>
                <th>Name</th>
                <th>Version</th>
                <th>Installed</th>
            </tr>
        </thead>
        <tbody>
            {% for m in mods %}
            <tr>
                <td><a href="/mods/{{ m.id }}">{{ m.name }}</a></td>
                <td>{{ m.version }}</td>
                <td class="text-muted text-sm">{{ m.installed_at }}</td>
            </tr>
            {% endfor %}
        </tbody>
    </table>
    {% endif %}
</div>

{% if pending_count > 0 %}
<div class="card">
    <h2>Pending Operations</h2>
    <p>{{ pending_count }} operation(s) queued. <a href="/queue">View queue</a></p>
</div>
{% endif %}

{% if !unmanaged_dirs.is_empty() %}
<div class="card">
    <h2>Unmanaged Mods</h2>
    <p class="text-muted text-sm mb-1">These mod directories are not tracked by Quartermaster. Use <code>quma track</code> to manage them.</p>
    <table>
        <thead><tr><th>Directory</th><th>Files</th></tr></thead>
        <tbody>
            {% for (dir, count) in &unmanaged_dirs %}
            <tr><td>{{ dir }}</td><td>{{ count }}</td></tr>
            {% endfor %}
        </tbody>
    </table>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 5: Verify it compiles and tests pass**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished`

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add templates/dashboard.html templates/partials/dashboard_server_status.html src/web/handlers/dashboard.rs src/web/mod.rs
git commit -m "feat: dashboard stat row with server status partial, polished empty states"
```

---

### Task 7: Status Page Redesign

**Files:**
- Modify: `templates/status.html` — hero card layout, confirm dialogs on server controls
- Modify: `templates/partials/status_detail.html` — hero card + 2-column grid layout

**Interfaces:**
- Consumes: `.hero-card`, `.hero-card.up`, `.hero-card.down`, `.hero-status`, `.hero-meta`, `.status-grid`, `.stat-card`, `.status-dot.glow` CSS from Task 1; icon macros from Task 2; `FlashMessage` from Task 3/4
- Produces: Redesigned status page with hero server card, 2-column mod/integrity grid, confirm dialogs

- [ ] **Step 1: Update status.html with confirm dialogs and icon imports**

Replace `templates/status.html` with:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% import "partials/icons.html" as icons %}
{% block title %}Status — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("status", user) %}{% endcall %}{% endblock %}
{% block content %}
<div class="flex-between">
    <h1>Server Status</h1>
    {% if user.is_admin() %}
    <div class="flex gap-1">
        <form method="post" action="/server/start" style="display:inline">
            <button type="submit" class="btn btn-sm btn-success">{% call icons::play() %} Start</button>
        </form>
        <form method="post" action="/server/restart" style="display:inline"
              onsubmit="return confirm('Restart the server?')">
            <button type="submit" class="btn btn-sm btn-warning">{% call icons::refresh() %} Restart</button>
        </form>
        <form method="post" action="/server/stop" style="display:inline"
              onsubmit="return confirm('Stop the server?')">
            <button type="submit" class="btn btn-sm btn-danger">{% call icons::stop() %} Stop</button>
        </form>
    </div>
    {% endif %}
</div>

<div hx-get="/api/status" hx-trigger="load, every 30s" hx-swap="innerHTML" id="status-content">
    <p class="text-muted loading-pulse">Loading status...</p>
</div>
{% endblock %}
```

- [ ] **Step 2: Redesign status_detail.html with hero card and grid**

Replace `templates/partials/status_detail.html` with:

```html
<div class="hero-card {% if report.server.reachable %}up{% else %}down{% endif %}">
    <div class="hero-status">
        {% if report.server.reachable %}
        <span class="status-dot up glow"></span> Running
        {% if let Some(ms) = report.server.latency_ms %}<span class="text-muted" style="font-size: 0.85rem; font-weight: 400;"> — {{ ms }}ms</span>{% endif %}
        {% else %}
        <span class="status-dot down"></span> Down
        {% if let Some(err) = &report.server.error %}<span class="text-muted" style="font-size: 0.85rem; font-weight: 400;"> — {{ err }}</span>{% endif %}
        {% endif %}
    </div>
    <div class="hero-meta">
        {{ report.server.address }}
        {% if let Some(v) = &report.server.version %}
         · v{{ v }}
            {% if let Some(matches) = report.server.version_matches %}
                {% if matches %}<span class="badge badge-success">matches</span>
                {% else %}<span class="badge badge-danger">mismatch</span>
                {% endif %}
            {% endif %}
        {% endif %}
    </div>
</div>

<div class="status-grid">
    <div class="card" style="border-left: 3px solid {% if report.mods.updates_available > 0 %}var(--warning){% else %}var(--success){% endif %}">
        <h2>Mods</h2>
        <table>
            <tr><th style="width:140px">Installed</th><td><strong>{{ report.mods.installed_count }}</strong></td></tr>
            <tr><th>Updates</th><td>
                {% if report.mods.updates_available > 0 %}
                <span class="badge badge-warning">{{ report.mods.updates_available }} available</span>
                {% else %}
                <span class="text-muted">All up to date</span>
                {% endif %}
            </td></tr>
            {% if !report.mods.incompatible_mods.is_empty() %}
            <tr><th>Incompatible</th><td>
                {% for name in &report.mods.incompatible_mods %}
                <span class="badge badge-danger">{{ name }}</span>
                {% endfor %}
            </td></tr>
            {% endif %}
        </table>
    </div>

    <div class="card" style="border-left: 3px solid {% if report.integrity.missing_files.is_empty() && report.integrity.modified_files.is_empty() %}var(--success){% else %}var(--danger){% endif %}">
        <h2>Integrity</h2>
        {% if report.integrity.missing_files.is_empty() && report.integrity.modified_files.is_empty() && report.integrity.untracked_dirs.is_empty() %}
        <p class="text-muted">{{ report.integrity.tracked_files }} tracked files — all present, hashes match.</p>
        {% else %}
        <table>
            <tr><th style="width:140px">Tracked</th><td>{{ report.integrity.tracked_files }} files</td></tr>
            {% if !report.integrity.missing_files.is_empty() %}
            <tr><th>Missing</th><td><span class="badge badge-danger">{{ report.integrity.missing_files.len() }}</span></td></tr>
            {% endif %}
            {% if !report.integrity.modified_files.is_empty() %}
            <tr><th>Modified</th><td><span class="badge badge-warning">{{ report.integrity.modified_files.len() }}</span></td></tr>
            {% endif %}
            {% if !report.integrity.untracked_dirs.is_empty() %}
            <tr><th>Untracked</th><td class="text-muted">{{ report.integrity.untracked_dirs.len() }} director{% if report.integrity.untracked_dirs.len() != 1 %}ies{% else %}y{% endif %}</td></tr>
            {% endif %}
        </table>
        {% endif %}
    </div>
</div>
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished`

- [ ] **Step 4: Commit**

```bash
git add templates/status.html templates/partials/status_detail.html
git commit -m "style: status page hero card with glow dots, 2-column grid, confirm dialogs"
```

---

### Task 8: Mods + Queue Pages Polish

**Files:**
- Modify: `templates/mods/list.html` — icons on buttons, confirm on Update All, empty state
- Modify: `templates/mods/detail.html` — icons on buttons
- Modify: `templates/queue.html` — icons, confirm dialogs on Cancel/Apply All, empty state

**Interfaces:**
- Consumes: Icon macros from Task 2, `.empty-state` CSS from Task 1, `FlashMessage` from Task 3/4
- Produces: Polished mods and queue pages with icons, confirms, and empty states

- [ ] **Step 1: Update mods/list.html**

Replace `templates/mods/list.html` with:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% import "partials/icons.html" as icons %}
{% block title %}Mods — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("mods", user) %}{% endcall %}{% endblock %}
{% block content %}
<div class="flex-between">
    <h1>Installed Mods</h1>
    {% if user.is_admin() %}
    <div class="flex gap-1">
        <form method="post" action="/mods/update-all" style="display:inline"
              onsubmit="return confirm('Update all mods?')">
            <button type="submit" class="btn btn-sm btn-outline">{% call icons::refresh() %} Update All</button>
        </form>
    </div>
    {% endif %}
</div>

{% if mods.is_empty() %}
<div class="card">
    <div class="empty-state">
        <p>No mods installed.</p>
        {% if user.is_admin() %}<p class="mt-1">Use the form below to install your first mod.</p>{% endif %}
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
                <th>Installed</th>
                {% if user.is_admin() %}<th>Actions</th>{% endif %}
            </tr>
        </thead>
        <tbody>
            {% for m in &mods %}
            <tr>
                <td><a href="/mods/{{ m.mod_info.id }}">{{ m.mod_info.name }}</a></td>
                <td>{{ m.mod_info.version }}</td>
                <td class="text-muted">{{ m.file_count }}</td>
                <td class="text-muted text-sm">{{ m.mod_info.installed_at }}</td>
                {% if user.is_admin() %}
                <td>
                    <form method="post" action="/mods/{{ m.mod_info.id }}/update" style="display:inline">
                        <button type="submit" class="btn btn-sm btn-outline">{% call icons::refresh() %} Update</button>
                    </form>
                    <form method="post" action="/mods/{{ m.mod_info.id }}/remove" style="display:inline"
                          data-name="{{ m.mod_info.name }}"
                          onsubmit="return confirm('Remove ' + this.dataset.name + '?')">
                        <button type="submit" class="btn btn-sm btn-danger">{% call icons::trash() %} Remove</button>
                    </form>
                </td>
                {% endif %}
            </tr>
            {% endfor %}
        </tbody>
    </table>
</div>
{% endif %}

{% if user.is_admin() %}
<div class="card">
    <h2>Install Mod</h2>
    <form method="post" action="/mods/install">
        <label for="mod_ref">Forge Mod ID or Name</label>
        <input type="text" id="mod_ref" name="mod_ref" placeholder="e.g. 2326 or SAIN" required>
        <button type="submit" class="btn">{% call icons::download() %} Install</button>
    </form>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 2: Update mods/detail.html**

Replace `templates/mods/detail.html` with:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% import "partials/icons.html" as icons %}
{% block title %}{{ mod_info.name }} — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("mods", user) %}{% endcall %}{% endblock %}
{% block content %}
<h1>{{ mod_info.name }}</h1>

<div class="card">
    <table>
        <tr><th style="width:140px">Forge ID</th><td>{{ mod_info.forge_mod_id }}</td></tr>
        <tr><th>Version</th><td>{{ mod_info.version }}</td></tr>
        {% if let Some(slug) = &mod_info.slug %}
        <tr><th>Slug</th><td>{{ slug }}</td></tr>
        {% endif %}
        <tr><th>Installed</th><td>{{ mod_info.installed_at }}</td></tr>
        {% if let Some(updated) = &mod_info.updated_at %}
        <tr><th>Updated</th><td>{{ updated }}</td></tr>
        {% endif %}
    </table>
</div>

{% if !dependencies.is_empty() %}
<div class="card">
    <h2>Dependencies</h2>
    <table>
        <thead><tr><th>Mod</th><th>Constraint</th></tr></thead>
        <tbody>
            {% for dep in &dependencies %}
            <tr>
                <td>
                    {% if let Some(dep_mod) = &dep.dep_mod %}
                    <a href="/mods/{{ dep_mod.id }}">{{ dep_mod.name }}</a>
                    {% else %}
                    <span class="text-muted">Unknown (ID: {{ dep.dep.depends_on_mod_id }})</span>
                    {% endif %}
                </td>
                <td class="text-muted">{{ dep.dep.version_constraint.as_deref().unwrap_or("any") }}</td>
            </tr>
            {% endfor %}
        </tbody>
    </table>
</div>
{% endif %}

<div class="card">
    <h2>Files ({{ files.len() }})</h2>
    <table>
        <thead><tr><th>Path</th><th>Size</th></tr></thead>
        <tbody>
            {% for f in &files %}
            <tr>
                <td class="text-sm">{{ f.file_path }}</td>
                <td class="text-muted text-sm">{% if let Some(size) = f.file_size %}{{ size }}{% else %}-{% endif %}</td>
            </tr>
            {% endfor %}
        </tbody>
    </table>
</div>

{% if user.is_admin() %}
<div class="flex gap-1 mt-2">
    <form method="post" action="/mods/{{ mod_info.id }}/update">
        <button type="submit" class="btn">{% call icons::refresh() %} Update</button>
    </form>
    <form method="post" action="/mods/{{ mod_info.id }}/remove"
          data-name="{{ mod_info.name }}"
          onsubmit="return confirm('Remove ' + this.dataset.name + '?')">
        <button type="submit" class="btn btn-danger">{% call icons::trash() %} Remove</button>
    </form>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 3: Update queue.html**

Replace `templates/queue.html` with:

```html
{% extends "base.html" %}
{% import "partials/nav.html" as nav %}
{% import "partials/icons.html" as icons %}
{% block title %}Queue — Quartermaster{% endblock %}
{% block nav %}{% call nav::nav("queue", user) %}{% endcall %}{% endblock %}
{% block content %}
<div class="flex-between">
    <h1>Pending Operations</h1>
    {% if user.is_admin() && !ops.is_empty() %}
    <form method="post" action="/queue/apply"
          onsubmit="return confirm('Apply all pending operations?')">
        <button type="submit" class="btn btn-success">{% call icons::play() %} Apply All</button>
    </form>
    {% endif %}
</div>

{% if ops.is_empty() %}
<div class="card">
    <div class="empty-state">
        {% call icons::check() %}
        <p>All clear — no pending operations.</p>
    </div>
</div>
{% else %}
<div class="card">
    <table>
        <thead>
            <tr>
                <th>Action</th>
                <th>Mod</th>
                <th>Queued</th>
                <th>By</th>
                {% if user.is_admin() %}<th>Actions</th>{% endif %}
            </tr>
        </thead>
        <tbody>
            {% for op in &ops %}
            <tr>
                <td>
                    {% if op.action == "install" %}<span class="badge badge-success">install</span>
                    {% else if op.action == "update" %}<span class="badge badge-warning">update</span>
                    {% else if op.action == "remove" %}<span class="badge badge-danger">remove</span>
                    {% else %}<span class="badge badge-muted">{{ op.action }}</span>
                    {% endif %}
                </td>
                <td>{{ op.mod_name }}</td>
                <td class="text-muted text-sm">{{ op.queued_at }}</td>
                <td class="text-muted text-sm">{{ op.queued_by.as_deref().unwrap_or("-") }}</td>
                {% if user.is_admin() %}
                <td>
                    <form method="post" action="/queue/{{ op.id }}/cancel" style="display:inline"
                          onsubmit="return confirm('Cancel this operation?')">
                        <button type="submit" class="btn btn-sm btn-outline">{% call icons::x() %} Cancel</button>
                    </form>
                </td>
                {% endif %}
            </tr>
            {% endfor %}
        </tbody>
    </table>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 4: Verify it compiles and tests pass**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished`

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add templates/mods/list.html templates/mods/detail.html templates/queue.html
git commit -m "style: mods + queue pages — icons, confirm dialogs, empty states"
```

---

## Task Dependency Graph

```
Task 1 (CSS) ──┐
               ├── Task 5 (Nav + Auth)
Task 2 (Icons) ┤
               ├── Task 6 (Dashboard) ──── requires Task 4
Task 3 (Flash) ┤
               ├── Task 4 (Flash Integration) ── requires Task 3
               │
               ├── Task 7 (Status Page)
               │
               └── Task 8 (Mods + Queue)
```

Tasks 1, 2, 3 can run in parallel. Task 4 depends on Task 3. Tasks 5-8 depend on Tasks 1, 2, and 4.
