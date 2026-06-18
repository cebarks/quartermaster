# TODO(debt) Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve all three `TODO(debt)` items in the codebase: remove dead `config_path` fields, log podman `is_running` errors, and add CSRF token validation to all POST forms.

**Architecture:** Tasks 1-2 are small targeted cleanups. Task 3 adds per-session CSRF tokens validated at the handler level (not middleware — actix-web middleware can't re-read form bodies after extraction). Each POST handler adds a `csrf_token` field to its form struct and validates before processing. Templates embed the token as a hidden input.

**Tech Stack:** Rust, actix-web 4, actix-session, Askama templates, rand 0.9 (already a dependency)

## Global Constraints

- No new crate dependencies
- Follow existing error handling patterns (`eprintln!` for CLI warnings, `WebError` for web layer)
- CSRF tokens are per-session, not per-request (acceptable for this app's threat model — single-user/small-group LAN tool with SameSite=Strict as the primary defense)
- All POST routes must validate CSRF tokens; GET routes must not

---

### Task 1: Remove dead `config_path` fields from `CliContext` and `AppState`

`CliContext.config_path` has zero read references (`ctx.config_path` never appears). The config command (`src/cli/config_cmd.rs`) resolves its own path via `Config::resolve_path()` without using `CliContext`. Same for `AppState.config_path` — stored but never read. Remove both fields, their `#[allow(dead_code)]` annotations, and update all construction sites.

**Files:**
- Modify: `src/cli/common.rs:13-21` (CliContext struct — remove field + field-level `#[allow(dead_code)]`)
- Modify: `src/cli/common.rs:37-44` (resolve_context — remove field from struct literal)
- Modify: `src/web/state.rs:11-19` (AppState struct — remove field + struct-level `#[allow(dead_code)]`)
- Modify: `src/web/mod.rs:39-59` (start_server fn — remove `config_path` parameter + field from AppState literal)
- Modify: `src/cli/serve.rs:39` (passes config_path to start_server)
- Modify: `src/cli/setup.rs:61-68` (constructs CliContext with `config_path: config_path.clone()`)
- Modify: `src/cli/remove.rs:122-132` (test constructs CliContext with config_path)
- Modify: `src/health.rs:560,613,653` (3 test sites construct CliContext with config_path)

**Interfaces:**
- Consumes: nothing
- Produces: `CliContext` without `config_path` field, `AppState` without `config_path` field, `start_server()` without `config_path` parameter

- [ ] **Step 1: Remove `config_path` from `CliContext` struct**

In `src/cli/common.rs`, remove lines 17-18 (the `#[allow(dead_code)]` attribute AND the `pub config_path: PathBuf` field). The attribute is on the field, not the struct:

```rust
pub struct CliContext {
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub config: Config,
    pub db: Database,
    pub forge: ForgeClient,
}
```

- [ ] **Step 2: Remove `config_path` from `resolve_context` struct literal**

In `src/cli/common.rs`, remove `config_path,` from the `CliContext` struct literal at line 41. Keep the local `config_path` variable — it's still needed to load the config:

```rust
    Ok(CliContext {
        spt_dir,
        spt_info,
        config,
        db,
        forge,
    })
```

- [ ] **Step 3: Remove `config_path` from `AppState`**

In `src/web/state.rs`, the `#[allow(dead_code)]` is on the struct itself (line 11), not the field. Remove both the attribute and the `config_path` field:

```rust
pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
}
```

- [ ] **Step 4: Remove `config_path` from `start_server` signature and AppState construction**

In `src/web/mod.rs`, remove the `config_path: std::path::PathBuf` parameter from `start_server()` (line 41) and remove `config_path,` from the AppState struct literal (line 56):

```rust
pub async fn start_server(
    config: Config,
    db: Database,
    forge: ForgeClient,
    spt_dir: std::path::PathBuf,
    spt_info: SptInfo,
) -> Result<()> {
```

```rust
    let app_state = web::Data::new(AppState {
        db,
        forge,
        config: config.clone(),
        spt_dir,
        spt_info,
    });
```

- [ ] **Step 5: Update all callers**

In `src/cli/serve.rs` line 39, remove `config_path` from the `start_server()` call:

```rust
    crate::web::start_server(config, db, forge, spt_dir, spt_info).await
```

In `src/cli/setup.rs` lines 61-68, remove `config_path: config_path.clone(),` from the CliContext literal:

```rust
    let ctx = super::common::CliContext {
        spt_dir: spt_dir.clone(),
        spt_info: spt_info.clone(),
        config: config.clone(),
        db,
        forge,
    };
```

In `src/cli/remove.rs` line 130, remove `config_path: tmp.path().join("quartermaster.toml"),` from the test's CliContext literal.

In `src/health.rs`, remove `config_path: tmp.path().join("quartermaster.toml"),` from all 3 test CliContext literals at lines 568, 621, and 661.

- [ ] **Step 6: Build and run tests**

Run: `cargo build 2>&1 | head -30 && cargo test 2>&1 | tail -20`
Expected: clean compile, all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/cli/common.rs src/web/state.rs src/web/mod.rs src/cli/serve.rs src/cli/setup.rs src/cli/remove.rs src/health.rs
git commit -m "refactor: remove unused config_path from CliContext and AppState"
```

---

### Task 2: Log `is_running` errors instead of swallowing them

The `podman.is_running().await.unwrap_or(false)` call at `src/cli/setup.rs:282` silently swallows errors like permission denied or missing socket. Replace with a match that logs the error before defaulting to `false`. Also remove the two-line TODO comment on lines 280-281.

**Files:**
- Modify: `src/cli/setup.rs:280-282` (replace TODO comment + unwrap_or with match + eprintln)

**Interfaces:**
- Consumes: nothing
- Produces: same behavior (defaults to false on error), but now prints a warning

- [ ] **Step 1: Replace `unwrap_or` with match + eprintln**

In `src/cli/setup.rs`, replace lines 280-282 (the two-line TODO comment AND the `let running = ...` line):

```rust
    let running = match podman.is_running().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: could not check if server is running: {e:#}");
            false
        }
    };
```

- [ ] **Step 2: Build**

Run: `cargo build 2>&1 | head -20`
Expected: clean compile

- [ ] **Step 3: Commit**

```bash
git add src/cli/setup.rs
git commit -m "fix: log podman is_running errors instead of swallowing them"
```

---

### Task 3: Add CSRF protection to POST forms

Add defense-in-depth CSRF token validation to all POST routes. Per-session token stored in `actix-session`, validated at the handler level via a `csrf_token` form field. No middleware approach — actix-web middleware can't re-read form bodies after `Form<T>` extraction.

**Inventory of all POST handlers and their current signatures:**

| Handler | File | Currently takes `Form<T>`? | Currently takes `Session`? |
|---------|------|---------------------------|---------------------------|
| `login_submit` | `handlers/auth.rs:87` | Yes (`Form<LoginForm>`) | Yes |
| `register_submit` | `handlers/auth.rs:225` | Yes (`Form<RegisterForm>`) | Yes (`_session`) |
| `logout` | `handlers/auth.rs:315` | No body | Yes |
| `install_mod` | `handlers/mods.rs:184` | Yes (`Form<InstallForm>`) | Yes |
| `update_mod` | `handlers/mods.rs:299` | No body (only `Path<i64>`) | Yes |
| `remove_mod` | `handlers/mods.rs:400` | No body (only `Path<i64>`) | Yes |
| `update_all_mods` | `handlers/mods.rs:457` | No body | Yes |
| `cancel_op` | `handlers/queue.rs:34` | No body (only `Path<i64>`) | Yes |
| `apply_queue` | `handlers/queue.rs:56` | No body | Yes |
| `start_server` | `handlers/server.rs:10` | No body | Yes |
| `stop_server` | `handlers/server.rs:31` | No body | Yes |
| `restart_server` | `handlers/server.rs:52` | No body | Yes |

**Inventory of all templates with `<form method="post">`:**

| Template | Forms | Needs `csrf_token` var from |
|----------|-------|-----------------------------|
| `templates/login.html` | 1 (login) | `LoginTemplate` |
| `templates/register.html` | 1 (register) | `RegisterTemplate` |
| `templates/partials/nav.html` | 1 (logout) | nav macro `csrf_token` param |
| `templates/status.html` | 3 (start, restart, stop) | `StatusPageTemplate` + nav macro |
| `templates/queue.html` | 2 (apply, cancel) | `QueueTemplate` + nav macro |
| `templates/mods/list.html` | 4 (update-all, N×update, N×remove, install) | `ModListTemplate` + nav macro |
| `templates/mods/detail.html` | 2 (update, remove) | `ModDetailTemplate` + nav macro |
| `templates/dashboard.html` | 0 (but has nav with logout form) | `DashboardTemplate` + nav macro |

**Inventory of all template construction sites (including error paths):**

| Site | File:Line | Notes |
|------|-----------|-------|
| `LoginTemplate { error: None }` | `handlers/auth.rs:81` | `login_page` — needs Session param added |
| `LoginTemplate { error: Some(...) }` | `handlers/auth.rs:107-113` | login fail: bad user |
| `LoginTemplate { error: Some(...) }` | `handlers/auth.rs:128-133` | login fail: bad password |
| `RegisterTemplate { error: Some(...), code, profiles: vec![] }` | `handlers/auth.rs:156-163` | register: empty code |
| `RegisterTemplate { error: Some(...), code, profiles: vec![] }` | `handlers/auth.rs:178-185` | register: invalid code |
| `RegisterTemplate { error: Some(...), code, profiles: vec![] }` | `handlers/auth.rs:188-195` | register: used code |
| `RegisterTemplate { error: Some(...), code, profiles: vec![] }` | `handlers/auth.rs:198-206` | register: expired code |
| `RegisterTemplate { error: None, code, profiles }` | `handlers/auth.rs:213-220` | register: success render |
| `render_register_error(msg, code, profiles)` | `handlers/auth.rs:53-66` | helper called from register_submit — must accept csrf_token |
| All `register_submit` calls to `render_register_error` | `handlers/auth.rs:239,247,255,259` | must pass csrf_token |
| `ModListTemplate { user, mods }` | `handlers/mods.rs:90` | |
| `ModDetailTemplate { user, mod_info, files, dependencies }` | `handlers/mods.rs:121-126` | |
| `StatusPageTemplate { user }` | `handlers/status.rs:24` | |
| `QueueTemplate { user, ops }` | `handlers/queue.rs:30` | |
| `DashboardTemplate { user, mods, pending_count, unmanaged_dirs }` | `handlers/dashboard.rs:38-43` | |

**Files:**
- Create: `src/web/csrf.rs` (token generation + validation functions only — no middleware)
- Modify: `src/web/mod.rs:1` (add `pub mod csrf;`, remove TODO comment at line 85)
- Modify: `src/web/handlers/auth.rs` (add csrf_token to LoginTemplate, RegisterTemplate, render_register_error, LoginForm, RegisterForm; add Session to login_page and register_page; validate in login_submit and register_submit; pass token in all error render paths)
- Modify: `src/web/handlers/mods.rs` (add csrf_token to ModListTemplate, ModDetailTemplate; add CsrfForm to update_mod, remove_mod, update_all_mods; validate in install_mod, update_mod, remove_mod, update_all_mods)
- Modify: `src/web/handlers/queue.rs` (add csrf_token to QueueTemplate; add CsrfForm to cancel_op, apply_queue; validate in both)
- Modify: `src/web/handlers/status.rs` (add csrf_token to StatusPageTemplate)
- Modify: `src/web/handlers/server.rs` (add CsrfForm + validate in start_server, stop_server, restart_server)
- Modify: `src/web/handlers/dashboard.rs` (add csrf_token to DashboardTemplate)
- Modify: `templates/partials/nav.html` (add csrf_token param to macro, add hidden input)
- Modify: `templates/login.html` (add hidden input)
- Modify: `templates/register.html` (add hidden input)
- Modify: `templates/status.html` (add hidden inputs, pass csrf_token to nav macro)
- Modify: `templates/queue.html` (add hidden inputs, pass csrf_token to nav macro)
- Modify: `templates/mods/list.html` (add hidden inputs, pass csrf_token to nav macro)
- Modify: `templates/mods/detail.html` (add hidden inputs, pass csrf_token to nav macro)
- Modify: `templates/dashboard.html` (pass csrf_token to nav macro)

**Interfaces:**
- Consumes: `actix_session::Session` for token storage
- Produces: `csrf::get_or_create_token(session) -> String` for handlers, `csrf::validate_token(session, form_token) -> bool` for validation

- [ ] **Step 1: Create `src/web/csrf.rs` with token functions and unit test**

```rust
use actix_session::Session;
use rand::Rng;

const CSRF_SESSION_KEY: &str = "csrf_token";
const TOKEN_LEN: usize = 32;

pub fn get_or_create_token(session: &Session) -> String {
    if let Ok(Some(token)) = session.get::<String>(CSRF_SESSION_KEY) {
        return token;
    }
    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(TOKEN_LEN)
        .map(char::from)
        .collect();
    let _ = session.insert(CSRF_SESSION_KEY, &token);
    token
}

pub fn validate_token(session: &Session, form_token: &str) -> bool {
    match session.get::<String>(CSRF_SESSION_KEY) {
        Ok(Some(session_token)) => session_token == form_token,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_length_and_charset() {
        let token: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(TOKEN_LEN)
            .map(char::from)
            .collect();
        assert_eq!(token.len(), TOKEN_LEN);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
```

Note: `validate_token` uses plain `==` comparison. Constant-time comparison (via `subtle` crate) is unnecessary for this threat model — LAN-only tool with SameSite=Strict as the primary defense and no network-observable timing oracle.

- [ ] **Step 2: Register csrf module and remove TODO comment**

In `src/web/mod.rs`, add `pub mod csrf;` after the existing module declarations (line 4).

Remove line 85:
```
// TODO(debt): add CSRF protection on state-mutating POST forms (SameSite=Strict mitigates most vectors)
```

- [ ] **Step 3: Run test to verify csrf module compiles**

Run: `cargo test csrf --lib -v 2>&1`
Expected: PASS — `token_length_and_charset` passes

- [ ] **Step 4: Create shared `CsrfForm` struct**

Add to the bottom of `src/web/csrf.rs` (before the `#[cfg(test)]` block):

```rust
#[derive(serde::Deserialize)]
pub struct CsrfForm {
    pub csrf_token: String,
}
```

This is used by POST handlers that currently accept no form body (logout, update_mod, remove_mod, update_all_mods, cancel_op, apply_queue, start_server, stop_server, restart_server).

- [ ] **Step 5: Update nav macro to accept csrf_token**

In `templates/partials/nav.html`, change the macro signature and add the hidden input to the logout form:

```html
{% macro nav(active, user, csrf_token) %}
<div class="links">
    <a href="/"{% if active == "dashboard" %} class="active"{% endif %}>Dashboard</a>
    <a href="/mods"{% if active == "mods" %} class="active"{% endif %}>Mods</a>
    <a href="/queue"{% if active == "queue" %} class="active"{% endif %}>Queue</a>
    <a href="/status"{% if active == "status" %} class="active"{% endif %}>Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">Logout</button>
    </form>
</div>
{% endmacro %}
```

- [ ] **Step 6: Update all nav macro call sites**

In every template that calls `nav::nav(...)`, add `csrf_token` as the third argument:

`templates/dashboard.html` line 4:
```
{% block nav %}{% call nav::nav("dashboard", user, csrf_token) %}{% endcall %}{% endblock %}
```

`templates/status.html` line 4:
```
{% block nav %}{% call nav::nav("status", user, csrf_token) %}{% endcall %}{% endblock %}
```

`templates/queue.html` line 4:
```
{% block nav %}{% call nav::nav("queue", user, csrf_token) %}{% endcall %}{% endblock %}
```

`templates/mods/list.html` line 4:
```
{% block nav %}{% call nav::nav("mods", user, csrf_token) %}{% endcall %}{% endblock %}
```

`templates/mods/detail.html` line 4:
```
{% block nav %}{% call nav::nav("mods", user, csrf_token) %}{% endcall %}{% endblock %}
```

- [ ] **Step 7: Add hidden CSRF inputs to all remaining template forms**

In `templates/login.html`, inside the `<form>` (after line 9):
```html
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
```

In `templates/register.html`, inside the `<form>` (after line 9, before the existing hidden `code` input):
```html
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
```

In `templates/status.html`, add to each of the 3 server forms (start, restart, stop — lines 10, 13, 16):
```html
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
```

In `templates/queue.html`, add to the apply form (line 9) and the cancel form (line 46):
```html
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
```

In `templates/mods/list.html`, add to:
- update-all form (line 10)
- per-mod update form (line 42)
- per-mod remove form (line 45)
- install form (line 62)

```html
            <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
```

In `templates/mods/detail.html`, add to:
- update form (line 62)
- remove form (line 65)

```html
        <input type="hidden" name="csrf_token" value="{{ csrf_token }}">
```

- [ ] **Step 8: Add csrf_token to all template structs**

In `src/web/handlers/auth.rs`, add `csrf_token: String` to both template structs:

```rust
struct LoginTemplate {
    error: Option<String>,
    csrf_token: String,
}

struct RegisterTemplate {
    error: Option<String>,
    code: String,
    profiles: Vec<SptProfile>,
    csrf_token: String,
}
```

In `src/web/handlers/mods.rs`, add `csrf_token: String` to:

```rust
struct ModListTemplate {
    user: SessionUser,
    mods: Vec<ModListEntry>,
    csrf_token: String,
}

struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
    csrf_token: String,
}
```

In `src/web/handlers/queue.rs`, add `csrf_token: String`:

```rust
struct QueueTemplate {
    user: SessionUser,
    ops: Vec<PendingOperation>,
    csrf_token: String,
}
```

In `src/web/handlers/status.rs`, add `csrf_token: String`:

```rust
struct StatusPageTemplate {
    user: SessionUser,
    csrf_token: String,
}
```

In `src/web/handlers/dashboard.rs`, add `csrf_token: String`:

```rust
struct DashboardTemplate {
    user: SessionUser,
    mods: Vec<InstalledMod>,
    pending_count: usize,
    unmanaged_dirs: Vec<(String, usize)>,
    csrf_token: String,
}
```

- [ ] **Step 9: Add csrf_token to LoginForm and RegisterForm, add csrf_token to InstallForm**

In `src/web/handlers/auth.rs`:

```rust
#[derive(serde::Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterForm {
    code: String,
    profile_id: String,
    password: String,
    password_confirm: String,
    csrf_token: String,
}
```

In `src/web/handlers/mods.rs`:

```rust
#[derive(serde::Deserialize)]
pub struct InstallForm {
    mod_ref: String,
    csrf_token: String,
}
```

- [ ] **Step 10: Update `render_register_error` to accept and pass csrf_token**

In `src/web/handlers/auth.rs`, update the helper function signature and struct construction:

```rust
fn render_register_error(
    msg: &str,
    code: String,
    profiles: Vec<SptProfile>,
    csrf_token: String,
) -> actix_web::Result<HttpResponse> {
    let tmpl = RegisterTemplate {
        error: Some(msg.to_string()),
        code,
        profiles,
        csrf_token,
    };
    Ok(HttpResponse::BadRequest()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 11: Update `login_page` — add Session parameter and generate token**

In `src/web/handlers/auth.rs`, `login_page` currently takes no parameters. Add `Session`:

```rust
pub async fn login_page(session: Session) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let tmpl = LoginTemplate { error: None, csrf_token };
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 12: Update `login_submit` — validate CSRF and pass token to error renders**

In `src/web/handlers/auth.rs`, add CSRF validation at the top of `login_submit` (after extracting the form), and pass `csrf_token` to all error template constructions:

```rust
pub async fn login_submit(
    form: Form<LoginForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();
    let username = form.username.clone();

    let user = web::block(move || {
        let db = db.lock();
        db.get_user_by_username(&username)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let user = match user {
        Some(u) => u,
        None => {
            let tmpl = LoginTemplate {
                error: Some("Invalid username or password".to_string()),
                csrf_token,
            };
            return Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?));
        }
    };

    let valid = match user.password_hash {
        Some(ref hash) => {
            let password = form.password.clone();
            let hash = hash.clone();
            web::block(move || verify_password(&password, &hash))
                .await
                .map_err(WebError::from)?
        }
        None => false,
    };

    if !valid {
        let tmpl = LoginTemplate {
            error: Some("Invalid username or password".to_string()),
            csrf_token,
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    session.renew();
    let session_user = SessionUser {
        user_id: user.id,
        username: user.username,
        role: user.role,
    };
    set_session_user(&session, &session_user).map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/"))
        .finish())
}
```

- [ ] **Step 13: Update `register_page` — add Session parameter and pass token to all render paths**

In `src/web/handlers/auth.rs`, add `Session` parameter to `register_page` and pass `csrf_token` to every `RegisterTemplate` construction (there are 5 in this function — empty code error, invalid code, used code, expired code, and success):

```rust
pub async fn register_page(
    query: Query<RegisterQuery>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let code = query.code.clone().unwrap_or_default();

    if code.is_empty() {
        let tmpl = RegisterTemplate {
            error: Some("Invite code required".to_string()),
            code: String::new(),
            profiles: vec![],
            csrf_token,
        };
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let db = state.db.clone();
    let code_check = code.clone();
    let invite = web::block(move || {
        let db = db.lock();
        db.get_invite(&code_check)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match invite {
        None => {
            let tmpl = RegisterTemplate {
                error: Some("Invalid invite code".to_string()),
                code,
                profiles: vec![],
                csrf_token,
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(inv) if inv.used_by.is_some() => {
            let tmpl = RegisterTemplate {
                error: Some("This invite code has already been used".to_string()),
                code,
                profiles: vec![],
                csrf_token,
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(ref inv) if is_invite_expired(inv.expires_at.as_deref()) => {
            let tmpl = RegisterTemplate {
                error: Some("This invite code has expired".to_string()),
                code,
                profiles: vec![],
                csrf_token,
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(_) => {
            let spt_dir = state.spt_dir.clone();
            let profiles = web::block(move || list_profiles(&spt_dir))
                .await
                .map_err(WebError::from)?
                .unwrap_or_default();
            let tmpl = RegisterTemplate {
                error: None,
                code,
                profiles,
                csrf_token,
            };
            Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
    }
}
```

- [ ] **Step 14: Update `register_submit` — validate CSRF and pass token to error renders**

In `src/web/handlers/auth.rs`, add CSRF validation at the top of `register_submit`. Change `_session` to `session`. Pass `csrf_token` to all `render_register_error` calls:

```rust
pub async fn register_submit(
    form: Form<RegisterForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let spt_dir = state.spt_dir.clone();
    let profiles = web::block(move || list_profiles(&spt_dir))
        .await
        .map_err(WebError::from)?
        .unwrap_or_default();

    if form.password.len() < MIN_PASSWORD_LEN {
        return render_register_error(
            &format!("Password must be at least {MIN_PASSWORD_LEN} characters"),
            form.code,
            profiles,
            csrf_token,
        );
    }

    if form.password.len() > MAX_PASSWORD_LEN {
        return render_register_error(
            &format!("Password must be at most {MAX_PASSWORD_LEN} characters"),
            form.code,
            profiles,
            csrf_token,
        );
    }

    if form.password != form.password_confirm {
        return render_register_error("Passwords do not match", form.code, profiles, csrf_token);
    }

    if form.profile_id.is_empty() {
        return render_register_error("Please select your SPT profile", form.code, profiles, csrf_token);
    }

    // ... rest of function unchanged (success path doesn't render a template with csrf_token)
```

- [ ] **Step 15: Update `logout` handler — add CsrfForm and validate**

In `src/web/handlers/auth.rs`:

```rust
pub async fn logout(session: Session, form: Form<crate::web::csrf::CsrfForm>) -> HttpResponse {
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return HttpResponse::Forbidden().body("forbidden");
    }
    session.purge();
    HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish()
}
```

- [ ] **Step 16: Update GET handlers to generate and pass csrf_token**

In `src/web/handlers/mods.rs`, update `list_mods` (line 90) and `mod_detail` (line 121):

```rust
// In list_mods, after require_admin:
let csrf_token = crate::web::csrf::get_or_create_token(&session);
// ... existing db query code ...
let tmpl = ModListTemplate { user, mods, csrf_token };
```

```rust
// In mod_detail, after require_auth:
let csrf_token = crate::web::csrf::get_or_create_token(&session);
// ... existing db query code ...
let tmpl = ModDetailTemplate { user, mod_info, files, dependencies, csrf_token };
```

In `src/web/handlers/queue.rs`, update `queue_page` (line 30):

```rust
let csrf_token = crate::web::csrf::get_or_create_token(&session);
// ... existing db query code ...
let tmpl = QueueTemplate { user, ops, csrf_token };
```

In `src/web/handlers/status.rs`, update `status_page` (line 24):

```rust
let csrf_token = crate::web::csrf::get_or_create_token(&session);
let tmpl = StatusPageTemplate { user, csrf_token };
```

In `src/web/handlers/dashboard.rs`, update `dashboard` (line 38):

```rust
let csrf_token = crate::web::csrf::get_or_create_token(&session);
// ... existing db query code ...
let tmpl = DashboardTemplate { user, mods, pending_count, unmanaged_dirs, csrf_token };
```

- [ ] **Step 17: Add CsrfForm + validation to POST handlers with no existing form body**

For handlers that currently take no form body, add `Form<crate::web::csrf::CsrfForm>` as a parameter and validate at the top.

In `src/web/handlers/mods.rs`:

```rust
pub async fn update_mod(
    state: Data<AppState>,
    path: Path<i64>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    // ... rest unchanged
```

Apply the same pattern to `remove_mod` and `update_all_mods` in `handlers/mods.rs`.

In `src/web/handlers/queue.rs`, apply to `cancel_op` and `apply_queue`.

In `src/web/handlers/server.rs`, apply to `start_server`, `stop_server`, and `restart_server`. Add `use actix_web::web::Form;` to imports.

- [ ] **Step 18: Add CSRF validation to `install_mod`**

In `src/web/handlers/mods.rs`, `install_mod` already takes `Form<InstallForm>`. Add validation after the `require_admin` check:

```rust
pub async fn install_mod(
    form: Form<InstallForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    // ... rest unchanged
```

- [ ] **Step 19: Build and run tests**

Run: `cargo build 2>&1 | head -40 && cargo test 2>&1 | tail -30`
Expected: clean compile, all tests pass

- [ ] **Step 20: Manual verification**

Start the dev server (`cargo run -- serve`) and verify:
1. Login form submits successfully (inspect form in browser — csrf_token hidden input present)
2. Register page renders with csrf_token
3. Mod install/update/remove works from the mods page
4. Server start/stop/restart works from the status page
5. Queue apply/cancel works
6. Logout works from the nav bar
7. All pages render without errors (dashboard, mods, queue, status, mod detail)

- [ ] **Step 21: Commit**

```bash
git add src/web/csrf.rs src/web/mod.rs src/web/handlers/ templates/
git commit -m "feat: add CSRF token validation on all POST forms"
```
