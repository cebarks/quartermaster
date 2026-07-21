pub mod api_auth;
pub mod auth;
pub mod csrf;
pub mod error;
pub mod flash;
pub mod handlers;
pub mod install;
pub mod integrity_cache;
pub mod invite;
pub mod mod_zip_cache;
pub mod nav;
pub mod proxy;
pub mod proxy_metrics;
pub mod proxy_ws;
pub mod raid_tracker;
pub mod scanner_guard;
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
use actix_web::{middleware, App, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use rust_embed::RustEmbed;

use actix_governor::governor::middleware::NoOpMiddleware;
use actix_governor::{Governor, GovernorConfig, GovernorConfigBuilder, PeerIpKeyExtractor};
use actix_web::middleware::from_fn;

use crate::config::Config;
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::forge::client::ForgeClient;
use crate::logging::{LogBroadcast, ReloadHandles};
use crate::spt::detect::SptInfo;
use crate::spt::game_data::GameData;

use state::AppState;

#[derive(RustEmbed)]
#[folder = "src/assets/"]
struct Assets;

fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

async fn serve_asset(path: web::Path<String>) -> HttpResponse {
    match Assets::get(&path) {
        Some(file) => HttpResponse::Ok()
            .content_type(content_type_for(&path))
            .insert_header(("Cache-Control", "public, max-age=3600"))
            .body(file.data.into_owned()),
        None => HttpResponse::NotFound().body("asset not found"),
    }
}

/// All state needed to launch the web server, bundled to avoid a 12-parameter function.
pub struct ServerContext {
    pub config: Config,
    pub config_handle: Arc<parking_lot::RwLock<Config>>,
    pub config_path: std::path::PathBuf,
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub dirs: QumaDirs,
    pub spt_info: SptInfo,
    pub log_broadcast: Arc<LogBroadcast>,
    pub reload_handles: Arc<ReloadHandles>,
    pub container_mgr: Option<Arc<crate::container::ContainerManager>>,
    pub client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    pub converging: Arc<std::sync::atomic::AtomicBool>,
    pub fika_installed: bool,
    pub log_level_counts: crate::logging::writer::LogLevelCounts,
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
    governor_conf: Option<GovernorConfig<PeerIpKeyExtractor, NoOpMiddleware>>,
    search_governor_conf: Option<GovernorConfig<PeerIpKeyExtractor, NoOpMiddleware>>,
) {
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
        // Static assets in their own scope — the explicit "/assets" prefix is
        // more specific than the catch-all web::scope("") that carries auth
        // middleware, so actix matches this first and serves files publicly.
        .service(web::scope("/assets").route("/{path:.*}", web::get().to(serve_asset)))
        // Auth routes (public)
        .route("/login", web::get().to(handlers::auth::login_page))
        .route("/logout", web::post().to(handlers::auth::logout))
        .route(
            "/health/integrity",
            web::get().to(handlers::mods::integrity_json),
        );

    // Rate-limited auth routes
    if enable_rate_limiting {
        let gov = governor_conf
            .as_ref()
            .expect("governor config required when rate limiting enabled");
        quma_scope = quma_scope
            .service(
                web::resource("/login")
                    .wrap(Governor::new(gov))
                    .route(web::post().to(handlers::auth::login_submit)),
            )
            .service(
                web::resource("/reset-password")
                    .wrap(Governor::new(gov))
                    .route(web::get().to(handlers::auth::reset_password_page))
                    .route(web::post().to(handlers::auth::reset_password_submit)),
            )
            .service(
                web::resource("/join")
                    .wrap(Governor::new(gov))
                    .route(web::get().to(handlers::join::join_page))
                    .route(web::post().to(handlers::join::join_submit)),
            )
            .service(
                web::resource("/join/mods.zip")
                    .wrap(Governor::new(gov))
                    .route(web::get().to(handlers::join::mod_archive)),
            )
            .service(
                web::resource("/join/bootstrap.sh")
                    .wrap(Governor::new(gov))
                    .route(web::get().to(handlers::join::bootstrap_bash)),
            )
            .service(
                web::resource("/join/bootstrap.ps1")
                    .wrap(Governor::new(gov))
                    .route(web::get().to(handlers::join::bootstrap_powershell)),
            )
            .service(
                web::resource("/setup/mods.zip")
                    .wrap(Governor::new(gov))
                    .route(web::get().to(handlers::setup::setup_mods_zip)),
            )
            .route("/convoy/catalog", web::get().to(handlers::convoy::catalog))
            .route(
                "/convoy/download",
                web::post().to(handlers::convoy::download),
            )
            .route(
                "/convoy/mod/{mod_id}/archive",
                web::get().to(handlers::convoy::single_mod_archive),
            )
            .route("/convoy/report", web::post().to(handlers::convoy::report));
    } else {
        // Same routes without rate limiting (for tests)
        quma_scope = quma_scope
            .route("/login", web::post().to(handlers::auth::login_submit))
            .route(
                "/reset-password",
                web::get().to(handlers::auth::reset_password_page),
            )
            .route(
                "/reset-password",
                web::post().to(handlers::auth::reset_password_submit),
            )
            .route("/join", web::get().to(handlers::join::join_page))
            .route("/join", web::post().to(handlers::join::join_submit))
            .route("/join/mods.zip", web::get().to(handlers::join::mod_archive))
            .route(
                "/join/bootstrap.sh",
                web::get().to(handlers::join::bootstrap_bash),
            )
            .route(
                "/join/bootstrap.ps1",
                web::get().to(handlers::join::bootstrap_powershell),
            )
            .route(
                "/setup/mods.zip",
                web::get().to(handlers::setup::setup_mods_zip),
            )
            .route("/convoy/catalog", web::get().to(handlers::convoy::catalog))
            .route(
                "/convoy/download",
                web::post().to(handlers::convoy::download),
            )
            .route(
                "/convoy/mod/{mod_id}/archive",
                web::get().to(handlers::convoy::single_mod_archive),
            )
            .route("/convoy/report", web::post().to(handlers::convoy::report));
    }

    // Build the API scope
    // api_auth_middleware must be outermost (last .wrap) so it runs first —
    // it checks X-Quma-Token before auth_middleware checks session cookies.
    let mut api_scope = web::scope("/api")
        .wrap(from_fn(auth::auth_middleware))
        .wrap(from_fn(api_auth::api_auth_middleware))
        .route("/events", web::get().to(crate::web::sse::events_stream))
        .route(
            "/mods/check-updates",
            web::get().to(handlers::mods::check_updates_partial),
        )
        .route(
            "/mods/refresh-updates",
            web::post().to(handlers::mods::refresh_updates),
        )
        .route(
            "/mods/update-status",
            web::get().to(handlers::mods::update_status_partial),
        )
        .route(
            "/mods/updates-carousel",
            web::get().to(handlers::mods::updates_carousel_partial),
        )
        .route(
            "/mods/list",
            web::get().to(handlers::mods::list_body_partial),
        )
        .route(
            "/mods/dep-tree",
            web::get().to(handlers::mods::dep_tree_partial),
        )
        // Mod groups API routes
        .route(
            "/mods/groups",
            web::get().to(handlers::mods::groups_partial),
        )
        .route(
            "/mods/groups/new",
            web::get().to(handlers::mods::new_group_card),
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
        .route(
            "/dashboard/players",
            web::get().to(handlers::dashboard::players_partial),
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
            "/mods/integrity/recheck",
            web::post().to(handlers::mods::integrity_recheck),
        )
        .route(
            "/mods/integrity/progress",
            web::get().to(handlers::mods::integrity_progress),
        )
        .route(
            "/headless/status-partial",
            web::get().to(handlers::clients::client_status_partial),
        )
        .route(
            "/headless/operations/{id}/partial",
            web::get().to(handlers::clients::operation_status_partial),
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
            "/logs/app/count",
            web::get().to(handlers::logs::app_logs_count),
        )
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
            "/logs/headless/containers",
            web::get().to(handlers::logs::headless_containers),
        )
        .route(
            "/logs/headless",
            web::get().to(handlers::logs::headless_logs_json),
        )
        .route(
            "/logs/headless/stream",
            web::get().to(handlers::logs::headless_logs_stream),
        )
        .route(
            "/mods/requests",
            web::get().to(handlers::requests::requests_tab),
        )
        .route(
            "/requests/tab",
            web::get().to(handlers::requests::request_tab_body),
        )
        .route(
            "/queue/content",
            web::get().to(handlers::queue::queue_content_partial),
        )
        .route(
            "/mods/{id}/addon-search",
            web::get().to(handlers::mods::search_addons),
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
        .route(
            "/requests/install-all",
            web::post().to(handlers::requests::install_all_approved),
        )
        .route(
            "/requests/{id}/install",
            web::post().to(handlers::requests::install_from_request),
        )
        .route(
            "/requests/{id}/reopen",
            web::post().to(handlers::requests::reopen_request),
        )
        .route(
            "/requests/{id}/history",
            web::get().to(handlers::requests::request_history),
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
        // Convoy API routes
        .route(
            "/convoy/preview",
            web::get().to(handlers::convoy::preview_partial),
        )
        .route(
            "/convoy/status",
            web::get().to(handlers::convoy::status_partial),
        )
        .route(
            "/convoy/settings",
            web::get().to(handlers::convoy::settings_partial),
        )
        .route(
            "/give-items/search",
            web::get().to(handlers::give_items::give_items_search),
        )
        .route(
            "/give-items/send",
            web::post().to(handlers::give_items::give_items_send),
        )
        .route(
            "/give-items/refresh",
            web::post().to(handlers::give_items::give_items_refresh),
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
                .route(
                    "/users/{id}/delete",
                    web::post().to(handlers::admin::delete_user),
                )
                .route(
                    "/users/{id}/link-profile",
                    web::post().to(handlers::admin::link_profile),
                )
                .route(
                    "/invites/{id}/delete",
                    web::post().to(handlers::admin::delete_invite),
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
        )
        // Headless JSON API (nested under /api/headless)
        .service(
            web::scope("/headless")
                .route(
                    "/status",
                    web::get().to(handlers::headless_api::api_headless_status),
                )
                .route(
                    "/{n}/start",
                    web::post().to(handlers::headless_api::api_client_start),
                )
                .route(
                    "/{n}/stop",
                    web::post().to(handlers::headless_api::api_client_stop),
                )
                .route(
                    "/{n}/restart",
                    web::post().to(handlers::headless_api::api_client_restart),
                )
                .route(
                    "/{n}/graceful-restart",
                    web::post().to(handlers::headless_api::api_client_graceful_restart),
                )
                .route("/scale", web::post().to(handlers::headless_api::api_scale))
                .route(
                    "/create",
                    web::post().to(handlers::headless_api::api_create),
                )
                .route(
                    "/{n}/delete",
                    web::post().to(handlers::headless_api::api_client_delete),
                )
                .route(
                    "/rebuild",
                    web::post().to(handlers::headless_api::api_rebuild),
                )
                .route(
                    "/converge",
                    web::post().to(handlers::headless_api::api_converge),
                )
                .route(
                    "/{n}/rename",
                    web::post().to(handlers::headless_api::api_client_rename),
                )
                .route(
                    "/{n}/image",
                    web::post().to(handlers::headless_api::api_client_set_image),
                )
                .route(
                    "/{n}/start-raid",
                    web::post().to(handlers::headless_api::api_client_start_raid),
                )
                .route(
                    "/{n}/logs",
                    web::get().to(handlers::headless_api::api_client_logs),
                )
                .route(
                    "/operations/{id}",
                    web::get().to(handlers::headless_api::api_operation_status),
                ),
        );

    // Rate-limited search routes
    if enable_rate_limiting {
        let search_gov = search_governor_conf
            .as_ref()
            .expect("search governor config required when rate limiting enabled");
        api_scope = api_scope
            .service(
                web::resource("/requests/search")
                    .wrap(Governor::new(search_gov))
                    .route(web::get().to(handlers::requests::search_mods)),
            )
            .service(
                web::resource("/mods/search")
                    .wrap(Governor::new(search_gov))
                    .route(web::get().to(handlers::mods::search_mods)),
            )
            .service(
                web::resource("/mods/{id}/compat-check")
                    .wrap(Governor::new(search_gov))
                    .route(web::get().to(handlers::mods::compat_check)),
            )
            .service(
                web::resource("/mods/{id}/versions")
                    .wrap(Governor::new(search_gov))
                    .route(web::get().to(handlers::mods::mod_versions)),
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
            )
            .route(
                "/mods/{id}/versions",
                web::get().to(handlers::mods::mod_versions),
            );
    }

    quma_scope = quma_scope.service(api_scope);

    // Authenticated routes (permission checks are per-handler via require_permission())
    let mut auth_scope = web::scope("").wrap(from_fn(auth::auth_middleware)).route(
        "/change-password",
        web::get().to(handlers::auth::change_password_page),
    );

    // Conditionally add rate-limited change-password POST
    if enable_rate_limiting {
        let gov = governor_conf
            .as_ref()
            .expect("governor config required when rate limiting enabled");
        auth_scope = auth_scope.service(
            web::resource("/change-password")
                .wrap(Governor::new(gov))
                .route(web::post().to(handlers::auth::change_password_submit)),
        );
    } else {
        auth_scope = auth_scope.route(
            "/change-password",
            web::post().to(handlers::auth::change_password_submit),
        );
    }

    auth_scope = auth_scope
        .route("/", web::get().to(handlers::dashboard::dashboard))
        .route("/setup", web::get().to(handlers::setup::setup_page))
        .route(
            "/setup/bootstrap.sh",
            web::get().to(handlers::setup::setup_bootstrap_bash),
        )
        .route(
            "/setup/bootstrap.ps1",
            web::get().to(handlers::setup::setup_bootstrap_powershell),
        )
        .route("/mods/{id}", web::get().to(handlers::mods::mod_detail))
        .route("/logs", web::get().to(handlers::logs::logs_page))
        .route("/metrics", web::get().to(handlers::metrics::metrics_page))
        .route("/admin", web::get().to(handlers::admin::admin_page))
        .route(
            "/give-items",
            web::get().to(handlers::give_items::give_items_page),
        )
        .route("/mods", web::get().to(handlers::mods::list_mods))
        .route("/files", web::get().to(handlers::mods::file_tracking_page))
        .route("/mods/groups", web::post().to(handlers::mods::save_groups))
        .route("/convoy", web::get().to(handlers::convoy::convoy_page))
        .route(
            "/convoy/settings",
            web::post().to(handlers::convoy::save_settings),
        )
        .route("/convoy/rehash", web::post().to(handlers::convoy::rehash))
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
        .service(
            web::resource("/svm/preset/import")
                .app_data(web::FormConfig::default().limit(1024 * 1024))
                .route(web::post().to(handlers::svm::import_preset)),
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
        .route(
            "/settings/fika",
            web::get().to(handlers::fika_settings::fika_settings_page),
        )
        .route(
            "/settings/fika",
            web::post().to(handlers::fika_settings::fika_settings_save),
        )
        .route("/headless", web::get().to(handlers::clients::headless_page))
        .route(
            "/headless/{n}",
            web::get().to(handlers::clients::client_detail),
        )
        .route("/stats", web::get().to(handlers::raids::stats_page))
        .route("/raids", web::get().to(handlers::raids::all_raids_page))
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
            "/mods/{id}/addons",
            web::get().to(handlers::mods::list_addons_partial),
        )
        .route(
            "/mods/{id}/addon-search",
            web::get().to(handlers::mods::search_addons),
        )
        .route(
            "/mods/{id}/install-addon",
            web::post().to(handlers::mods::install_addon),
        )
        .route(
            "/addons/{id}/update",
            web::post().to(handlers::mods::update_addon),
        )
        .route(
            "/addons/{id}/remove",
            web::post().to(handlers::mods::remove_addon),
        )
        .route(
            "/addons/{id}/toggle-disable",
            web::post().to(handlers::mods::toggle_addon_disable),
        )
        // Config management routes
        .route("/configs", web::get().to(handlers::configs::configs_list))
        .route(
            "/mods/{id}/config/{file}",
            web::get().to(handlers::configs::config_editor),
        )
        .route(
            "/mods/{id}/config/{file}",
            web::post().to(handlers::configs::config_save),
        )
        .route(
            "/mods/{id}/config/{file}/history",
            web::get().to(handlers::configs::config_history),
        )
        .route(
            "/mods/{id}/config/{file}/history/view",
            web::get().to(handlers::configs::config_history_view),
        )
        .route(
            "/mods/{id}/config/{file}/restore",
            web::post().to(handlers::configs::config_restore),
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
        .route(
            "/queue/{id}/cancel-reject",
            web::post().to(handlers::queue::cancel_and_reject_op),
        )
        .route("/queue/apply", web::post().to(handlers::queue::apply_queue))
        .route(
            "/headless/{n}/restart",
            web::post().to(handlers::clients::client_restart),
        )
        .route(
            "/headless/{n}/graceful-restart",
            web::post().to(handlers::clients::client_graceful_restart),
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
            "/headless/converge",
            web::post().to(handlers::clients::client_converge),
        )
        .route(
            "/headless/rebuild",
            web::post().to(handlers::clients::client_rebuild),
        )
        .route(
            "/headless/create",
            web::post().to(handlers::clients::client_create),
        )
        .route(
            "/headless/{n}/delete",
            web::post().to(handlers::clients::client_delete),
        )
        .route(
            "/headless/{n}/rename",
            web::post().to(handlers::clients::client_rename),
        )
        .route(
            "/headless/{n}/image",
            web::post().to(handlers::clients::client_set_image),
        )
        .route(
            "/headless/{n}/start-raid",
            web::post().to(handlers::clients::client_start_raid),
        )
        .route("/broadcast", web::post().to(handlers::dashboard::broadcast))
        .route(
            "/players/{profile_id}/message",
            web::post().to(handlers::dashboard::send_player_message),
        )
        .route("/notes", web::get().to(handlers::notes::notes_page))
        .route("/notes/new", web::get().to(handlers::notes::new_note_form))
        .route("/notes", web::post().to(handlers::notes::create_note))
        .route(
            "/notes/{id}/edit",
            web::get().to(handlers::notes::edit_note_form),
        )
        .route(
            "/notes/{id}/update",
            web::post().to(handlers::notes::update_note),
        )
        .route(
            "/notes/{id}/delete",
            web::post().to(handlers::notes::delete_note),
        );

    quma_scope = quma_scope.service(auth_scope);

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

pub async fn start_server(ctx: ServerContext, api_token: String) -> Result<()> {
    let ServerContext {
        config,
        config_handle,
        config_path,
        db,
        forge,
        dirs,
        spt_info,
        log_broadcast,
        reload_handles,
        container_mgr,
        client_states,
        converging,
        fika_installed,
        log_level_counts,
    } = ctx;
    let bind_addr = format!("{}:{}", config.web_bind, config.web_port);

    let session_key = Key::derive_from(config.session_secret.as_bytes());

    let (events_tx, _) = tokio::sync::broadcast::channel::<crate::web::sse::ServerEvent>(64);

    let game_data = Arc::new(GameData::load(&dirs).unwrap_or_else(|e| {
        tracing::warn!(err = %e, "failed to load SPT game data, lookups will return raw IDs");
        GameData::load_empty()
    }));

    // db is already Arc<Mutex<Database>> from serve.rs
    let db_arc = db;

    // Migrate disabled mods from old .disabled suffix scheme to stash directory
    if let Err(e) = crate::ops::migrate_disabled_to_stash(&db_arc.lock(), &dirs) {
        tracing::error!(err = %e, "failed to migrate disabled mods to stash");
    }

    crate::ops::cleanup_staging(&dirs);

    // Recover any interrupted async mod updates from a previous crash
    if let Err(e) = crate::ops::recover_pending_updates(&db_arc.lock(), &dirs) {
        tracing::error!(err = %e, "failed to recover pending updates on startup");
    }

    // Remove orphaned queued archives (no matching pending operation)
    crate::queue::sweep_orphaned_archives(&dirs, &db_arc.lock());

    let svm = crate::svm::SvmManager::detect(&dirs.spt_server)
        .map(|mgr| Arc::new(parking_lot::RwLock::new(mgr)));
    let svm_installed_flag = svm.is_some();
    if svm_installed_flag {
        tracing::info!("SVM detected — web config editor enabled");
    }

    let config_mgmt = crate::config_mgmt::ConfigManager::new(&dirs);

    let tls_enabled = config.tls_enabled;

    let proxy_client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build proxy HTTP client");

    let mod_zip_cache = crate::web::mod_zip_cache::ModZipCache::new(
        dirs.spt_server.clone(),
        db_arc.clone(),
        config_handle.clone(),
    );

    // Initialize FikaClient if fika.jsonc exists and has an API key
    let fika_client = if fika_installed {
        let fika_config_path = crate::fika::config::fika_config_path(&dirs.spt_server);
        match crate::fika::config::read_fika_config(&fika_config_path) {
            Ok(fika_config) if !fika_config.server.api_key.is_empty() => {
                let base_url = format!(
                    "https://{}:{}",
                    fika_config.server.spt.http.backend_ip,
                    fika_config.server.spt.http.backend_port
                );
                match crate::fika::client::FikaClient::new(&base_url, fika_config.server.api_key) {
                    Ok(client) => {
                        tracing::info!("FikaClient initialized");
                        Some(Arc::new(client))
                    }
                    Err(e) => {
                        tracing::warn!(err = %e, "failed to initialize FikaClient");
                        None
                    }
                }
            }
            Ok(_) => {
                tracing::warn!("Fika installed but api_key is empty");
                None
            }
            Err(e) => {
                tracing::warn!(err = %e, "failed to read fika.jsonc");
                None
            }
        }
    } else {
        None
    };

    let catalog_cache = crate::convoy::catalog::CatalogCache::new(
        dirs.spt_server.clone(),
        db_arc.clone(),
        config_handle.clone(),
    );

    let dirs = Arc::new(dirs);

    // ponytail: HeadlessService only when headless is configured (container_mgr + client_states both Some)
    let config_lock = Arc::new(parking_lot::Mutex::new(()));
    let fika_config_lock = Arc::new(parking_lot::Mutex::new(()));
    let headless_service =
        if let (Some(ref mgr), Some(ref states)) = (&container_mgr, &client_states) {
            Some(crate::headless::HeadlessService::new(
                mgr.as_ref().clone(),
                Arc::clone(&config_handle),
                config_path.clone(),
                Arc::clone(&config_lock),
                Arc::clone(&dirs),
                Arc::clone(&db_arc),
                Arc::clone(&converging),
                Arc::clone(states),
                fika_client.clone(),
                Arc::clone(&fika_config_lock),
                forge.clone(),
            ))
        } else {
            None
        };

    let app_state = web::Data::new(AppState {
        db: db_arc,
        forge,
        config: config_handle,
        config_path,
        config_lock,
        dirs: Arc::clone(&dirs),
        spt_info,
        tasks: crate::web::tasks::TaskTracker::new(events_tx.clone()),
        update_cache: crate::web::update_cache::UpdateCache::new(config.update_check_interval),
        integrity_cache: crate::web::integrity_cache::IntegrityCache::new(600),
        events: events_tx,
        log_broadcast,
        reload_handles,
        container_mgr,
        converging,
        fika_installed,
        svm,
        svm_installed: std::sync::atomic::AtomicBool::new(svm_installed_flag),
        config_mgmt,
        server_transition: Arc::new(parking_lot::Mutex::new(None)),
        game_data,
        proxy_metrics: crate::web::proxy_metrics::ProxyMetrics::new(),
        proxy_client,
        mod_zip_cache,
        log_level_counts,
        fika_client,
        fika_config_lock,
        catalog_cache,
        fika_items: Arc::new(parking_lot::Mutex::new(None)),
        headless_service,
    });

    // Pre-warm mod ZIP cache in background
    app_state.mod_zip_cache.invalidate();

    // One-time modsync-to-convoy migration
    {
        let config = app_state.config.read();
        let db = app_state.db.lock();
        if let Err(e) = crate::convoy::migrate::migrate_modsync_to_convoy(
            &config,
            &db,
            &app_state.dirs.spt_server,
        ) {
            tracing::error!("failed to migrate modsync groups to convoy: {e}");
        }
    }

    // Invalidate convoy catalog on startup if enabled
    if app_state
        .config
        .read()
        .convoy
        .as_ref()
        .is_some_and(|c| c.enabled)
    {
        app_state.catalog_cache.invalidate();
    }

    // Clean up old convoy sync data (30 day retention)
    {
        let db = app_state.db.lock();
        match db.cleanup_old_sync_data(30) {
            Ok((events, reports)) => {
                if events > 0 || reports > 0 {
                    tracing::info!(events, reports, "cleaned up old convoy sync data");
                }
            }
            Err(e) => tracing::warn!(err = %e, "failed to clean up old convoy sync data"),
        }
    }

    let governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(12)
        .burst_size(5)
        .finish()
        .expect("invalid governor config");

    let search_governor_conf = GovernorConfigBuilder::default()
        .seconds_per_request(6)
        .burst_size(10)
        .finish()
        .expect("invalid search governor config");

    let scanner_guard_data = if config.scanner_guard.enabled {
        let guard = scanner_guard::ScannerGuard::new(
            config.scanner_guard.threshold,
            std::time::Duration::from_secs(config.scanner_guard.ban_duration),
        );
        Some(web::Data::new(guard))
    } else {
        None
    };

    let api_token_state = web::Data::new(api_auth::ApiTokenState { token: api_token });

    let server_builder = HttpServer::new(move || {
        let gov = governor_conf.clone();
        let search_gov = search_governor_conf.clone();
        let mut app = App::new()
            .app_data(app_state.clone())
            .app_data(api_token_state.clone())
            .app_data(web::PayloadConfig::new(64 * 1024 * 1024));

        if let Some(ref guard) = scanner_guard_data {
            app = app.app_data(guard.clone());
        }

        // Scanner guard is first .wrap() (innermost), so it sees BoxBody
        // from handlers. Ban checks happen before the handler; TracingLogger
        // (outer) still logs blocked requests. When ScannerGuard is not in
        // app_data, the middleware is a no-op.
        app.wrap(from_fn(scanner_guard::scanner_guard_middleware))
            .wrap(middleware::NormalizePath::new(
                middleware::TrailingSlash::MergeOnly,
            ))
            .wrap(tracing_actix_web::TracingLogger::default())
            .wrap(
                middleware::DefaultHeaders::new()
                    .add(("X-Content-Type-Options", "nosniff"))
                    .add(("X-Frame-Options", "DENY")),
            )
            .configure(|cfg| {
                configure_app(
                    cfg,
                    session_key.clone(),
                    tls_enabled,
                    true,
                    Some(gov),
                    Some(search_gov),
                )
            })
    });

    let server_builder = if let Some(workers) = config.web_workers {
        let workers = workers.clamp(1, 256);
        server_builder.workers(workers)
    } else {
        server_builder
    };

    let server = if config.tls_enabled {
        let tls_config = crate::tls::load_or_generate_tls_config(&config, &dirs)
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
