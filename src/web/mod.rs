pub mod auth;
pub mod csrf;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod sse;
pub mod state;
pub mod tasks;
pub mod template_filters;
pub mod update_cache;

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
use actix_web::middleware::from_fn;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::logging::LogBroadcast;
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

#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    config: Config,
    db: Database,
    forge: ForgeClient,
    spt_dir: std::path::PathBuf,
    spt_info: SptInfo,
    log_broadcast: Arc<LogBroadcast>,
    container_mgr: Option<Arc<crate::container::ContainerManager>>,
    client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    converging: Arc<std::sync::atomic::AtomicBool>,
    fika_installed: bool,
) -> Result<()> {
    let bind_addr = format!("{}:{}", config.web_bind, config.web_port);

    let session_key = Key::derive_from(config.session_secret.as_bytes());

    let (events_tx, _) = tokio::sync::broadcast::channel::<crate::web::sse::ServerEvent>(64);

    let db = Arc::new(parking_lot::Mutex::new(db));
    let app_state = web::Data::new(AppState {
        db,
        forge,
        config: config.clone(),
        spt_dir,
        spt_info,
        tasks: crate::web::tasks::TaskTracker::new(events_tx.clone()),
        update_cache: crate::web::update_cache::UpdateCache::new(config.update_check_interval),
        events: events_tx,
        log_broadcast,
        container_mgr,
        client_states,
        converging,
        fika_installed,
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
                    .wrap(from_fn(auth::auth_middleware))
                    .route("/events", web::get().to(crate::web::sse::events_stream))
                    .route(
                        "/mods/check-updates",
                        web::get().to(handlers::mods::check_updates_partial),
                    )
                    .route(
                        "/mods/update-status",
                        web::get().to(handlers::mods::update_status_partial),
                    )
                    .route(
                        "/mods/list",
                        web::get().to(handlers::mods::list_body_partial),
                    )
                    .route(
                        "/mods/dep-tree",
                        web::get().to(handlers::mods::dep_tree_partial),
                    )
                    .route(
                        "/status/server",
                        web::get().to(handlers::status::server_partial),
                    )
                    .route(
                        "/status/mods",
                        web::get().to(handlers::status::mods_partial),
                    )
                    .route(
                        "/status/integrity",
                        web::get().to(handlers::status::integrity_partial),
                    )
                    .route(
                        "/dashboard/server-status",
                        web::get().to(handlers::dashboard::server_status_partial),
                    )
                    .route(
                        "/dashboard/clients-status",
                        web::get().to(handlers::clients::dashboard_clients_status_partial),
                    )
                    .route(
                        "/clients/status",
                        web::get().to(handlers::clients::client_status_partial),
                    )
                    .route(
                        "/tasks/status",
                        web::get().to(handlers::tasks::task_status_partial),
                    )
                    .route(
                        "/tasks/{id}/dismiss",
                        web::post().to(handlers::tasks::dismiss_task),
                    )
                    .route("/logs/app", web::get().to(handlers::logs::app_logs_json))
                    .route(
                        "/logs/app/stream",
                        web::get().to(handlers::logs::app_logs_stream),
                    )
                    .route(
                        "/logs/server",
                        web::get().to(handlers::logs::server_logs_json),
                    )
                    .route(
                        "/logs/server/stream",
                        web::get().to(handlers::logs::server_logs_stream),
                    ),
            )
            // Authenticated routes — admin checks are per-handler via require_admin()
            .service(
                web::scope("")
                    .wrap(from_fn(auth::auth_middleware))
                    .route("/", web::get().to(handlers::dashboard::dashboard))
                    .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
                    .route("/status", web::get().to(handlers::status::status_page))
                    .route("/queue", web::get().to(handlers::queue::queue_page))
                    .route("/logs", web::get().to(handlers::logs::logs_page))
                    .route("/mods", web::get().to(handlers::mods::list_mods))
                    .route("/clients", web::get().to(handlers::clients::client_list))
                    .route(
                        "/clients/{n}",
                        web::get().to(handlers::clients::client_detail),
                    )
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
                    .route("/queue/apply", web::post().to(handlers::queue::apply_queue))
                    .route(
                        "/clients/{n}/restart",
                        web::post().to(handlers::clients::client_restart),
                    )
                    .route(
                        "/clients/{n}/stop",
                        web::post().to(handlers::clients::client_stop),
                    )
                    .route(
                        "/clients/{n}/start",
                        web::post().to(handlers::clients::client_start),
                    )
                    .route(
                        "/clients/scale",
                        web::post().to(handlers::clients::client_scale),
                    ),
            )
    })
    .bind(&bind_addr)
    .with_context(|| format!("failed to bind to {bind_addr}"))?
    .run()
    .await
    .context("web server error")
}
