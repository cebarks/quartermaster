use actix_session::Session;
use actix_web::web::{Data, Form, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::config::{
    ClientsConfig, Config, ConsoleLogConfig, FileLogConfig, LogFormat, LoggingConfig,
    RestartPolicy, RotationPolicy, WebLogConfig,
};
use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage};
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
    fika_installed: bool,
    modsync_installed: bool,
    config: Config,
    active_tab: String,
    console_format: String,
    file_format: String,
    file_rotation: String,
    restart_policy: String,
}

pub async fn settings_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<SettingsQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_users)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let config = Config::load(&state.config_path).map_err(WebError::from)?;

    let active_tab = query.tab.clone().unwrap_or_else(|| "web".to_string());

    // Format enum values for template
    let console_format = match config.logging.console.format {
        LogFormat::Text => "text",
        LogFormat::Json => "json",
    }
    .to_string();

    let file_format = match config.logging.file.format {
        LogFormat::Text => "text",
        LogFormat::Json => "json",
    }
    .to_string();

    let file_rotation = match config.logging.file.rotation {
        RotationPolicy::None => "none",
        RotationPolicy::Size => "size",
        RotationPolicy::Daily => "daily",
    }
    .to_string();

    let restart_policy = match config
        .clients
        .as_ref()
        .map(|c| &c.restart_policy)
        .unwrap_or(&RestartPolicy::Auto)
    {
        RestartPolicy::Auto => "auto",
        RestartPolicy::Manual => "manual",
    }
    .to_string();

    let tmpl = SettingsTemplate {
        user,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        config,
        active_tab,
        console_format,
        file_format,
        file_rotation,
        restart_policy,
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
    file_rotation: String,
    file_max_size_mb: u64,
    file_max_files: usize,
    web_buffer_size: usize,
}

#[derive(serde::Deserialize)]
pub struct ClientsSettingsForm {
    csrf_token: String,
    count: u32,
    install_dir: String,
    restart_policy: String,
    max_restart_attempts: u32,
    restart_backoff_cap: u64,
    base_udp_port: u16,
    image: String,
    isolated_paths: String,
}

pub async fn save_web_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<WebSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_users)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let tls_on = form.tls_enabled.is_some();
    let cert = form.tls_cert.trim();
    let key = form.tls_key.trim();

    if tls_on && (cert.is_empty() || key.is_empty()) {
        set_flash(
            &session,
            "TLS certificate and key paths are required when TLS is enabled",
            "error",
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=web"))
            .finish());
    }

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.web_bind = form.web_bind.trim().to_string();
    config.web_port = form.web_port;
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

    set_flash(
        &session,
        "Web settings saved. Restart required for changes to take effect.",
        "success",
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
    require_capability(&user, Role::can_manage_users)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let port: Option<u16> = if form.server_port.trim().is_empty() {
        None
    } else {
        match form.server_port.trim().parse::<u16>() {
            Ok(p) if p > 0 => Some(p),
            _ => {
                set_flash(&session, "Invalid server port", "error");
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

    set_flash(&session, "Server settings saved", "success");
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
    require_capability(&user, Role::can_manage_users)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.queue_changes = form.queue_changes.is_some();
    config.auto_drain_on_lifecycle = form.auto_drain_on_lifecycle.is_some();
    config.update_check_interval = form.update_check_interval;

    config.save(&state.config_path).map_err(WebError::from)?;

    set_flash(&session, "Queue settings saved", "success");
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
    require_capability(&user, Role::can_manage_users)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let ttl: Option<u64> = if form.forge_cache_ttl.trim().is_empty() {
        None
    } else {
        match form.forge_cache_ttl.trim().parse::<u64>() {
            Ok(t) => Some(t),
            Err(_) => {
                set_flash(&session, "Invalid cache TTL value", "error");
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/settings?tab=forge"))
                    .finish());
            }
        }
    };

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.forge_token = non_empty_opt(&form.forge_token);
    config.forge_cache_ttl = ttl;

    config.save(&state.config_path).map_err(WebError::from)?;

    set_flash(&session, "Forge settings saved", "success");
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
    require_capability(&user, Role::can_manage_users)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let valid_levels = ["trace", "debug", "info", "warn", "error"];
    if !valid_levels.contains(&form.log_level.as_str()) {
        set_flash(&session, "Invalid log level", "error");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=logging"))
            .finish());
    }

    let parse_format = |s: &str| -> LogFormat {
        match s {
            "json" => LogFormat::Json,
            _ => LogFormat::Text,
        }
    };

    let parse_rotation = |s: &str| -> RotationPolicy {
        match s {
            "size" => RotationPolicy::Size,
            "daily" => RotationPolicy::Daily,
            _ => RotationPolicy::None,
        }
    };

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.logging = LoggingConfig {
        level: form.log_level.trim().to_string(),
        console: ConsoleLogConfig {
            enabled: form.console_enabled.is_some(),
            format: parse_format(&form.console_format),
        },
        file: FileLogConfig {
            enabled: form.file_enabled.is_some(),
            path: form.file_path.trim().to_string(),
            format: parse_format(&form.file_format),
            rotation: parse_rotation(&form.file_rotation),
            max_size_mb: form.file_max_size_mb,
            max_files: form.file_max_files,
        },
        web: WebLogConfig {
            buffer_size: form.web_buffer_size,
        },
    };

    config.save(&state.config_path).map_err(WebError::from)?;

    set_flash(&session, "Logging settings saved", "success");
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=logging"))
        .finish())
}

pub async fn save_clients_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ClientsSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_users)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    if form.count > 0 && form.install_dir.trim().is_empty() {
        set_flash(
            &session,
            "Install directory is required when client count > 0",
            "error",
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=clients"))
            .finish());
    }

    let max_port = form.base_udp_port as u32 + form.count.saturating_sub(1);
    if form.count > 0 && max_port > 65535 {
        set_flash(
            &session,
            &format!(
                "Base UDP port ({}) + count ({}) exceeds port range (max would be {})",
                form.base_udp_port, form.count, max_port
            ),
            "error",
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=clients"))
            .finish());
    }

    let restart_policy = match form.restart_policy.as_str() {
        "manual" => RestartPolicy::Manual,
        _ => RestartPolicy::Auto,
    };

    let isolated: Vec<String> = form
        .isolated_paths
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let clients = ClientsConfig {
        count: form.count,
        install_dir: std::path::PathBuf::from(form.install_dir.trim()),
        restart_policy,
        max_restart_attempts: form.max_restart_attempts,
        restart_backoff_cap: form.restart_backoff_cap,
        base_udp_port: form.base_udp_port,
        image: form.image.trim().to_string(),
        isolated_paths: isolated,
    };

    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    config.clients = if form.count == 0 && form.install_dir.trim().is_empty() {
        None
    } else {
        Some(clients)
    };

    config.save(&state.config_path).map_err(WebError::from)?;

    set_flash(&session, "Clients settings saved", "success");
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=clients"))
        .finish())
}
