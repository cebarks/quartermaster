pub mod auth;
pub mod csrf;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod invite;
pub mod nav;
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
use crate::logging::{LogBroadcast, ReloadHandles};
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

/// All state needed to launch the web server, bundled to avoid a 12-parameter function.
pub struct ServerContext {
    pub config: Config,
    pub config_path: std::path::PathBuf,
    pub db: Database,
    pub forge: ForgeClient,
    pub spt_dir: std::path::PathBuf,
    pub spt_info: SptInfo,
    pub log_broadcast: Arc<LogBroadcast>,
    pub reload_handles: Arc<ReloadHandles>,
    pub container_mgr: Option<Arc<crate::container::ContainerManager>>,
    pub client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    pub converging: Arc<std::sync::atomic::AtomicBool>,
    pub fika_installed: bool,
    pub modsync_installed: bool,
}

/// Configure the web application routes and middleware.
///
/// This function registers all routes, session middleware, auth middleware, and rate limiting
/// on the provided `ServiceConfig`. It's extracted into a standalone function so it can be
/// reused by both the production server and integration tests.
///
/// # Arguments
///
/// * `cfg` - The actix-web ServiceConfig to register routes on
/// * `session_key` - Session encryption key
/// * `tls_enabled` - Whether TLS is enabled (affects cookie secure flag)
/// * `enable_rate_limiting` - Whether to apply rate limiting (tests pass false to avoid flakiness)
pub fn configure_app(
    cfg: &mut web::ServiceConfig,
    session_key: Key,
    tls_enabled: bool,
    enable_rate_limiting: bool,
) {
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

    // Build the /quma scope with all routes and middleware
    let mut quma_scope = web::scope("/quma")
        .wrap(
            SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
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
        .route("/logout", web::post().to(handlers::auth::logout));

    // Rate-limited auth routes
    if enable_rate_limiting {
        quma_scope = quma_scope
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
            .service(
                web::resource("/reset-password")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::get().to(handlers::auth::reset_password_page))
                    .route(web::post().to(handlers::auth::reset_password_submit)),
            );
    } else {
        // Same routes without rate limiting (for tests)
        quma_scope = quma_scope
            .route("/login", web::post().to(handlers::auth::login_submit))
            .route("/register", web::get().to(handlers::auth::register_page))
            .route("/register", web::post().to(handlers::auth::register_submit))
            .route(
                "/reset-password",
                web::get().to(handlers::auth::reset_password_page),
            )
            .route(
                "/reset-password",
                web::post().to(handlers::auth::reset_password_submit),
            );
    }

    // Build the API scope
    let mut api_scope = web::scope("/api")
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
        // Dashboard partials
        .route(
            "/dashboard/server",
            web::get().to(handlers::dashboard::server_partial),
        )
        .route(
            "/dashboard/mods",
            web::get().to(handlers::dashboard::mods_partial),
        )
        .route(
            "/dashboard/headless-status",
            web::get().to(handlers::clients::dashboard_clients_status_partial),
        )
        // Metrics partials
        .route(
            "/metrics/proxy",
            web::get().to(handlers::metrics::proxy_metrics_partial),
        )
        // Mods integrity partial
        .route(
            "/mods/integrity",
            web::get().to(handlers::mods::integrity_partial),
        )
        .route(
            "/headless/status",
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
        .route(
            "/mods/requests",
            web::get().to(handlers::requests::requests_tab),
        )
        .route(
            "/queue/content",
            web::get().to(handlers::queue::queue_content_partial),
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
        // SVM API routes
        .route(
            "/svm/edit/{section}",
            web::get().to(handlers::svm::section_partial),
        )
        .route(
            "/svm/edit/{section}",
            web::post().to(handlers::svm::save_section),
        )
        // ModSync API routes
        .route(
            "/modsync/settings",
            web::get().to(handlers::modsync::settings_partial),
        )
        .route(
            "/modsync/groups",
            web::get().to(handlers::modsync::groups_partial),
        )
        .route(
            "/modsync/groups/new",
            web::get().to(handlers::modsync::new_group_card),
        )
        .route(
            "/modsync/mods",
            web::get().to(handlers::modsync::mods_partial),
        )
        .route(
            "/modsync/preview",
            web::get().to(handlers::modsync::preview_partial),
        )
        // Admin API (requires can_manage_users via scoped middleware)
        .service(
            web::scope("/admin")
                .wrap(from_fn(handlers::admin::admin_middleware))
                .route("/users", web::get().to(handlers::admin::admin_users))
                .route("/invites", web::get().to(handlers::admin::admin_invites))
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
                .route("/invites", web::post().to(handlers::admin::create_invite))
                .route("/roles", web::get().to(handlers::admin::admin_roles))
                .route(
                    "/roles",
                    web::post().to(handlers::admin::create_role_handler),
                )
                .route(
                    "/roles/{name}/permissions",
                    web::post().to(handlers::admin::update_role_handler),
                )
                .route(
                    "/roles/{name}/delete",
                    web::post().to(handlers::admin::delete_role_handler),
                ),
        );

    // Rate-limited search routes
    if enable_rate_limiting {
        api_scope = api_scope
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
            );
    } else {
        // Same routes without rate limiting
        api_scope = api_scope
            .route(
                "/requests/search",
                web::get().to(handlers::requests::search_mods),
            )
            .route("/mods/search", web::get().to(handlers::mods::search_mods))
            .route(
                "/mods/{id}/compat-check",
                web::get().to(handlers::mods::compat_check),
            );
    }

    quma_scope = quma_scope.service(api_scope);

    // Authenticated routes (admin checks are per-handler via require_admin())
    quma_scope = quma_scope.service(
        web::scope("")
            .wrap(from_fn(auth::auth_middleware))
            .route(
                "/change-password",
                web::get().to(handlers::auth::change_password_page),
            )
            .service(
                web::resource("/change-password")
                    .wrap(Governor::new(&governor_conf))
                    .route(web::post().to(handlers::auth::change_password_submit)),
            )
            .route("/", web::get().to(handlers::dashboard::dashboard))
            .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
            .route("/logs", web::get().to(handlers::logs::logs_page))
            .route("/metrics", web::get().to(handlers::metrics::metrics_page))
            .route("/admin", web::get().to(handlers::admin::admin_page))
            .route("/mods", web::get().to(handlers::mods::list_mods))
            .route("/files", web::get().to(handlers::mods::file_tracking_page))
            .route("/modsync", web::get().to(handlers::modsync::modsync_page))
            .route(
                "/modsync/settings",
                web::post().to(handlers::modsync::save_settings),
            )
            .route(
                "/modsync/groups",
                web::post().to(handlers::modsync::save_groups),
            )
            .route(
                "/modsync/mods",
                web::post().to(handlers::modsync::save_mods),
            )
            .route("/svm", web::get().to(handlers::svm::manager_page))
            .route("/svm/view", web::get().to(handlers::svm::player_view))
            .route("/svm/edit", web::get().to(handlers::svm::editor_page))
            .route(
                "/svm/preset/switch",
                web::post().to(handlers::svm::switch_preset),
            )
            .route(
                "/svm/preset/create",
                web::post().to(handlers::svm::create_preset),
            )
            .route(
                "/svm/preset/duplicate",
                web::post().to(handlers::svm::duplicate_preset),
            )
            .route(
                "/svm/preset/delete",
                web::post().to(handlers::svm::delete_preset),
            )
            .route(
                "/svm/preset/export/{name}",
                web::get().to(handlers::svm::export_preset),
            )
            .route(
                "/svm/preset/import",
                web::post().to(handlers::svm::import_preset),
            )
            .route(
                "/svm/reload",
                web::post().to(handlers::svm::reload_from_disk),
            )
            .route(
                "/settings",
                web::get().to(handlers::settings::settings_page),
            )
            .route(
                "/settings/web",
                web::post().to(handlers::settings::save_web_settings),
            )
            .route(
                "/settings/server",
                web::post().to(handlers::settings::save_server_settings),
            )
            .route(
                "/settings/queue",
                web::post().to(handlers::settings::save_queue_settings),
            )
            .route(
                "/settings/forge",
                web::post().to(handlers::settings::save_forge_settings),
            )
            .route(
                "/settings/logging",
                web::post().to(handlers::settings::save_logging_settings),
            )
            .route(
                "/settings/headless",
                web::post().to(handlers::settings::save_headless_settings),
            )
            .route("/headless", web::get().to(handlers::clients::client_list))
            .route(
                "/headless/{n}",
                web::get().to(handlers::clients::client_detail),
            )
            .route("/stats", web::get().to(handlers::raids::stats_page))
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
                "/mods/{id}/backups",
                web::get().to(handlers::backup::mod_backups_partial),
            )
            .route(
                "/mods/{id}/backup",
                web::post().to(handlers::backup::create_mod_backup),
            )
            .route(
                "/backups/{id}/restore",
                web::post().to(handlers::backup::restore_backup),
            )
            .route(
                "/admin/backups",
                web::get().to(handlers::backup::admin_backups_page),
            )
            .route(
                "/admin/backups/full",
                web::post().to(handlers::backup::create_full_backup),
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
                "/headless/{n}/restart",
                web::post().to(handlers::clients::client_restart),
            )
            .route(
                "/headless/{n}/stop",
                web::post().to(handlers::clients::client_stop),
            )
            .route(
                "/headless/{n}/start",
                web::post().to(handlers::clients::client_start),
            )
            .route(
                "/headless/scale",
                web::post().to(handlers::clients::client_scale),
            )
            .route(
                "/headless/create",
                web::post().to(handlers::clients::client_create),
            )
            .route(
                "/headless/{n}/delete",
                web::post().to(handlers::clients::client_delete),
            )
            // Redirect old /clients URLs to /headless
            .route(
                "/clients",
                web::get().to(|| async {
                    HttpResponse::MovedPermanently()
                        .insert_header(("Location", "/quma/headless"))
                        .finish()
                }),
            )
            .route(
                "/clients/{n}",
                web::get().to(|path: web::Path<u32>| async move {
                    HttpResponse::MovedPermanently()
                        .insert_header((
                            "Location",
                            format!("/quma/headless/{}", path.into_inner()),
                        ))
                        .finish()
                }),
            ),
    );

    cfg.service(quma_scope);

    // Root redirect and default proxy handler
    cfg.route(
        "/",
        web::get().to(|| async {
            HttpResponse::Found()
                .insert_header(("Location", "/quma/"))
                .finish()
        }),
    );
    cfg.default_service(web::to(proxy::proxy_handler));
}

pub async fn start_server(ctx: ServerContext) -> Result<()> {
    let ServerContext {
        config,
        config_path,
        db,
        forge,
        spt_dir,
        spt_info,
        log_broadcast,
        reload_handles,
        container_mgr,
        client_states,
        converging,
        fika_installed,
        modsync_installed,
    } = ctx;
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

    let svm =
        crate::svm::SvmManager::detect(&spt_dir).map(|mgr| Arc::new(parking_lot::RwLock::new(mgr)));
    let svm_installed_flag = svm.is_some();
    if svm_installed_flag {
        tracing::info!("SVM detected — web config editor enabled");
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
        config: Arc::new(parking_lot::RwLock::new(config.clone())),
        config_path,
        config_lock: parking_lot::Mutex::new(()),
        spt_dir,
        spt_info,
        tasks: crate::web::tasks::TaskTracker::new(events_tx.clone()),
        update_cache: crate::web::update_cache::UpdateCache::new(config.update_check_interval),
        events: events_tx,
        log_broadcast,
        reload_handles,
        container_mgr,
        client_states,
        converging,
        fika_installed,
        modsync_installed: std::sync::atomic::AtomicBool::new(modsync_installed),
        svm,
        svm_installed: std::sync::atomic::AtomicBool::new(svm_installed_flag),
        server_transition: Arc::new(parking_lot::Mutex::new(None)),
        game_data,
        proxy_metrics: crate::web::proxy_metrics::ProxyMetrics::new(),
        proxy_client,
    });

    let server_builder = HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .app_data(web::PayloadConfig::new(64 * 1024 * 1024))
            .wrap(middleware::NormalizePath::new(
                middleware::TrailingSlash::MergeOnly,
            ))
            .wrap(tracing_actix_web::TracingLogger::default())
            .configure(|cfg| configure_app(cfg, session_key.clone(), tls_enabled, true))
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
