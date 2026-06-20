use actix_session::Session;
use actix_web::web::{Data, Form, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::config::{Config, LogFormat, RestartPolicy, RotationPolicy};
use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::state::AppState;

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
    #[allow(dead_code)] // Used in later tasks
    config: Config,
    active_tab: String,
    #[allow(dead_code)] // Used in later tasks
    console_format: String,
    #[allow(dead_code)] // Used in later tasks
    file_format: String,
    #[allow(dead_code)] // Used in later tasks
    file_rotation: String,
    #[allow(dead_code)] // Used in later tasks
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

    let config = Config::load_with_env(&state.config_path).unwrap_or_default();

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

// Stub form structs (will be filled in by later tasks)
#[derive(serde::Deserialize)]
pub struct WebSettingsForm {
    #[allow(dead_code)]
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct ServerSettingsForm {
    #[allow(dead_code)]
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct QueueSettingsForm {
    #[allow(dead_code)]
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct ForgeSettingsForm {
    #[allow(dead_code)]
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct LoggingSettingsForm {
    #[allow(dead_code)]
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct ClientsSettingsForm {
    #[allow(dead_code)]
    csrf_token: String,
}

// Stub save handlers (will be implemented by later tasks)
pub async fn save_web_settings(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _form: Form<WebSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=web"))
        .finish())
}

pub async fn save_server_settings(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _form: Form<ServerSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=server"))
        .finish())
}

pub async fn save_queue_settings(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _form: Form<QueueSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=queue"))
        .finish())
}

pub async fn save_forge_settings(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _form: Form<ForgeSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=forge"))
        .finish())
}

pub async fn save_logging_settings(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _form: Form<LoggingSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=logging"))
        .finish())
}

pub async fn save_clients_settings(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _form: Form<ClientsSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=clients"))
        .finish())
}
