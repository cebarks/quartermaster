use actix_session::Session;
use actix_web::web::{Data, Form, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::client::ClientState;
use crate::config::{
    Config, ConsoleFormat, ConsoleLogConfig, FileFormat, FileLogConfig, HeadlessConfig,
    HeadlessDisplayServer, LoggingConfig, RestartPolicy, RotationPolicy, WebLogConfig,
};
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

fn non_empty_opt(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(serde::Deserialize)]
pub struct SettingsQuery {
    pub tab: Option<String>,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    config: Config,
    active_tab: String,
    console_format: String,
    file_format: String,
    file_rotation: String,
    restart_policy: String,
    display_server: String,
    has_forge_token: bool,
    headless_clients: Vec<ClientState>,
    headless_converging: bool,
    headless_target_count: u32,
}

pub async fn settings_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<SettingsQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let config = Config::load(&state.config_path).map_err(WebError::from)?;

    const VALID_TABS: &[&str] = &["web", "server", "queue", "forge", "logging", "headless"];
    let active_tab = query
        .tab
        .as_deref()
        .filter(|t| VALID_TABS.contains(t))
        .unwrap_or("web")
        .to_string();

    // Format enum values for template
    let console_format = config.logging.console.format.to_string();
    let file_format = config.logging.file.format.to_string();
    let file_rotation = config.logging.file.rotation.to_string();
    let restart_policy = config
        .headless
        .as_ref()
        .map(|c| c.restart_policy.to_string())
        .unwrap_or_else(|| RestartPolicy::Auto.to_string());

    let display_server = config
        .headless
        .as_ref()
        .map(|c| match c.display_server {
            HeadlessDisplayServer::Gamescope => "gamescope",
            HeadlessDisplayServer::Xvfb => "xvfb",
        })
        .unwrap_or("gamescope")
        .to_string();

    let has_forge_token = config.forge_token.is_some();

    let headless_clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };
    let headless_converging = state.converging.load(std::sync::atomic::Ordering::Relaxed);
    let headless_target_count = config
        .headless
        .as_ref()
        .map(|h| h.client_count())
        .unwrap_or(0);

    let tmpl = SettingsTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        config,
        active_tab,
        console_format,
        file_format,
        file_rotation,
        restart_policy,
        display_server,
        has_forge_token,
        headless_clients,
        headless_converging,
        headless_target_count,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct WebSettingsForm {
    csrf_token: String,
    web_bind: String,
    web_port: u16,
    external_url: String,
    server_name: String,
    tls_enabled: Option<String>,
    tls_cert: String,
    tls_key: String,
    proxy_enabled: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ServerSettingsForm {
    csrf_token: String,
    server_container: String,
    server_host: String,
    server_port: String,
    auto_start_server: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct QueueSettingsForm {
    csrf_token: String,
    queue_changes: Option<String>,
    auto_drain_on_lifecycle: Option<String>,
    update_check_interval: u64,
}

#[derive(serde::Deserialize)]
pub struct ForgeSettingsForm {
    csrf_token: String,
    forge_token: String,
    forge_cache_ttl: String,
    clear_forge_token: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct LoggingSettingsForm {
    csrf_token: String,
    log_level: String,
    console_enabled: Option<String>,
    console_format: String,
    file_enabled: Option<String>,
    file_path: String,
    file_format: String,
    file_level: String,
    file_rotation: String,
    file_max_size_mb: u64,
    file_max_files: usize,
    web_buffer_size: usize,
    web_level: String,
    web_retention_days: u64,
    web_max_entries: u64,
}

#[derive(serde::Deserialize)]
pub struct HeadlessSettingsForm {
    csrf_token: String,
    install_dir: String,
    restart_policy: String,
    max_restart_attempts: u32,
    restart_backoff_cap: u64,
    base_udp_port: u16,
    image: String,
    isolated_paths: String,
    display_server: String,
}

pub async fn save_web_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<WebSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let tls_on = form.tls_enabled.is_some();
    let cert = form.tls_cert.trim();
    let key = form.tls_key.trim();

    if tls_on && (cert.is_empty() != key.is_empty()) {
        set_flash(
            &session,
            "TLS certificate and key paths must both be set, or both empty (auto-generated)",
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=web"))
            .finish());
    }

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.web_bind = form.web_bind.trim().to_string();
    config.web_port = form.web_port;
    config.external_url = non_empty_opt(form.external_url.trim_end_matches('/'));
    config.server_name = non_empty_opt(&form.server_name);
    config.tls_enabled = tls_on;
    config.tls_cert = if cert.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(cert))
    };
    config.tls_key = if key.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(key))
    };
    config.proxy_enabled = form.proxy_enabled.is_some();

    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }

    set_flash(
        &session,
        "Web settings saved. Restart required for changes to take effect.",
        FlashType::Success,
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=web"))
        .finish())
}

pub async fn save_server_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ServerSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let port: Option<u16> = if form.server_port.trim().is_empty() {
        None
    } else {
        match form.server_port.trim().parse::<u16>() {
            Ok(p) if p > 0 => Some(p),
            _ => {
                set_flash(&session, "Invalid server port", FlashType::Error);
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/settings?tab=server"))
                    .finish());
            }
        }
    };

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.server_container = non_empty_opt(&form.server_container);
    config.server_host = non_empty_opt(&form.server_host);
    config.server_port = port;
    config.auto_start_server = form.auto_start_server.is_some();

    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }

    set_flash(&session, "Server settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=server"))
        .finish())
}

pub async fn save_queue_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<QueueSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.queue_changes = form.queue_changes.is_some();
    config.auto_drain_on_lifecycle = form.auto_drain_on_lifecycle.is_some();
    config.update_check_interval = form.update_check_interval;

    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }

    set_flash(&session, "Queue settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=queue"))
        .finish())
}

pub async fn save_forge_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ForgeSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let ttl: Option<u64> = if form.forge_cache_ttl.trim().is_empty() {
        Some(0)
    } else {
        match form.forge_cache_ttl.trim().parse::<u64>() {
            Ok(t) => Some(t),
            Err(_) => {
                set_flash(&session, "Invalid cache TTL value", FlashType::Error);
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/settings?tab=forge"))
                    .finish());
            }
        }
    };

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;

    if form.clear_forge_token.is_some() {
        config.forge_token = None;
    } else {
        let token_input = form.forge_token.trim();
        if !token_input.is_empty() {
            config.forge_token = Some(token_input.to_string());
        }
        // else: leave config.forge_token as-is (unchanged from disk)
    }

    config.forge_cache_ttl = ttl;

    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }

    set_flash(&session, "Forge settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=forge"))
        .finish())
}

pub async fn save_logging_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<LoggingSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let valid_levels = ["trace", "debug", "info", "warn", "error"];
    if !valid_levels.contains(&form.log_level.as_str()) {
        set_flash(&session, "Invalid log level", FlashType::Error);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=logging"))
            .finish());
    }

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.logging = LoggingConfig {
        level: form.log_level.trim().to_string(),
        console: ConsoleLogConfig {
            enabled: form.console_enabled.is_some(),
            format: form
                .console_format
                .parse()
                .unwrap_or(ConsoleFormat::Compact),
        },
        file: FileLogConfig {
            enabled: form.file_enabled.is_some(),
            path: form.file_path.trim().to_string(),
            format: form.file_format.parse().unwrap_or(FileFormat::Json),
            level: form.file_level.trim().to_string(),
            rotation: form.file_rotation.parse().unwrap_or(RotationPolicy::Daily),
            max_size_mb: form.file_max_size_mb,
            max_files: form.file_max_files,
        },
        web: WebLogConfig {
            buffer_size: form.web_buffer_size,
            level: form.web_level.trim().to_string(),
            retention_days: form.web_retention_days,
            max_entries: form.web_max_entries,
        },
    };

    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }

    let filter = crate::logging::resolve_log_filter(&config.logging, 0, None);
    state
        .reload_handles
        .reconfigure(&config.logging, &filter, Some(&state.spt_dir));

    set_flash(
        &session,
        "Logging settings saved and applied",
        FlashType::Success,
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=logging"))
        .finish())
}

pub async fn save_headless_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<HeadlessSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let restart_policy: RestartPolicy = form.restart_policy.parse().unwrap_or(RestartPolicy::Auto);

    let isolated: Vec<String> = form
        .isolated_paths
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let headless = HeadlessConfig {
        install_dir: std::path::PathBuf::from(form.install_dir.trim()),
        restart_policy,
        max_restart_attempts: form.max_restart_attempts,
        restart_backoff_cap: form.restart_backoff_cap,
        base_udp_port: form.base_udp_port,
        image: form.image.trim().to_string(),
        isolated_paths: isolated,
        clients: Vec::new(), // clients managed via create/delete, not settings
        ..HeadlessConfig::default()
    };

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    // Preserve fields not editable from the web form
    let existing = config.headless.as_ref();
    let mut final_config = headless;
    final_config.clients = existing.map(|h| h.clients.clone()).unwrap_or_default();
    final_config.runner = existing.map(|h| h.runner.clone()).unwrap_or_default();
    final_config.ntsync = existing.map(|h| h.ntsync).unwrap_or(true);
    final_config.esync = existing.map(|h| h.esync).unwrap_or(false);
    final_config.fsync = existing.map(|h| h.fsync).unwrap_or(false);
    final_config.display_server = match form.display_server.as_str() {
        "xvfb" => HeadlessDisplayServer::Xvfb,
        _ => HeadlessDisplayServer::Gamescope,
    };
    final_config.save_log_on_exit = existing.map(|h| h.save_log_on_exit).unwrap_or(true);
    final_config.enable_log_purge = existing.map(|h| h.enable_log_purge).unwrap_or(false);
    final_config.overwrite_fika = existing.map(|h| h.overwrite_fika).unwrap_or(true);
    config.headless = if form.install_dir.trim().is_empty() {
        None
    } else {
        Some(final_config)
    };

    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }

    set_flash(&session, "Headless settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=headless"))
        .finish())
}
