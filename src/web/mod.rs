pub mod auth;
pub mod csrf;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod proxy;
pub mod proxy_metrics;
pub mod proxy_ws;
pub mod raid_tracker;
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
use crate::spt::game_data::GameData;

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
    config_path: std::path::PathBuf,
    db: Database,
    forge: ForgeClient,
    spt_dir: std::path::PathBuf,
    spt_info: SptInfo,
    log_broadcast: Arc<LogBroadcast>,
    container_mgr: Option<Arc<crate::container::ContainerManager>>,
    client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    converging: Arc<std::sync::atomic::AtomicBool>,
    fika_installed: bool,
    modsync_installed: bool,
) -> Result<()> {
    let bind_addr = format!("{}:{}", config.web_bind, config.web_port);

    let session_key = Key::derive_from(config.session_secret.as_bytes());

    let (events_tx, _) = tokio::sync::broadcast::channel::<crate::web::sse::ServerEvent>(64);

    let game_data = Arc::new(GameData::load(&spt_dir).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to load SPT game data, lookups will return raw IDs");
        GameData::load_empty()
    }));

    let db_arc = Arc::new(parking_lot::Mutex::new(db));

    // Regenerate NarcoNet config on startup to ensure consistency
    if modsync_installed && config.modsync.is_some() {
        if let Err(e) = crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db_arc.lock()) {
            tracing::warn!(error = %e, "failed to regenerate NarcoNet config on startup");
        }
    }

    let tls_enabled = config.tls_enabled;
    let spt_dir_for_tls = spt_dir.clone();

    let proxy_client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build proxy HTTP client");

    let app_state = web::Data::new(AppState {
        db: db_arc,
        forge,
        config: config.clone(),
        config_path,
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
        modsync_installed: std::sync::atomic::AtomicBool::new(modsync_installed),
        server_transition: Arc::new(parking_lot::Mutex::new(None)),
        game_data,
        proxy_metrics: crate::web::proxy_metrics::ProxyMetrics::new(),
        proxy_client,
    });

    let governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(12) // 5 per minute = 1 per 12 seconds replenish
        .burst_size(5) // allow bursting up to 5
        .finish()
        .expect("invalid governor config");

    let search_governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(6) // 10 per minute = 1 per 6 seconds replenish
        .burst_size(10)
        .finish()
        .expect("invalid search governor config");

    let server_builder = HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .app_data(web::PayloadConfig::new(64 * 1024 * 1024))
            .wrap(middleware::NormalizePath::new(
                middleware::TrailingSlash::MergeOnly,
            ))
            .wrap(tracing_actix_web::TracingLogger::default())
            .service(
                web::scope("/quma")
                    .wrap(
                        SessionMiddleware::builder(
                            CookieSessionStore::default(),
                            session_key.clone(),
                        )
                        .session_lifecycle(
                            PersistentSession::default().session_ttl(CookieDuration::days(7)),
                        )
                        .cookie_http_only(true)
                        .cookie_same_site(actix_web::cookie::SameSite::Strict)
                        .cookie_secure(tls_enabled)
                        .build(),
                    )
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
                    // Password reset (public, rate-limited)
                    .service(
                        web::resource("/reset-password")
                            .wrap(Governor::new(&governor_conf))
                            .route(web::get().to(handlers::auth::reset_password_page))
                            .route(web::post().to(handlers::auth::reset_password_submit)),
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
                                "/status/proxy",
                                web::get().to(handlers::status::proxy_metrics_partial),
                            )
                            .route(
                                "/status/container-stats",
                                web::get().to(handlers::status::container_stats_partial),
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
                                "/profiles/{username}/quests",
                                web::get().to(handlers::profiles::quests_partial),
                            )
                            .route(
                                "/profiles/{username}/traders",
                                web::get().to(handlers::profiles::traders_partial),
                            )
                            .route(
                                "/profiles/{username}/hideout",
                                web::get().to(handlers::profiles::hideout_partial),
                            )
                            .route(
                                "/profiles/{username}/stash",
                                web::get().to(handlers::profiles::stash_partial),
                            )
                            .route(
                                "/profiles/{username}/stash/visibility",
                                web::post().to(handlers::profiles::toggle_stash_visibility),
                            )
                            .route(
                                "/raids/active",
                                web::get().to(handlers::raids::active_raids_partial),
                            )
                            .route(
                                "/raids/recent",
                                web::get().to(handlers::raids::recent_raids_partial),
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
                            )
                            // Mod request routes
                            .service(
                                web::resource("/requests/search")
                                    .wrap(Governor::new(&search_governor_conf))
                                    .route(web::get().to(handlers::requests::search_mods)),
                            )
                            .service(
                                web::resource("/mods/search")
                                    .wrap(Governor::new(&search_governor_conf))
                                    .route(web::get().to(handlers::mods::search_mods)),
                            )
                            .service(
                                web::resource("/mods/{id}/compat-check")
                                    .wrap(Governor::new(&search_governor_conf))
                                    .route(web::get().to(handlers::mods::compat_check)),
                            )
                            .route(
                                "/mods/requests",
                                web::get().to(handlers::requests::requests_tab),
                            )
                            .route(
                                "/requests",
                                web::post().to(handlers::requests::create_request),
                            )
                            .route(
                                "/requests/{id}/vote",
                                web::post().to(handlers::requests::vote),
                            )
                            .route(
                                "/requests/{id}/votes",
                                web::get().to(handlers::requests::vote_comments),
                            )
                            .route(
                                "/requests/{id}/resolve",
                                web::post().to(handlers::requests::resolve_request),
                            )
                            // Admin API (requires can_manage_users via scoped middleware)
                            .service(
                                web::scope("/admin")
                                    .wrap(from_fn(handlers::admin::admin_middleware))
                                    .route("/users", web::get().to(handlers::admin::admin_users))
                                    .route(
                                        "/invites",
                                        web::get().to(handlers::admin::admin_invites),
                                    )
                                    .route(
                                        "/users/{id}/role",
                                        web::post().to(handlers::admin::change_role),
                                    )
                                    .route(
                                        "/users/{id}/disable",
                                        web::post().to(handlers::admin::toggle_disable),
                                    )
                                    .route(
                                        "/users/{id}/reset-password",
                                        web::post().to(handlers::admin::create_reset_token),
                                    )
                                    .route(
                                        "/invites",
                                        web::post().to(handlers::admin::create_invite),
                                    ),
                            ),
                    )
                    // Authenticated routes -- admin checks are per-handler via require_admin()
                    .service(
                        web::scope("")
                            .wrap(from_fn(auth::auth_middleware))
                            .route("/", web::get().to(handlers::dashboard::dashboard))
                            .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
                            .route("/status", web::get().to(handlers::status::status_page))
                            .route("/queue", web::get().to(handlers::queue::queue_page))
                            .route("/logs", web::get().to(handlers::logs::logs_page))
                            .route("/admin", web::get().to(handlers::admin::admin_page))
                            .route("/mods", web::get().to(handlers::mods::list_mods))
                            .route("/modsync", web::get().to(handlers::modsync::modsync_page))
                            .route(
                                "/modsync/settings",
                                web::post().to(handlers::modsync::save_settings),
                            )
                            .route("/clients", web::get().to(handlers::clients::client_list))
                            .route(
                                "/clients/{n}",
                                web::get().to(handlers::clients::client_detail),
                            )
                            .route("/raids", web::get().to(handlers::raids::server_raids_page))
                            .route(
                                "/leaderboard",
                                web::get().to(handlers::leaderboard::leaderboard_page),
                            )
                            .route(
                                "/raids/{raid_id}",
                                web::get().to(handlers::raids::raid_detail_page),
                            )
                            .route(
                                "/profiles/{username}/raids",
                                web::get().to(handlers::raids::player_raids_page),
                            )
                            .route(
                                "/profiles/{username}",
                                web::get().to(handlers::profiles::profile_page),
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
                                "/mods/{id}/toggle-disable",
                                web::post().to(handlers::mods::toggle_disable),
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
                    ),
            )
            .route(
                "/",
                web::get().to(|| async {
                    HttpResponse::Found()
                        .insert_header(("Location", "/quma/"))
                        .finish()
                }),
            )
            .default_service(web::to(proxy::proxy_handler))
    });

    let server = if config.tls_enabled {
        let tls_config = crate::tls::load_or_generate_tls_config(&config, &spt_dir_for_tls)
            .context("failed to configure TLS")?;
        tracing::info!("Quartermaster starting on https://{bind_addr}");
        server_builder
            .bind_rustls_0_23(&bind_addr, tls_config)
            .with_context(|| format!("failed to bind TLS to {bind_addr}"))?
    } else {
        tracing::info!("Quartermaster starting on http://{bind_addr} (TLS disabled)");
        server_builder
            .bind(&bind_addr)
            .with_context(|| format!("failed to bind to {bind_addr}"))?
    };

    server.run().await.context("web server error")
}
