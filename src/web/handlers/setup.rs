use actix_session::Session;
use actix_web::{web, HttpRequest, HttpResponse};
use askama::Template;

use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::handlers::join::{
    build_mod_zip, build_spt_server_url, generate_bash_script, generate_powershell_script,
    DEFAULT_SERVER_NAME, FIKA_INSTALLER_URL,
};
use crate::web::mod_zip_cache::filter_setup_zip_files;
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    user: SessionUser,
    nav: NavContext,
    flash: Option<FlashMessage>,
    csrf_token: String,
    server_name: String,
    spt_version: String,
    spt_server_url: String,
    fika_installer_url: &'static str,
    fika_installed: bool,
    modsync_installed: bool,
    external_url_configured: bool,
}

pub async fn setup_page(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let nav = NavContext::from_state(&state);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let config = state.config.read();
    let server_name = config
        .server_name
        .clone()
        .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
    let external_url = config.external_url.clone();
    drop(config);

    let external_url_configured = external_url.as_ref().is_some_and(|u| !u.is_empty());
    let spt_server_url = match &external_url {
        Some(url) => build_spt_server_url(url),
        None => String::new(),
    };

    let tmpl = SetupTemplate {
        user,
        nav,
        flash,
        csrf_token,
        server_name,
        spt_version: state.spt_info.spt_version.clone(),
        spt_server_url,
        fika_installer_url: FIKA_INSTALLER_URL,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        external_url_configured,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn setup_bootstrap_bash(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    require_auth(&req)?;

    let config = state.config.read();
    let external_url = match &config.external_url {
        Some(url) if !url.is_empty() => url.clone(),
        _ => {
            return Ok(HttpResponse::ServiceUnavailable()
                .content_type("text/plain")
                .body("Bootstrap not configured: external_url is required"));
        }
    };
    let server_name = config
        .server_name
        .clone()
        .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
    drop(config);

    let spt_server_url = build_spt_server_url(&external_url);

    let archive_url = format!("{}/quma/setup/mods.zip", external_url.trim_end_matches('/'));
    let script = generate_bash_script(&server_name, &archive_url, &spt_server_url);

    Ok(HttpResponse::Ok()
        .content_type("text/x-shellscript")
        .insert_header((
            "content-disposition",
            "attachment; filename=\"quma-bootstrap.sh\"",
        ))
        .insert_header(("referrer-policy", "no-referrer"))
        .body(script))
}

pub async fn setup_bootstrap_powershell(
    state: web::Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    require_auth(&req)?;

    let config = state.config.read();
    let external_url = match &config.external_url {
        Some(url) if !url.is_empty() => url.clone(),
        _ => {
            return Ok(HttpResponse::ServiceUnavailable()
                .content_type("text/plain")
                .body("Bootstrap not configured: external_url is required"));
        }
    };
    let server_name = config
        .server_name
        .clone()
        .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
    drop(config);

    let spt_server_url = build_spt_server_url(&external_url);

    let archive_url = format!("{}/quma/setup/mods.zip", external_url.trim_end_matches('/'));
    let script = generate_powershell_script(&server_name, &archive_url, &spt_server_url);

    Ok(HttpResponse::Ok()
        .content_type("text/plain")
        .insert_header((
            "content-disposition",
            "attachment; filename=\"quma-bootstrap.ps1\"",
        ))
        .insert_header(("referrer-policy", "no-referrer"))
        .body(script))
}

// TODO(debt): This endpoint is intentionally unauthenticated so bootstrap scripts can
// curl it without session cookies. Add a signed/expiring download token when this matters.
pub async fn setup_mods_zip(state: web::Data<AppState>) -> actix_web::Result<HttpResponse> {
    // Serve cached file if available
    if let Some(path) = state.mod_zip_cache.get() {
        let bytes = web::block(move || std::fs::read(path))
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;

        return Ok(HttpResponse::Ok()
            .content_type("application/zip")
            .insert_header((
                "content-disposition",
                "attachment; filename=\"quma-mods.zip\"",
            ))
            .body(bytes));
    }

    // Fallback: startup warmup hasn't finished yet — build synchronously.
    // The background warmup will populate the cache for subsequent requests.
    let setup_zip_config = {
        let config = state.config.read();
        config.setup_zip.clone()
    };

    let db = state.db.clone();
    let mut files = web::block(move || {
        let db = db.lock();
        db.get_all_enabled_mod_files()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    files = filter_setup_zip_files(files, &setup_zip_config);

    if files.is_empty() {
        return Ok(HttpResponse::ServiceUnavailable()
            .content_type("text/plain")
            .body("No mods are installed on this server"));
    }

    let spt_dir = state.spt_dir.clone();
    let zip_bytes = web::block(move || build_mod_zip(&spt_dir, &files))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::Internal)?;

    Ok(HttpResponse::Ok()
        .content_type("application/zip")
        .insert_header((
            "content-disposition",
            "attachment; filename=\"quma-mods.zip\"",
        ))
        .body(zip_bytes))
}
