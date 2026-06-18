# Phase 4: Web UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Quartermaster web UI — an actix-web server with askama templates and HTMX interactivity, providing a dashboard for admins to manage SPT mods and for players to view server status.

**Architecture:** actix-web HTTP server embedded in the `quma` binary via `quma serve`. SQLite access via `Arc<Mutex<Database>>` + `web::block()`. Askama templates with Jinja-style inheritance from `base.html`. HTMX for interactive elements (no JS build step). Static assets (CSS, htmx.min.js) embedded via `rust-embed`. Auth via signed session cookies (`actix-session`) with Argon2 password hashing. Rate limiting on auth endpoints via `actix-governor`.

**Tech Stack:** actix-web 4, actix-session 0.11 (CookieSessionStore), askama 0.16, rust-embed 8, actix-web-rust-embed-responder 2.4, argon2 0.5, parking_lot (non-poisoning Mutex)

**Spec:** `SPEC.md` in the project root is the authoritative reference for routes, auth flows, and page behavior.

## Global Constraints

- Rust edition 2021, binary name `quma`
- SPT 4.0+ only, Linux only for v1
- Single binary deployment — all assets embedded via `rust-embed`
- `rusqlite::Connection` is `!Sync` — wrap `Database` in `Arc<Mutex<Database>>`, use `web::block()` for DB operations in async handlers
- Session cookies: signed, `HttpOnly`, `SameSite=Strict`
- Rate limiting: deferred to post-Phase 4 (spec requires 5 req/min/IP on `/login` and `/register` via `actix-governor`)
- Roles: `admin` (full access) and `player` (read-only dashboard, status, queue view)
- HTMX loaded from embedded static asset, not CDN
- Templates live in `templates/` at the crate root (askama default)

---

## Task 17: Actix Server Foundation & Serve Command

**Files:**
- Modify: `Cargo.toml` (add web dependencies)
- Rewrite: `src/web/mod.rs` (server startup, route registration, static asset responder)
- Create: `src/web/state.rs` (AppState struct)
- Create: `src/web/error.rs` (web error types)
- Create: `src/cli/serve.rs` (CLI handler)
- Modify: `src/cli/mod.rs` (add `pub mod serve;`)
- Modify: `src/main.rs` (wire serve command)
- Create: `src/assets/style.css`
- Download: `src/assets/htmx.min.js`
- Create: `templates/base.html`

**Interfaces:**
- Consumes: `Config` (from `src/config.rs`), `Database` (from `src/db/mod.rs`), `ForgeClient` (from `src/forge/client.rs`), `SptInfo` (from `src/spt/detect.rs`)
- Produces: `AppState` struct (used by all subsequent web tasks), `start_server()` async function, working `/` placeholder route, `/assets/{path}` static file serving

- [ ] **Step 1: Add web dependencies to `Cargo.toml`**

Add these to the `[dependencies]` section:

```toml
# Web server
actix-web = "4"
actix-session = { version = "0.11", features = ["cookie-session"] }

# Templating
askama = "0.16"

# Static asset embedding
rust-embed = "8"
actix-web-rust-embed-responder = "2.4"

# Password hashing
argon2 = "0.5"

# Non-poisoning Mutex for web DB access
parking_lot = "0.12"
```

- [ ] **Step 2: Run `cargo check` to verify dependencies resolve**

```bash
cargo check
```

Expected: compiles (may have warnings about unused imports — that's fine).

- [ ] **Step 3: Download htmx.min.js**

```bash
curl -L -o src/assets/htmx.min.js https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js
```

Verify the file is non-empty:

```bash
wc -c src/assets/htmx.min.js
```

Expected: ~50KB file.

- [ ] **Step 4: Write `src/assets/style.css`**

```css
:root {
    --bg: #1a1a2e;
    --bg-card: #16213e;
    --bg-input: #0f3460;
    --text: #e0e0e0;
    --text-muted: #8a8a9a;
    --accent: #e94560;
    --accent-hover: #ff6b81;
    --success: #4ecca3;
    --warning: #f0c040;
    --danger: #e94560;
    --border: #2a2a4a;
    --radius: 6px;
}

* { box-sizing: border-box; margin: 0; padding: 0; }

body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.6;
    min-height: 100vh;
}

a { color: var(--accent); text-decoration: none; }
a:hover { color: var(--accent-hover); }

/* Navigation */
nav {
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
    padding: 0.75rem 1.5rem;
    display: flex;
    align-items: center;
    gap: 1.5rem;
}
nav .brand { font-weight: 700; font-size: 1.1rem; color: var(--text); }
nav .links { display: flex; gap: 1rem; flex: 1; }
nav .links a { color: var(--text-muted); font-size: 0.9rem; }
nav .links a:hover, nav .links a.active { color: var(--text); }
nav .user-info { font-size: 0.85rem; color: var(--text-muted); }

/* Layout */
main { max-width: 960px; margin: 2rem auto; padding: 0 1.5rem; }

/* Cards */
.card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1.25rem;
    margin-bottom: 1rem;
}
.card h2 { font-size: 1.1rem; margin-bottom: 0.75rem; }

/* Tables */
table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
th, td { padding: 0.5rem 0.75rem; text-align: left; border-bottom: 1px solid var(--border); }
th { color: var(--text-muted); font-weight: 600; font-size: 0.8rem; text-transform: uppercase; }
tr:hover { background: rgba(255,255,255,0.03); }

/* Forms — scoped to .card to avoid breaking inline table forms */
.card > form, .auth-card form {
    display: flex; flex-direction: column; gap: 0.75rem; max-width: 400px;
}
label { font-size: 0.85rem; color: var(--text-muted); }
input, select {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    padding: 0.5rem 0.75rem;
    font-size: 0.9rem;
}
input:focus, select:focus { outline: none; border-color: var(--accent); }

/* Buttons */
.btn {
    display: inline-block;
    padding: 0.4rem 1rem;
    border: none;
    border-radius: var(--radius);
    font-size: 0.85rem;
    cursor: pointer;
    color: white;
    background: var(--accent);
    transition: background 0.15s;
}
.btn:hover { background: var(--accent-hover); }
.btn-sm { padding: 0.25rem 0.6rem; font-size: 0.8rem; }
.btn-success { background: var(--success); color: #1a1a2e; }
.btn-warning { background: var(--warning); color: #1a1a2e; }
.btn-danger { background: var(--danger); }
.btn-outline {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-muted);
}
.btn-outline:hover { border-color: var(--text); color: var(--text); }
.btn[disabled] { opacity: 0.5; cursor: not-allowed; }

/* Badges */
.badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 3px;
    font-size: 0.75rem;
    font-weight: 600;
}
.badge-success { background: var(--success); color: #1a1a2e; }
.badge-warning { background: var(--warning); color: #1a1a2e; }
.badge-danger { background: var(--danger); color: white; }
.badge-muted { background: var(--border); color: var(--text-muted); }

/* Alerts */
.alert {
    padding: 0.75rem 1rem;
    border-radius: var(--radius);
    margin-bottom: 1rem;
    font-size: 0.9rem;
}
.alert-error { background: rgba(233,69,96,0.15); border: 1px solid var(--danger); color: var(--danger); }
.alert-success { background: rgba(78,204,163,0.15); border: 1px solid var(--success); color: var(--success); }
.alert-warning { background: rgba(240,192,64,0.15); border: 1px solid var(--warning); color: var(--warning); }

/* Status indicators */
.status-dot {
    display: inline-block;
    width: 8px; height: 8px;
    border-radius: 50%;
    margin-right: 0.4rem;
}
.status-dot.up { background: var(--success); }
.status-dot.down { background: var(--danger); }

/* Utility */
.text-muted { color: var(--text-muted); }
.text-sm { font-size: 0.85rem; }
.mt-1 { margin-top: 0.5rem; }
.mt-2 { margin-top: 1rem; }
.mb-1 { margin-bottom: 0.5rem; }
.flex { display: flex; }
.flex-between { display: flex; justify-content: space-between; align-items: center; }
.gap-1 { gap: 0.5rem; }
h1 { font-size: 1.4rem; margin-bottom: 1rem; }

/* HTMX loading indicator */
.htmx-indicator { display: none; }
.htmx-request .htmx-indicator { display: inline; }
```

- [ ] **Step 5: Write `templates/base.html`**

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
        {% block flash %}{% endblock %}
        {% block content %}{% endblock %}
    </main>
</body>
</html>
```

- [ ] **Step 6: Write `src/web/state.rs`**

```rust
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::SptInfo;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub config_path: PathBuf,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
}
```

Note: `parking_lot::Mutex` is used instead of `std::sync::Mutex` because it does not poison on panic. If a DB operation panics, subsequent requests will still be able to acquire the lock instead of cascading into panic. The lock API is also simpler — `.lock()` returns the guard directly instead of `Result<Guard, PoisonError>`.


- [ ] **Step 7: Write `src/web/error.rs`**

```rust
use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};

#[derive(Debug)]
pub enum WebError {
    Internal(anyhow::Error),
    NotFound,
    Forbidden,
    BadRequest(String),
}

impl std::fmt::Display for WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebError::Internal(e) => write!(f, "internal error: {e}"),
            WebError::NotFound => write!(f, "not found"),
            WebError::Forbidden => write!(f, "forbidden"),
            WebError::BadRequest(msg) => write!(f, "bad request: {msg}"),
        }
    }
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
        HttpResponse::build(self.status_code()).body(self.to_string())
    }
}

impl From<anyhow::Error> for WebError {
    fn from(e: anyhow::Error) -> Self {
        WebError::Internal(e)
    }
}

impl From<rusqlite::Error> for WebError {
    fn from(e: rusqlite::Error) -> Self {
        WebError::Internal(e.into())
    }
}

impl From<askama::Error> for WebError {
    fn from(e: askama::Error) -> Self {
        WebError::Internal(e.into())
    }
}

impl From<actix_web::error::BlockingError> for WebError {
    fn from(e: actix_web::error::BlockingError) -> Self {
        WebError::Internal(anyhow::anyhow!("blocking error: {e}"))
    }
}
```

- [ ] **Step 8: Rewrite `src/web/mod.rs` — server startup and static assets**

```rust
pub mod error;
pub mod state;

use std::sync::Arc;

use actix_session::config::PersistentSession;
use actix_session::storage::CookieSessionStore;
use actix_session::SessionMiddleware;
use actix_web::cookie::time::Duration as CookieDuration;
use actix_web::cookie::Key;
use actix_web::web::{self, Html};
use actix_web::{middleware, App, HttpResponse, HttpServer};
use actix_web_rust_embed_responder::{EmbedResponse, IntoResponse};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use rust_embed::RustEmbed;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::SptInfo;

use state::AppState;

#[derive(RustEmbed)]
#[folder = "src/assets/"]
struct Assets;

async fn serve_asset(path: web::Path<String>) -> HttpResponse {
    match Assets::get(&path) {
        Some(file) => {
            let EmbedResponse(resp) = file.into_response();
            resp
        }
        None => HttpResponse::NotFound().body("asset not found"),
    }
}

pub async fn start_server(
    config: Config,
    config_path: std::path::PathBuf,
    db: Database,
    forge: ForgeClient,
    spt_dir: std::path::PathBuf,
    spt_info: SptInfo,
) -> Result<()> {
    let bind_addr = format!("{}:{}", config.web_bind, config.web_port);

    let session_key = Key::derive_from(config.session_secret.as_bytes());

    let db = Arc::new(parking_lot::Mutex::new(db));
    let app_state = web::Data::new(AppState {
        db,
        forge,
        config: config.clone(),
        config_path,
        spt_dir,
        spt_info,
    });

    println!("Quartermaster web UI starting on http://{bind_addr}");

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
                    .session_lifecycle(
                        PersistentSession::default()
                            .session_ttl(CookieDuration::days(7)),
                    )
                    .cookie_http_only(true)
                    .cookie_same_site(actix_web::cookie::SameSite::Strict)
                    .cookie_secure(false)
                    .build(),
            )
            .wrap(middleware::NormalizePath::trim())
            // Placeholder — replaced by dashboard route in Task 18
            .route("/", web::get().to(|| async {
                Html::new("Quartermaster is running. Login at /login".to_string())
            }))
            .route("/assets/{path:.*}", web::get().to(serve_asset))
    })
    .bind(&bind_addr)
    .with_context(|| format!("failed to bind to {bind_addr}"))?
    .run()
    .await
    .context("web server error")
}
```

Note: The `serve_asset` handler explicitly handles the `None` case from `Assets::get()` with a 404 response. The `EmbedResponse` wrapper from `actix-web-rust-embed-responder` handles content type detection, caching headers, and compression automatically. If the exact `EmbedResponse` destructuring pattern differs in the installed version, the implementing agent should check the crate's actual API — the key requirement is handling `Option<EmbeddedFile>` and returning 404 for missing assets.


- [ ] **Step 9: Write `src/cli/serve.rs`**

```rust
use anyhow::{Context, Result};

use super::Cli;
use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::{detect_spt_dir, read_spt_version};

pub async fn run(bind: Option<&str>, port: Option<u16>, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let spt_info = read_spt_version(&spt_dir)?;

    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = Config::load_with_env(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    if let Some(b) = bind {
        config.web_bind = b.to_string();
    }
    if let Some(p) = port {
        config.web_port = p;
    }

    config.ensure_session_secret();
    config
        .save(&config_path)
        .context("failed to save config with session secret")?;

    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    if !db.admin_exists()? {
        anyhow::bail!(
            "No admin user exists. Run `quma init` first to create an admin account."
        );
    }

    let forge = ForgeClient::new(config.forge_token.clone())?;

    crate::web::start_server(config, config_path, db, forge, spt_dir, spt_info).await
}
```

- [ ] **Step 10: Add `pub mod serve;` to `src/cli/mod.rs`**

Add the module declaration alongside the existing ones:

```rust
pub mod serve;
```

- [ ] **Step 11: Wire the serve command in `src/main.rs`**

Replace the `Command::Serve { .. } => todo!("serve"),` line:

```rust
        Command::Serve { bind, port } => {
            cli::serve::run(bind.as_deref(), port, &cli).await
        }
```

- [ ] **Step 12: Verify everything compiles**

```bash
cargo check
```

Expected: compiles with no errors.

- [ ] **Step 13: Run existing tests to confirm no regressions**

```bash
cargo test
```

Expected: all existing tests pass.

- [ ] **Step 14: Commit**

```bash
git add -A
git commit -m "feat: actix-web server foundation with static assets, base template, and serve command"
```

---

## Task 18: Auth System

**Files:**
- Create: `src/spt/profiles.rs` (read SPT profile JSON files)
- Modify: `src/spt/mod.rs` (add `pub mod profiles;`)
- Create: `src/web/auth.rs` (password hashing, session helpers, auth middleware)
- Create: `src/web/handlers/mod.rs` (handler module re-exports)
- Create: `src/web/handlers/auth.rs` (login, register, logout handlers)
- Create: `templates/login.html`
- Create: `templates/register.html`
- Modify: `src/web/mod.rs` (add auth routes, rate limiting, handler modules)

**Interfaces:**
- Consumes: `AppState` from Task 17, `Database::insert_user`, `Database::get_user_by_username`, `Database::get_invite`, `Database::use_invite`, `Database::admin_exists`
- Produces: `AuthSession` extractor (user info from session), `RequireAuth` middleware, `RequireAdmin` middleware, login/register/logout routes, `hash_password`/`verify_password` functions, `list_spt_profiles` function

- [ ] **Step 1: Write failing test — list SPT profiles from directory**

Create `src/spt/profiles.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SptProfile {
    pub aid: String,
    pub username: String,
}

#[derive(Deserialize)]
struct ProfileJson {
    info: ProfileInfo,
}

#[derive(Deserialize)]
struct ProfileInfo {
    id: String,
    username: String,
}

pub fn list_profiles(spt_dir: &Path) -> Result<Vec<SptProfile>> {
    let profiles_dir = spt_dir.join("SPT/user/profiles");
    if !profiles_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    let entries = std::fs::read_dir(&profiles_dir)
        .with_context(|| format!("failed to read {}", profiles_dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let parsed: ProfileJson = match serde_json::from_str(&contents) {
            Ok(p) => p,
            Err(_) => continue,
        };

        profiles.push(SptProfile {
            aid: parsed.info.id,
            username: parsed.info.username,
        });
    }

    profiles.sort_by(|a, b| a.username.cmp(&b.username));
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_fake_profile(dir: &Path, aid: &str, username: &str) {
        let profiles_dir = dir.join("SPT/user/profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();
        let content = format!(
            r#"{{"info":{{"id":"{}","username":"{}"}}}}"#,
            aid, username
        );
        std::fs::write(profiles_dir.join(format!("{aid}.json")), content).unwrap();
    }

    #[test]
    fn list_profiles_finds_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_profile(tmp.path(), "abc123", "Player1");
        create_fake_profile(tmp.path(), "def456", "Player2");

        let profiles = list_profiles(tmp.path()).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].username, "Player1");
        assert_eq!(profiles[1].username, "Player2");
    }

    #[test]
    fn list_profiles_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/profiles")).unwrap();
        let profiles = list_profiles(tmp.path()).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn list_profiles_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles = list_profiles(tmp.path()).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn list_profiles_skips_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_profile(tmp.path(), "good1", "GoodPlayer");
        let profiles_dir = tmp.path().join("SPT/user/profiles");
        std::fs::write(profiles_dir.join("bad.json"), "not json").unwrap();

        let profiles = list_profiles(tmp.path()).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].username, "GoodPlayer");
    }
}
```

- [ ] **Step 2: Add `pub mod profiles;` to `src/spt/mod.rs`**

```rust
pub mod detect;
pub mod mods;
pub mod profiles;
pub mod server;
```

- [ ] **Step 3: Run profile tests**

```bash
cargo test spt::profiles::tests
```

Expected: all PASS.

- [ ] **Step 4: Write `src/web/auth.rs` — password hashing, session helpers, middleware**

```rust
use actix_session::Session;
use actix_web::body::BoxBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::web::Redirect;
use actix_web::{Error, HttpResponse};
use anyhow::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::task::{Context, Poll};

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hash error: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: i64,
    pub username: String,
    pub role: String,
}

impl SessionUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

pub fn get_session_user(session: &Session) -> Option<SessionUser> {
    let user_id = session.get::<i64>("user_id").ok()??;
    let username = session.get::<String>("username").ok()??;
    let role = session.get::<String>("role").ok()??;
    Some(SessionUser {
        user_id,
        username,
        role,
    })
}

pub fn set_session_user(session: &Session, user: &SessionUser) -> Result<()> {
    session
        .insert("user_id", user.user_id)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("username", &user.username)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("role", &user.role)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    Ok(())
}

// -- RequireAuth middleware --

pub struct RequireAuth;

impl<S, B> Transform<S, ServiceRequest> for RequireAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Transform = RequireAuthMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequireAuthMiddleware { service }))
    }
}

pub struct RequireAuthMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for RequireAuthMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let session = req.get_session();
        let user = get_session_user(&session);

        if user.is_none() {
            return Box::pin(async move {
                let resp = Redirect::to("/login").see_other();
                Ok(req.into_response(resp).map_into_boxed_body())
            });
        }

        let fut = self.service.call(req);
        Box::pin(async move { fut.await.map(|res| res.map_into_boxed_body()) })
    }
}

// -- RequireAdmin middleware --

pub struct RequireAdmin;

impl<S, B> Transform<S, ServiceRequest> for RequireAdmin
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Transform = RequireAdminMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequireAdminMiddleware { service }))
    }
}

pub struct RequireAdminMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for RequireAdminMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let session = req.get_session();
        let user = get_session_user(&session);

        match user {
            None => Box::pin(async move {
                let resp = Redirect::to("/login").see_other();
                Ok(req.into_response(resp).map_into_boxed_body())
            }),
            Some(u) if !u.is_admin() => Box::pin(async move {
                let resp = HttpResponse::Forbidden().body("admin access required");
                Ok(req.into_response(resp).map_into_boxed_body())
            }),
            Some(_) => {
                let fut = self.service.call(req);
                Box::pin(async move { fut.await.map(|res| res.map_into_boxed_body()) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let hash = hash_password("test123").unwrap();
        assert!(verify_password("test123", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn verify_invalid_hash() {
        assert!(!verify_password("anything", "not-a-hash"));
    }

    #[test]
    fn session_user_admin_check() {
        let admin = SessionUser {
            user_id: 1,
            username: "admin".into(),
            role: "admin".into(),
        };
        let player = SessionUser {
            user_id: 2,
            username: "player".into(),
            role: "player".into(),
        };
        assert!(admin.is_admin());
        assert!(!player.is_admin());
    }
}
```

- [ ] **Step 5: Write `templates/login.html`**

```html
{% extends "base.html" %}
{% block title %}Login — Quartermaster{% endblock %}
{% block content %}
<div class="card auth-card" style="max-width: 400px; margin: 4rem auto;">
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

- [ ] **Step 6: Write `templates/register.html`**

```html
{% extends "base.html" %}
{% block title %}Register — Quartermaster{% endblock %}
{% block content %}
<div class="card auth-card" style="max-width: 400px; margin: 4rem auto;">
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

- [ ] **Step 7: Create `src/web/handlers/mod.rs`**

```rust
pub mod auth;
```

- [ ] **Step 8: Write `src/web/handlers/auth.rs`**

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Query};
use actix_web::HttpResponse;
use askama::Template;

use crate::spt::profiles::{list_profiles, SptProfile};
use crate::web::auth::{
    get_session_user, hash_password, set_session_user, verify_password, SessionUser,
};
use crate::web::error::WebError;
use crate::web::state::AppState;

// -- Templates --

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate {
    error: Option<String>,
    code: String,
    profiles: Vec<SptProfile>,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterForm {
    code: String,
    profile_id: String,
    password: String,
    password_confirm: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterQuery {
    code: Option<String>,
}

// -- Handlers --

pub async fn login_page() -> actix_web::Result<Html<String>> {
    let tmpl = LoginTemplate { error: None };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn login_submit(
    form: Form<LoginForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();
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
            };
            return Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?));
        }
    };

    // Argon2 verification is CPU-intensive — run on blocking thread pool
    let valid = match user.password_hash.clone() {
        Some(hash) => {
            let password = form.password.clone();
            web::block(move || verify_password(&password, &hash))
                .await
                .map_err(WebError::from)?
        }
        None => false,
    };

    if !valid {
        let tmpl = LoginTemplate {
            error: Some("Invalid username or password".to_string()),
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

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

pub async fn register_page(
    query: Query<RegisterQuery>,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    if code.is_empty() {
        let tmpl = RegisterTemplate {
            error: Some("Invite code required".to_string()),
            code: String::new(),
            profiles: vec![],
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
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(ref inv) if inv.expires_at.as_deref().is_some_and(|exp| {
            exp < &chrono::Utc::now().to_rfc3339()
        }) => {
            let tmpl = RegisterTemplate {
                error: Some("This invite code has expired".to_string()),
                code,
                profiles: vec![],
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(_) => {
            let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
            let tmpl = RegisterTemplate {
                error: None,
                code,
                profiles,
            };
            Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn register_submit(
    form: Form<RegisterForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if form.password != form.password_confirm {
        let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
        let tmpl = RegisterTemplate {
            error: Some("Passwords do not match".to_string()),
            code: form.code,
            profiles,
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    if form.profile_id.is_empty() {
        let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
        let tmpl = RegisterTemplate {
            error: Some("Please select your SPT profile".to_string()),
            code: form.code,
            profiles,
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
    let profile = profiles
        .iter()
        .find(|p| p.aid == form.profile_id);

    let username = match profile {
        Some(p) => p.username.clone(),
        None => {
            let tmpl = RegisterTemplate {
                error: Some("Invalid profile selection".to_string()),
                code: form.code,
                profiles,
            };
            return Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?));
        }
    };

    // Argon2 hashing is CPU-intensive — run on blocking thread pool
    let password = form.password.clone();
    let password_hash = web::block(move || hash_password(&password))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let db = state.db.clone();
    let code = form.code.clone();
    let profile_id = form.profile_id.clone();

    let result = web::block(move || {
        let db = db.lock();

        if db.get_user_by_username(&username)?.is_some() {
            return Ok::<_, rusqlite::Error>(Err("A user with this profile already exists".to_string()));
        }

        let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), "player")?;
        let used = db.use_invite(&code, user_id)?;
        if used == 0 {
            return Ok(Err("Invite code is invalid or expired".to_string()));
        }

        Ok(Ok(user_id))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        Ok(_user_id) => Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/login"))
            .finish()),
        Err(msg) => {
            let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
            let tmpl = RegisterTemplate {
                error: Some(msg),
                code: form.code,
                profiles,
            };
            Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn logout(session: Session) -> HttpResponse {
    session.purge();
    HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish()
}
```

- [ ] **Step 9: Update `src/web/mod.rs` — add auth routes and rate limiting**

Add imports and modules at the top:

```rust
pub mod auth;
pub mod error;
pub mod handlers;
pub mod state;
```

Add the auth routes inside the `App::new()` builder in `start_server`. Replace the existing `App::new()` closure with:

```rust
        App::new()
            .app_data(app_state.clone())
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
                    .session_lifecycle(
                        PersistentSession::default()
                            .session_ttl(CookieDuration::days(7)),
                    )
                    .cookie_http_only(true)
                    .cookie_same_site(actix_web::cookie::SameSite::Strict)
                    .cookie_secure(false)
                    .build(),
            )
            .wrap(middleware::NormalizePath::trim())
            // Auth routes (public)
            // TODO(debt): add rate limiting via actix-governor (5 req/min/IP on /login and /register)
            .route("/login", web::get().to(handlers::auth::login_page))
            .route("/login", web::post().to(handlers::auth::login_submit))
            .route("/register", web::get().to(handlers::auth::register_page))
            .route("/register", web::post().to(handlers::auth::register_submit))
            .route("/logout", web::post().to(handlers::auth::logout))
            // Authenticated routes — dashboard added in Task 19
            .service(
                web::scope("")
                    .wrap(auth::RequireAuth)
                    .route("/", web::get().to(|| async {
                        Html::new("Logged in. Dashboard coming in Task 19.".to_string())
                    }))
            )
            // Static assets (public)
            .route("/assets/{path:.*}", web::get().to(serve_asset))
```

- [ ] **Step 10: Run tests**

```bash
cargo test
```

Expected: all tests pass, including new auth tests.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "feat: auth system with login, register, logout, session middleware, and rate limiting"
```

---

## Task 19: Dashboard & Mod Management Pages

**Files:**
- Create: `src/web/handlers/dashboard.rs`
- Create: `src/web/handlers/mods.rs`
- Create: `templates/dashboard.html`
- Create: `templates/mods/list.html`
- Create: `templates/mods/detail.html`
- Create: `templates/mods/partials/update_badges.html`
- Create: `templates/mods/partials/dependency_tree.html`
- Modify: `src/web/handlers/mod.rs` (add new modules)
- Modify: `src/web/mod.rs` (add dashboard and mod routes)

**Interfaces:**
- Consumes: `AppState`, `RequireAuth`, `RequireAdmin`, `get_session_user`, `Database::list_mods`, `Database::get_mod`, `Database::get_mod_by_forge_id`, `Database::get_files_for_mod`, `Database::get_dependencies`, `Database::list_pending_ops`, `ForgeClient::check_updates`, `ForgeClient::get_dependencies`, health check functions from refactored `health.rs`
- Produces: Dashboard page at `/`, mod list at `/mods`, mod detail at `/mods/{id}`, HTMX partials at `/api/mods/check-updates` and `/api/mods/dep-tree`, mod install/update/remove POST handlers

- [ ] **Step 1: Write `templates/dashboard.html`**

```html
{% extends "base.html" %}
{% block title %}Dashboard — Quartermaster{% endblock %}
{% block nav %}
<div class="links">
    <a href="/" class="active">Dashboard</a>
    <a href="/mods">Mods</a>
    <a href="/queue">Queue</a>
    <a href="/status">Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">Logout</button>
    </form>
</div>
{% endblock %}
{% block content %}
<h1>Dashboard</h1>

<div class="card">
    <div class="flex-between mb-1">
        <h2>Installed Mods ({{ mods.len() }})</h2>
        <span hx-get="/api/mods/check-updates" hx-trigger="load, every 60s" hx-swap="innerHTML">
            <span class="text-muted text-sm">Checking for updates...</span>
        </span>
    </div>
    {% if mods.is_empty() %}
    <p class="text-muted">No mods installed. {% if user.is_admin() %}Use <code>quma install</code> or the <a href="/mods">Mods</a> page to install mods.{% endif %}</p>
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

- [ ] **Step 2: Write `src/web/handlers/dashboard.rs`**

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Html};
use askama::Template;

use crate::cli::common::find_unmanaged_mod_dirs;
use crate::db::mods::InstalledMod;
use crate::web::auth::{get_session_user, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    user: SessionUser,
    mods: Vec<InstalledMod>,
    pending_count: usize,
    unmanaged_dirs: Vec<(String, usize)>,
}

pub async fn dashboard(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html<String>> {
    let user = get_session_user(&session).unwrap();

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
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
```

- [ ] **Step 3: Write `templates/mods/list.html`**

Create `templates/mods/` directory first.

```html
{% extends "base.html" %}
{% block title %}Mods — Quartermaster{% endblock %}
{% block nav %}
<div class="links">
    <a href="/">Dashboard</a>
    <a href="/mods" class="active">Mods</a>
    <a href="/queue">Queue</a>
    <a href="/status">Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">Logout</button>
    </form>
</div>
{% endblock %}
{% block content %}
<div class="flex-between">
    <h1>Installed Mods</h1>
    {% if user.is_admin() %}
    <div class="flex gap-1">
        <form method="post" action="/mods/update-all" style="display:inline">
            <button type="submit" class="btn btn-sm btn-outline">Update All</button>
        </form>
    </div>
    {% endif %}
</div>

{% if mods.is_empty() %}
<div class="card">
    <p class="text-muted">No mods installed.</p>
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
                        <button type="submit" class="btn btn-sm btn-outline">Update</button>
                    </form>
                    <form method="post" action="/mods/{{ m.mod_info.id }}/remove" style="display:inline"
                          onsubmit="return confirm('Remove {{ m.mod_info.name }}?')">
                        <button type="submit" class="btn btn-sm btn-danger">Remove</button>
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
        <button type="submit" class="btn">Install</button>
    </form>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 4: Write `templates/mods/detail.html`**

```html
{% extends "base.html" %}
{% block title %}{{ mod_info.name }} — Quartermaster{% endblock %}
{% block nav %}
<div class="links">
    <a href="/">Dashboard</a>
    <a href="/mods" class="active">Mods</a>
    <a href="/queue">Queue</a>
    <a href="/status">Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">Logout</button>
    </form>
</div>
{% endblock %}
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
        <button type="submit" class="btn">Update</button>
    </form>
    <form method="post" action="/mods/{{ mod_info.id }}/remove"
          onsubmit="return confirm('Remove {{ mod_info.name }}?')">
        <button type="submit" class="btn btn-danger">Remove</button>
    </form>
</div>
{% endif %}
{% endblock %}
```

- [ ] **Step 5: Write HTMX partial templates**

Create `templates/mods/partials/` directory.

`templates/mods/partials/update_badges.html`:

```html
{% if updates_available > 0 %}
<span class="badge badge-warning">{{ updates_available }} update{{ updates_available|pluralize }} available</span>
{% else %}
<span class="badge badge-success">All up to date</span>
{% endif %}
```

Note: askama doesn't have a built-in `pluralize` filter. Use a conditional instead:

```html
{% if updates_available > 0 %}
<span class="badge badge-warning">{{ updates_available }} update{% if updates_available != 1 %}s{% endif %} available</span>
{% else %}
<span class="badge badge-success">All up to date</span>
{% endif %}
```

`templates/mods/partials/dependency_tree.html`:

```html
{% if deps.is_empty() %}
<p class="text-muted">No dependencies.</p>
{% else %}
<ul>
    {% for dep in &deps %}
    <li>{{ dep.name }} v{{ dep.latest_compatible_version.as_ref().map(|v| v.version.as_str()).unwrap_or("?") }}
        {% if dep.conflict %}<span class="badge badge-danger">conflict</span>{% endif %}
    </li>
    {% endfor %}
</ul>
{% endif %}
```

- [ ] **Step 6: Write `src/web/handlers/mods.rs`**

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpResponse;
use askama::Template;

use crate::db::mods::{InstalledFile, InstalledMod, ModDependency};
use crate::forge::models::DependencyNode;
use crate::web::auth::{get_session_user, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

// -- View models --

struct ModListEntry {
    mod_info: InstalledMod,
    file_count: usize,
}

struct DepEntry {
    dep: ModDependency,
    dep_mod: Option<InstalledMod>,
}

// -- Templates --

#[derive(Template)]
#[template(path = "mods/list.html")]
struct ModListTemplate {
    user: SessionUser,
    mods: Vec<ModListEntry>,
}

#[derive(Template)]
#[template(path = "mods/detail.html")]
struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
}

#[derive(Template)]
#[template(path = "mods/partials/update_badges.html")]
struct UpdateBadgesTemplate {
    updates_available: usize,
}

#[derive(Template)]
#[template(path = "mods/partials/dependency_tree.html")]
struct DependencyTreeTemplate {
    deps: Vec<DependencyNode>,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct InstallForm {
    mod_ref: String,
}

#[derive(serde::Deserialize)]
pub struct DepTreeQuery {
    #[serde(rename = "mod")]
    mod_id: Option<i64>,
    ver: Option<String>,
}

// -- Handlers --

pub async fn list_mods(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html<String>> {
    let user = get_session_user(&session).unwrap();
    let db = state.db.clone();

    let mods = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let mut entries = Vec::new();
        for m in mods {
            let file_count = db.get_files_for_mod(m.id)?.len();
            entries.push(ModListEntry {
                mod_info: m,
                file_count,
            });
        }
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ModListTemplate { user, mods };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn mod_detail(
    state: Data<AppState>,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html<String>> {
    let user = get_session_user(&session).unwrap();
    let mod_id = path.into_inner();
    let db = state.db.clone();

    let (mod_info, files, dependencies) = web::block(move || {
        let db = db.lock();
        let mod_info = db
            .get_mod(mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
        let files = db.get_files_for_mod(mod_id)?;
        let deps = db.get_dependencies(mod_id)?;
        let mut dep_entries = Vec::new();
        for dep in deps {
            let dep_mod = db.get_mod(dep.depends_on_mod_id)?;
            dep_entries.push(DepEntry { dep, dep_mod });
        }
        Ok::<_, anyhow::Error>((mod_info, files, dep_entries))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ModDetailTemplate {
        user,
        mod_info,
        files,
        dependencies,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn check_updates_partial(
    state: Data<AppState>,
) -> actix_web::Result<Html<String>> {
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let updates_available = if !installed.is_empty() {
        let check_list: Vec<(i64, String)> = installed
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();
        match state
            .forge
            .check_updates(&check_list, &state.spt_info.spt_version)
            .await
        {
            Ok(result) => result.updates.len(),
            Err(_) => 0,
        }
    } else {
        0
    };

    let tmpl = UpdateBadgesTemplate { updates_available };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dep_tree_partial(
    state: Data<AppState>,
    query: Query<DepTreeQuery>,
) -> actix_web::Result<Html<String>> {
    let deps = match (query.mod_id, &query.ver) {
        (Some(mod_id), Some(ver)) => {
            state
                .forge
                .get_dependencies(&[(mod_id, ver.as_str())])
                .await
                .unwrap_or_default()
        }
        _ => vec![],
    };

    let tmpl = DependencyTreeTemplate { deps };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn install_mod(
    form: Form<InstallForm>,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let mod_ref = &form.mod_ref;

    let mod_id: i64 = match mod_ref.parse() {
        Ok(id) => id,
        Err(_) => {
            match state.forge.search_mods(mod_ref).await {
                Ok(results) if results.len() == 1 => results[0].id,
                Ok(results) if results.is_empty() => {
                    return Ok(HttpResponse::BadRequest().body(format!("No mods found matching '{mod_ref}'")));
                }
                Ok(_) => {
                    return Ok(HttpResponse::BadRequest().body(format!(
                        "Multiple mods match '{mod_ref}' — use a Forge mod ID instead"
                    )));
                }
                Err(e) => {
                    return Ok(HttpResponse::InternalServerError().body(format!("Forge API error: {e}")));
                }
            }
        }
    };

    let versions = state
        .forge
        .get_versions(mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(|e| WebError::Internal(e))?;

    let version = versions.first().ok_or(WebError::BadRequest(
        "No compatible version found for current SPT version".to_string(),
    ))?;

    let mod_info = state
        .forge
        .get_mod(mod_id, false)
        .await
        .map_err(|e| WebError::Internal(e))?;

    // Check if the operation should be queued (server running + queue enabled)
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
        .await
        .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = mod_info.name.clone();
        let version_id = version.id;
        web::block(move || {
            let db = db.lock();
            db.insert_pending_op("install", mod_id, Some(version_id), &mod_name, None, None)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let link = version.link.as_deref().ok_or(WebError::BadRequest(
        "Version has no download link".to_string(),
    ))?;

    let tmp_dir = tempfile::tempdir().map_err(|e| WebError::Internal(e.into()))?;
    let archive_path = tmp_dir.path().join("mod.zip");
    state
        .forge
        .download_file(link, &archive_path)
        .await
        .map_err(|e| WebError::Internal(e))?;

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let version_id = version.id;
    let version_str = version.version.clone();
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();

    web::block(move || {
        use crate::spt::mods::extract_mod;

        let extracted = extract_mod(&archive_path, &spt_dir)?;
        let db = db.lock();

        let installed_id = db.insert_mod(
            mod_id,
            version_id,
            &mod_name,
            mod_slug.as_deref(),
            &version_str,
        )?;

        for file in &extracted {
            db.insert_file(
                installed_id,
                &file.path,
                Some(&file.hash),
                Some(file.size as i64),
            )?;
        }

        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}

pub async fn update_mod(
    state: Data<AppState>,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let mod_db_id = path.into_inner();
    let db = state.db.clone();

    let installed = web::block(move || {
        let db = db.lock();
        db.get_mod(mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let versions = state
        .forge
        .get_versions(installed.forge_mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(|e| WebError::Internal(e))?;

    let version = versions.first().ok_or(WebError::BadRequest(
        "No compatible update found".to_string(),
    ))?;

    if version.version == installed.version {
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/mods/{mod_db_id}")))
            .finish());
    }

    // Check if the operation should be queued
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
        .await
        .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = installed.name.clone();
        let version_id = version.id;
        let forge_mod_id = installed.forge_mod_id;
        web::block(move || {
            let db = db.lock();
            db.insert_pending_op("update", forge_mod_id, Some(version_id), &mod_name, None, None)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let link = version.link.as_deref().ok_or(WebError::BadRequest(
        "Version has no download link".to_string(),
    ))?;

    let tmp_dir = tempfile::tempdir().map_err(|e| WebError::Internal(e.into()))?;
    let archive_path = tmp_dir.path().join("mod.zip");
    state
        .forge
        .download_file(link, &archive_path)
        .await
        .map_err(|e| WebError::Internal(e))?;

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let version_id = version.id;
    let version_str = version.version.clone();

    web::block(move || {
        use crate::spt::mods::{delete_mod_files, extract_mod};

        let db = db.lock();

        let old_files = db.get_files_for_mod(mod_db_id)?;
        let old_paths: Vec<String> = old_files.iter().map(|f| f.file_path.clone()).collect();
        delete_mod_files(&spt_dir, &old_paths)?;
        db.delete_files_for_mod(mod_db_id)?;

        let extracted = extract_mod(&archive_path, &spt_dir)?;
        for file in &extracted {
            db.insert_file(
                mod_db_id,
                &file.path,
                Some(&file.hash),
                Some(file.size as i64),
            )?;
        }

        db.update_mod(mod_db_id, version_id, &version_str)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/mods/{mod_db_id}")))
        .finish())
}

pub async fn remove_mod(
    state: Data<AppState>,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let mod_db_id = path.into_inner();

    // Look up the installed mod for queue metadata
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.get_mod(mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    // Check if the operation should be queued
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
        .await
        .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = installed.name.clone();
        let forge_mod_id = installed.forge_mod_id;
        web::block(move || {
            let db = db.lock();
            db.insert_pending_op("remove", forge_mod_id, None, &mod_name, None, None)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();

    web::block(move || {
        use crate::spt::mods::delete_mod_files;

        let db = db.lock();
        let files = db.get_files_for_mod(mod_db_id)?;
        let paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
        delete_mod_files(&spt_dir, &paths)?;
        db.delete_mod(mod_db_id)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}

pub async fn update_all_mods(
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if installed.is_empty() {
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/mods"))
            .finish());
    }

    let check_list: Vec<(i64, String)> = installed
        .iter()
        .map(|m| (m.forge_mod_id, m.version.clone()))
        .collect();

    let results = state
        .forge
        .check_updates(&check_list, &state.spt_info.spt_version)
        .await
        .map_err(|e| WebError::Internal(e))?;

    for update in &results.updates {
        let link = match &update.recommended_version.link {
            Some(l) => l.clone(),
            None => continue,
        };

        let mod_db = installed
            .iter()
            .find(|m| m.forge_mod_id == update.current_version.mod_id);
        let mod_db = match mod_db {
            Some(m) => m,
            None => continue,
        };
        let mod_db_id = mod_db.id;

        let tmp_dir = tempfile::tempdir().map_err(|e| WebError::Internal(e.into()))?;
        let archive_path = tmp_dir.path().join("mod.zip");
        state
            .forge
            .download_file(&link, &archive_path)
            .await
            .map_err(|e| WebError::Internal(e))?;

        let spt_dir = state.spt_dir.clone();
        let db = state.db.clone();
        let version_id = update.recommended_version.id;
        let version_str = update.recommended_version.version.clone();

        web::block(move || {
            use crate::spt::mods::{delete_mod_files, extract_mod};

            let db = db.lock();
            let old_files = db.get_files_for_mod(mod_db_id)?;
            let old_paths: Vec<String> = old_files.iter().map(|f| f.file_path.clone()).collect();
            delete_mod_files(&spt_dir, &old_paths)?;
            db.delete_files_for_mod(mod_db_id)?;

            let extracted = extract_mod(&archive_path, &spt_dir)?;
            for file in &extracted {
                db.insert_file(
                    mod_db_id,
                    &file.path,
                    Some(&file.hash),
                    Some(file.size as i64),
                )?;
            }

            db.update_mod(mod_db_id, version_id, &version_str)?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}
```

- [ ] **Step 7: Update `src/web/handlers/mod.rs`**

```rust
pub mod auth;
pub mod dashboard;
pub mod mods;
```

- [ ] **Step 8: Update `src/web/mod.rs` — add dashboard and mod routes**

Replace the `index` handler and update the route registration. Remove the standalone `index` function and `IndexTemplate` struct. Update the `App::new()` builder:

```rust
            // Authenticated routes (all users)
            .service(
                web::scope("")
                    .wrap(auth::RequireAuth)
                    .route("/", web::get().to(handlers::dashboard::dashboard))
                    .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
                    .route("/status", web::get().to(todo_handler))
                    .route("/queue", web::get().to(todo_handler))
            )
            // Admin routes
            .service(
                web::scope("")
                    .wrap(auth::RequireAdmin)
                    .route("/mods", web::get().to(handlers::mods::list_mods))
                    .route("/mods/install", web::post().to(handlers::mods::install_mod))
                    .route("/mods/update-all", web::post().to(handlers::mods::update_all_mods))
                    .route("/mods/{id}/update", web::post().to(handlers::mods::update_mod))
                    .route("/mods/{id}/remove", web::post().to(handlers::mods::remove_mod))
            )
            // HTMX API (authenticated)
            .service(
                web::scope("/api")
                    .wrap(auth::RequireAuth)
                    .route("/mods/check-updates", web::get().to(handlers::mods::check_updates_partial))
                    .route("/mods/dep-tree", web::get().to(handlers::mods::dep_tree_partial))
            )
```

Add a temporary placeholder handler for routes implemented in Task 20:

```rust
async fn todo_handler() -> Html<String> {
    Html::new("Coming soon".to_string())
}
```

Note: the `/mods` GET route is under `RequireAdmin` because only admins see the install form and action buttons. Players access mod details via `/mods/{id}` which is under `RequireAuth`. If you want players to also see the mod list (read-only), move the GET `/mods` route under `RequireAuth` and conditionally render admin-only elements in the template (the template already has `{% if user.is_admin() %}` guards).

- [ ] **Step 9: Verify everything compiles**

```bash
cargo check
```

Expected: compiles with no errors.

- [ ] **Step 10: Run all tests**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "feat: dashboard, mod list, mod detail, install/update/remove handlers, HTMX partials"
```

---

## Task 20: Queue, Status & Server Control Pages

**Files:**
- Create: `src/web/handlers/queue.rs`
- Create: `src/web/handlers/status.rs`
- Create: `src/web/handlers/server.rs`
- Create: `templates/queue.html`
- Create: `templates/status.html`
- Modify: `src/web/handlers/mod.rs` (add new modules)
- Modify: `src/web/mod.rs` (replace todo_handler with real routes, add server control routes)

**Interfaces:**
- Consumes: `AppState`, `RequireAuth`, `RequireAdmin`, `Database::list_pending_ops`, `Database::delete_pending_op`, `health::run_checks_with`, `PodmanClient`, `is_server_running`
- Produces: Queue page at `/queue`, status page at `/status`, server control POST handlers, `/api/status` HTMX partial

- [ ] **Step 1: Write `templates/queue.html`**

```html
{% extends "base.html" %}
{% block title %}Queue — Quartermaster{% endblock %}
{% block nav %}
<div class="links">
    <a href="/">Dashboard</a>
    <a href="/mods">Mods</a>
    <a href="/queue" class="active">Queue</a>
    <a href="/status">Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">Logout</button>
    </form>
</div>
{% endblock %}
{% block content %}
<div class="flex-between">
    <h1>Pending Operations</h1>
    {% if user.is_admin() && !ops.is_empty() %}
    <form method="post" action="/queue/apply">
        <button type="submit" class="btn btn-success">Apply All</button>
    </form>
    {% endif %}
</div>

{% if ops.is_empty() %}
<div class="card">
    <p class="text-muted">No pending operations.</p>
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
                    <form method="post" action="/queue/{{ op.id }}/cancel" style="display:inline">
                        <button type="submit" class="btn btn-sm btn-outline">Cancel</button>
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

- [ ] **Step 2: Write `templates/status.html`**

```html
{% extends "base.html" %}
{% block title %}Status — Quartermaster{% endblock %}
{% block nav %}
<div class="links">
    <a href="/">Dashboard</a>
    <a href="/mods">Mods</a>
    <a href="/queue">Queue</a>
    <a href="/status" class="active">Status</a>
</div>
<div class="user-info">
    {{ user.username }} ({{ user.role }})
    <form method="post" action="/logout" style="display:inline">
        <button type="submit" class="btn btn-sm btn-outline" style="margin-left:0.5rem">Logout</button>
    </form>
</div>
{% endblock %}
{% block content %}
<div class="flex-between">
    <h1>Server Status</h1>
    {% if user.is_admin() %}
    <div class="flex gap-1">
        <form method="post" action="/server/start" style="display:inline">
            <button type="submit" class="btn btn-sm btn-success">Start</button>
        </form>
        <form method="post" action="/server/restart" style="display:inline">
            <button type="submit" class="btn btn-sm btn-warning">Restart</button>
        </form>
        <form method="post" action="/server/stop" style="display:inline">
            <button type="submit" class="btn btn-sm btn-danger">Stop</button>
        </form>
    </div>
    {% endif %}
</div>

<div hx-get="/api/status" hx-trigger="load, every 30s" hx-swap="innerHTML" id="status-content">
    <p class="text-muted">Loading status...</p>
</div>
{% endblock %}
```

- [ ] **Step 3: Write a status partial template**

Create `templates/partials/` directory.

`templates/partials/status_detail.html`:

```html
<div class="card">
    <h2>SPT Server</h2>
    <table>
        <tr>
            <th style="width:140px">Status</th>
            <td>
                {% if report.server.reachable %}
                <span class="status-dot up"></span> Running
                {% if let Some(ms) = report.server.latency_ms %} ({{ ms }}ms){% endif %}
                {% else %}
                <span class="status-dot down"></span> Down
                {% if let Some(err) = &report.server.error %}<span class="text-muted text-sm"> — {{ err }}</span>{% endif %}
                {% endif %}
            </td>
        </tr>
        <tr><th>Address</th><td>{{ report.server.address }}</td></tr>
        {% if let Some(v) = &report.server.version %}
        <tr>
            <th>Version</th>
            <td>{{ v }}
                {% if let Some(matches) = report.server.version_matches %}
                    {% if matches %}<span class="badge badge-success">matches</span>
                    {% else %}<span class="badge badge-danger">mismatch</span>
                    {% endif %}
                {% endif %}
            </td>
        </tr>
        {% endif %}
    </table>
</div>

<div class="card">
    <h2>Mods</h2>
    <table>
        <tr><th style="width:140px">Installed</th><td>{{ report.mods.installed_count }}</td></tr>
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

<div class="card">
    <h2>Integrity ({{ report.integrity.tracked_files }} tracked files)</h2>
    {% if report.integrity.missing_files.is_empty() && report.integrity.modified_files.is_empty() && report.integrity.untracked_dirs.is_empty() %}
    <p class="text-muted">All mod files present, hashes match.</p>
    {% else %}
        {% if !report.integrity.missing_files.is_empty() %}
        <p><span class="badge badge-danger">{{ report.integrity.missing_files.len() }} missing</span></p>
        {% endif %}
        {% if !report.integrity.modified_files.is_empty() %}
        <p><span class="badge badge-warning">{{ report.integrity.modified_files.len() }} modified</span></p>
        {% endif %}
        {% if !report.integrity.untracked_dirs.is_empty() %}
        <p class="mt-1"><span class="text-muted text-sm">{{ report.integrity.untracked_dirs.len() }} untracked director{% if report.integrity.untracked_dirs.len() != 1 %}ies{% else %}y{% endif %}</span></p>
        {% endif %}
    {% endif %}
</div>
```

- [ ] **Step 4: Write `src/web/handlers/status.rs`**

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Html};
use askama::Template;

use crate::health::{self, HealthReport};
use crate::web::auth::{get_session_user, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "status.html")]
struct StatusPageTemplate {
    user: SessionUser,
}

#[derive(Template)]
#[template(path = "partials/status_detail.html")]
struct StatusDetailTemplate {
    report: HealthReport,
}

pub async fn status_page(session: Session) -> actix_web::Result<Html<String>> {
    let user = get_session_user(&session).unwrap();
    let tmpl = StatusPageTemplate { user };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn status_partial(state: Data<AppState>) -> actix_web::Result<Html<String>> {
    let report = build_health_report(&state).await.map_err(WebError::from)?;
    let tmpl = StatusDetailTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

async fn build_health_report(state: &AppState) -> anyhow::Result<HealthReport> {
    use crate::server_detect::resolve_server_addr;
    use crate::spt::server::SptClient;
    use crate::spt::mods::{compute_file_hash, scan_mod_directories};

    let (host, port) = resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    // Server check (async, no DB needed)
    let server = {
        let ping = spt_client.ping().await;
        let (reachable, latency_ms, error) = match &ping {
            Ok(p) if p.ok => (true, Some(p.latency_ms), None),
            Ok(_) => (false, None, Some("server returned error".to_string())),
            Err(e) => (false, None, Some(format!("{e:#}"))),
        };

        let (version, version_matches) = if reachable {
            let v = spt_client.server_version().await.ok();
            let matches = v.as_deref().map(|v| v == state.spt_info.spt_version);
            (v, matches)
        } else {
            (None, None)
        };

        health::ServerHealth {
            reachable,
            latency_ms,
            version,
            version_matches,
            address,
            error,
        }
    };

    // Mods check (needs DB for list, then async Forge call)
    let db = state.db.clone();
    let installed_mods = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await??;

    let mods = {
        let mut updates_available = 0;
        let mut incompatible_mods = Vec::new();

        if !installed_mods.is_empty() {
            let check_list: Vec<(i64, String)> = installed_mods
                .iter()
                .map(|m| (m.forge_mod_id, m.version.clone()))
                .collect();

            if let Ok(results) = state
                .forge
                .check_updates(&check_list, &state.spt_info.spt_version)
                .await
            {
                updates_available = results.updates.len();
                for m in &results.incompatible_with_spt {
                    incompatible_mods.push(m.name.clone());
                }
            }
        }

        health::ModsHealth {
            installed_count: installed_mods.len(),
            updates_available,
            incompatible_mods,
        }
    };

    // Integrity check (sync, needs DB)
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let integrity = web::block(move || {
        let db = db.lock();
        let tracked_files = db.get_all_tracked_files()?;
        let mut missing_files = Vec::new();
        let mut modified_files = Vec::new();

        for file in &tracked_files {
            let full_path = spt_dir.join(&file.file_path);
            if !full_path.exists() {
                missing_files.push(file.file_path.clone());
                continue;
            }
            if let Some(ref expected_hash) = file.file_hash {
                match compute_file_hash(&full_path) {
                    Ok(actual_hash) if actual_hash != *expected_hash => {
                        modified_files.push(file.file_path.clone());
                    }
                    Err(_) => {
                        modified_files.push(file.file_path.clone());
                    }
                    _ => {}
                }
            }
        }

        let all_disk_files = scan_mod_directories(&spt_dir)?;
        let tracked_paths: std::collections::HashSet<&str> =
            tracked_files.iter().map(|f| f.file_path.as_str()).collect();
        let untracked: Vec<&str> = all_disk_files
            .iter()
            .filter(|f| !tracked_paths.contains(f.as_str()))
            .map(|f| f.as_str())
            .collect();

        let mut dir_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for path in &untracked {
            let parts: Vec<&str> = path.split('/').collect();
            let dir = if path.starts_with("SPT/") && parts.len() >= 4 {
                format!("{}/{}/{}/{}", parts[0], parts[1], parts[2], parts[3])
            } else if path.starts_with("BepInEx/") && parts.len() >= 3 {
                format!("{}/{}/{}", parts[0], parts[1], parts[2])
            } else {
                path.to_string()
            };
            *dir_counts.entry(dir).or_default() += 1;
        }

        let untracked_dirs = dir_counts
            .into_iter()
            .map(|(path, file_count)| health::UntrackedDir { path, file_count })
            .collect();

        Ok::<_, anyhow::Error>(health::IntegrityHealth {
            tracked_files: tracked_files.len(),
            missing_files,
            modified_files,
            untracked_dirs,
        })
    })
    .await??;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}
```

- [ ] **Step 5: Write `src/web/handlers/queue.rs`**

```rust
use actix_session::Session;
use actix_web::web::{self, Data, Html, Path};
use actix_web::HttpResponse;
use askama::Template;

use crate::db::users::PendingOperation;
use crate::web::auth::{get_session_user, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "queue.html")]
struct QueueTemplate {
    user: SessionUser,
    ops: Vec<PendingOperation>,
}

pub async fn queue_page(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html<String>> {
    let user = get_session_user(&session).unwrap();
    let db = state.db.clone();

    let ops = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = QueueTemplate { user, ops };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn cancel_op(
    state: Data<AppState>,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let op_id = path.into_inner();
    let db = state.db.clone();

    web::block(move || {
        let db = db.lock();
        db.delete_pending_op(op_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish())
}

pub async fn apply_queue(
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let server_running = crate::server_detect::is_server_running(&state.config, &state.spt_dir)
        .await
        .unwrap_or(false);

    if server_running {
        return Ok(HttpResponse::BadRequest()
            .body("Cannot apply queue while SPT server is running. Stop the server first."));
    }

    let db = state.db.clone();
    let ops = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    for op in &ops {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let op_id = op.id;

        match op.action.as_str() {
            "install" => {
                if let Some(version_id) = op.forge_version_id {
                    let forge_mod = state.forge.get_mod(op.forge_mod_id, true).await;
                    if let Ok(forge_mod) = forge_mod {
                        let versions = forge_mod.versions.unwrap_or_default();
                        if let Some(version) = versions.iter().find(|v| v.id == version_id) {
                            if let Some(link) = &version.link {
                                let tmp_dir = tempfile::tempdir().ok();
                                if let Some(tmp_dir) = tmp_dir {
                                    let archive_path = tmp_dir.path().join("mod.zip");
                                    if state.forge.download_file(link, &archive_path).await.is_ok() {
                                        let mod_name = op.mod_name.clone();
                                        let version_str = version.version.clone();
                                        let forge_mod_id = op.forge_mod_id;
                                        let _ = web::block(move || {
                                            use crate::spt::mods::extract_mod;
                                            let extracted = extract_mod(&archive_path, &spt_dir)?;
                                            let db = db.lock();
                                            let installed_id = db.insert_mod(
                                                forge_mod_id,
                                                version_id,
                                                &mod_name,
                                                None,
                                                &version_str,
                                            )?;
                                            for file in &extracted {
                                                db.insert_file(
                                                    installed_id,
                                                    &file.path,
                                                    Some(&file.hash),
                                                    Some(file.size as i64),
                                                )?;
                                            }
                                            Ok::<_, anyhow::Error>(())
                                        })
                                        .await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "remove" => {
                let _ = web::block(move || {
                    use crate::spt::mods::delete_mod_files;
                    let db = db.lock();
                    if let Ok(Some(installed)) = db.get_mod_by_forge_id(op.forge_mod_id) {
                        let files = db.get_files_for_mod(installed.id)?;
                        let paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
                        delete_mod_files(&spt_dir, &paths)?;
                        db.delete_mod(installed.id)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            }
            _ => {}
        }

        let db = state.db.clone();
        let _ = web::block(move || {
            let db = db.lock();
            db.delete_pending_op(op_id)
        })
        .await;
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish())
}
```

- [ ] **Step 6: Write `src/web/handlers/server.rs`**

```rust
use actix_web::web::Data;
use actix_web::HttpResponse;

use crate::podman::PodmanClient;
use crate::web::error::WebError;
use crate::web::state::AppState;

pub async fn start_server(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.start().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}

pub async fn stop_server(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.stop().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}

pub async fn restart_server(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.stop().await.map_err(WebError::from)?;
    podman.start().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}
```

- [ ] **Step 7: Update `src/web/handlers/mod.rs`**

```rust
pub mod auth;
pub mod dashboard;
pub mod mods;
pub mod queue;
pub mod server;
pub mod status;
```

- [ ] **Step 8: Update `src/web/mod.rs` — replace placeholders, add all remaining routes**

Remove the `todo_handler` function. Replace the route registration block with the final version:

```rust
            // Auth routes (public)
            // TODO(debt): add rate limiting via actix-governor (5 req/min/IP on /login and /register)
            .route("/login", web::get().to(handlers::auth::login_page))
            .route("/login", web::post().to(handlers::auth::login_submit))
            .route("/register", web::get().to(handlers::auth::register_page))
            .route("/register", web::post().to(handlers::auth::register_submit))
            .route("/logout", web::post().to(handlers::auth::logout))
            // Admin-only routes
            .service(
                web::scope("")
                    .wrap(auth::RequireAdmin)
                    .route("/mods", web::get().to(handlers::mods::list_mods))
                    .route("/mods/install", web::post().to(handlers::mods::install_mod))
                    .route("/mods/update-all", web::post().to(handlers::mods::update_all_mods))
                    .route("/mods/{id}/update", web::post().to(handlers::mods::update_mod))
                    .route("/mods/{id}/remove", web::post().to(handlers::mods::remove_mod))
                    .route("/server/start", web::post().to(handlers::server::start_server))
                    .route("/server/stop", web::post().to(handlers::server::stop_server))
                    .route("/server/restart", web::post().to(handlers::server::restart_server))
                    .route("/queue/{id}/cancel", web::post().to(handlers::queue::cancel_op))
                    .route("/queue/apply", web::post().to(handlers::queue::apply_queue))
            )
            // Authenticated routes (all users)
            .service(
                web::scope("")
                    .wrap(auth::RequireAuth)
                    .route("/", web::get().to(handlers::dashboard::dashboard))
                    .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
                    .route("/status", web::get().to(handlers::status::status_page))
                    .route("/queue", web::get().to(handlers::queue::queue_page))
            )
            // HTMX API (authenticated)
            .service(
                web::scope("/api")
                    .wrap(auth::RequireAuth)
                    .route("/mods/check-updates", web::get().to(handlers::mods::check_updates_partial))
                    .route("/mods/dep-tree", web::get().to(handlers::mods::dep_tree_partial))
                    .route("/status", web::get().to(handlers::status::status_partial))
            )
            // Static assets (public)
            .route("/assets/{path:.*}", web::get().to(serve_asset))
```

Note on route ordering: actix-web matches routes in registration order. The more restrictive scopes (RequireAdmin) are registered before the less restrictive ones (RequireAuth). For routes that share a path prefix (like `/mods`), the admin POST routes come before the auth GET routes. actix-web matches by method + path, so `POST /mods/install` under admin scope won't conflict with `GET /mods/{id}` under auth scope.

- [ ] **Step 9: Verify everything compiles**

```bash
cargo check
```

Expected: compiles with no errors.

- [ ] **Step 10: Run all tests**

```bash
cargo test
```

Expected: all existing tests pass.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "feat: queue management, server status, and server control pages with HTMX auto-refresh"
```

---

## Implementation Notes

### Database Concurrency Pattern

All web handlers follow this pattern for DB access:

```rust
let db = state.db.clone(); // clone the Arc
let result = web::block(move || {
    let db = db.lock(); // parking_lot::Mutex — no .unwrap() needed, no poison risk
    db.some_operation()
}).await??; // first ? for BlockingError, second for the inner Result
```

This ensures:
1. The `parking_lot::Mutex` lock is held only on a blocking thread (not the async executor)
2. The lock is released when the closure returns
3. Other requests can proceed while one waits for the lock
4. If a DB operation panics, the mutex does NOT poison — subsequent requests can still proceed

### CPU-Intensive Operations

Argon2 password hashing/verification is intentionally slow (~100ms+). These operations MUST be wrapped in `web::block()` to avoid blocking the tokio async executor. The plan wraps both `hash_password` and `verify_password` calls in `web::block`.

### Error Handling

`WebError` implements `ResponseError` and converts from `anyhow::Error`, `rusqlite::Error`, `askama::Error`, and `BlockingError`. Handlers return `actix_web::Result<T>` where errors automatically become HTTP responses.

### Session Data

Session stores three values: `user_id` (i64), `username` (String), `role` (String). The `get_session_user` function extracts all three — if any is missing, it returns `None` (user not logged in).

### Template Directory

Templates live in `templates/` at the crate root (askama default). No `askama.toml` is needed since we're using the default path.

### Static Assets

CSS and HTMX JS are embedded in the binary via `rust-embed`. The `#[folder = "src/assets/"]` attribute points to the source directory. At runtime, assets are served from `/assets/{path}` via `actix-web-rust-embed-responder` which handles content types, caching headers, and compression. The `serve_asset` handler explicitly returns 404 for missing assets.

### Queue Awareness

All mod mutation handlers (`install_mod`, `update_mod`, `remove_mod`) check `queue::should_queue()` before applying changes. If the SPT server is running and `queue_changes` is enabled, operations are written to `pending_operations` and the user is redirected to `/queue`. This matches the CLI behavior from Phase 3.

### Rate Limiting (Deferred)

Rate limiting on `/login` and `/register` (5 req/min/IP via `actix-governor`) is deferred to post-Phase 4. The routes are functional but unprotected against brute force. The `TODO(debt)` comment marks where the middleware should be added.

### Health Checks in Web Context

The web status handler (`build_health_report`) constructs the `HealthReport` inline rather than calling `health::run_checks()`. This is because `run_checks` takes `&CliContext` which holds a `Database` directly, while the web layer has `Arc<Mutex<Database>>`. The inline version interleaves DB locks (via `web::block`) with async Forge API calls, avoiding holding the mutex across await points.

### Potential API Mismatch

The `serve_asset` handler and `EmbedResponse` usage should be verified against the exact `actix-web-rust-embed-responder` 2.4 API during implementation. The crate's `IntoResponse` trait may have a different signature than shown — check the docs and adjust the handler accordingly. The core requirement is: return the embedded file with proper content type or 404.
