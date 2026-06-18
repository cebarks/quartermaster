pub mod auth;
pub mod csrf;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod state;
pub mod tasks;

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

use actix_governor::{Governor, GovernorConfigBuilder};

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
        spt_dir,
        spt_info,
        tasks: crate::web::tasks::TaskTracker::new(),
    });

    tracing::info!("Quartermaster web UI starting on http://{bind_addr}");

    let governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(12) // 5 per minute = 1 per 12 seconds replenish
        .burst_size(5) // allow bursting up to 5
        .finish()
        .expect("invalid governor config");

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
            .wrap(tracing_actix_web::TracingLogger::default())
            // Static assets (public, before auth scope to avoid shadowing)
            .route("/assets/{path:.*}", web::get().to(serve_asset))
            // Auth routes (public)
            .route("/login", web::get().to(handlers::auth::login_page))
            .route("/logout", web::post().to(handlers::auth::logout))
            // Rate-limited auth routes (5 req/min/IP on login POST + register)
            .service(
                web::resource("/login")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::post().to(handlers::auth::login_submit)),
            )
            .service(
                web::resource("/register")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::get().to(handlers::auth::register_page))
                    .route(web::post().to(handlers::auth::register_submit)),
            )
            // HTMX API (authenticated, registered before catch-all scope)
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
                    )
                    .route(
                        "/tasks/status",
                        web::get().to(handlers::tasks::task_status_partial),
                    )
                    .route(
                        "/tasks/{id}/dismiss",
                        web::post().to(handlers::tasks::dismiss_task),
                    ),
            )
            // Authenticated routes — admin checks are per-handler via require_admin()
            .service(
                web::scope("")
                    .wrap(auth::RequireAuth)
                    .route("/", web::get().to(handlers::dashboard::dashboard))
                    .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
                    .route("/status", web::get().to(handlers::status::status_page))
                    .route("/queue", web::get().to(handlers::queue::queue_page))
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
    })
    .bind(&bind_addr)
    .with_context(|| format!("failed to bind to {bind_addr}"))?
    .run()
    .await
    .context("web server error")
}
