pub mod auth;
pub mod error;
pub mod handlers;
pub mod state;

use std::sync::Arc;

use actix_session::config::PersistentSession;
use actix_session::storage::CookieSessionStore;
use actix_session::SessionMiddleware;
use actix_web::cookie::time::Duration as CookieDuration;
use actix_web::cookie::Key;
use actix_web::web;
use actix_web::{middleware, App, Either, HttpResponse, HttpServer, Responder};
use actix_web_rust_embed_responder::IntoResponse;
use anyhow::{Context, Result};
use rust_embed::RustEmbed;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::SptInfo;

use state::AppState;

#[derive(RustEmbed)]
#[folder = "src/assets/"]
struct Assets;

async fn serve_asset(path: web::Path<String>) -> impl Responder {
    match Assets::get(&path) {
        Some(file) => Either::Left(file.into_response()),
        None => Either::Right(HttpResponse::NotFound().body("asset not found")),
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
                        PersistentSession::default().session_ttl(CookieDuration::days(7)),
                    )
                    .cookie_http_only(true)
                    .cookie_same_site(actix_web::cookie::SameSite::Strict)
                    .cookie_secure(false)
                    .build(),
            )
            .wrap(middleware::NormalizePath::trim())
            // Auth routes (public)
            // TODO(debt): add rate limiting via actix-governor (5 req/min/IP on /login and /register)
            // TODO(debt): add CSRF protection on state-mutating POST forms (SameSite=Strict mitigates most vectors)
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
                    .route(
                        "/mods/update-all",
                        web::post().to(handlers::mods::update_all_mods),
                    )
                    .route(
                        "/mods/{id}/update",
                        web::post().to(handlers::mods::update_mod),
                    )
                    .route(
                        "/mods/{id}/remove",
                        web::post().to(handlers::mods::remove_mod),
                    )
                    .route(
                        "/server/start",
                        web::post().to(handlers::server::start_server),
                    )
                    .route(
                        "/server/stop",
                        web::post().to(handlers::server::stop_server),
                    )
                    .route(
                        "/server/restart",
                        web::post().to(handlers::server::restart_server),
                    )
                    .route(
                        "/queue/{id}/cancel",
                        web::post().to(handlers::queue::cancel_op),
                    )
                    .route("/queue/apply", web::post().to(handlers::queue::apply_queue)),
            )
            // Authenticated routes (all users)
            .service(
                web::scope("")
                    .wrap(auth::RequireAuth)
                    .route("/", web::get().to(handlers::dashboard::dashboard))
                    .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
                    .route("/status", web::get().to(handlers::status::status_page))
                    .route("/queue", web::get().to(handlers::queue::queue_page)),
            )
            // HTMX API (authenticated)
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
                    .route("/status", web::get().to(handlers::status::status_partial)),
            )
            // Static assets (public)
            .route("/assets/{path:.*}", web::get().to(serve_asset))
    })
    .bind(&bind_addr)
    .with_context(|| format!("failed to bind to {bind_addr}"))?
    .run()
    .await
    .context("web server error")
}
