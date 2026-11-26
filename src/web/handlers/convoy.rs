use actix_session::Session;
use actix_web::{web, HttpRequest, HttpResponse};
use askama::Template;
use serde::{Deserialize, Serialize};

use crate::convoy::publisher::{Admission, DeliveryError, PublicationStatus, PublishMode};
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

// ── Page & Partials ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ConvoyQuery {
    #[serde(default = "default_tab")]
    tab: String,
}

fn default_tab() -> String {
    "preview".to_string()
}

#[derive(Template)]
#[template(path = "convoy.html")]
struct ConvoyTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    convoy_enabled: bool,
    active_tab: String,
    tab_content: String,
}

pub async fn convoy_page(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: web::Query<ConvoyQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let nav = NavContext::from_state(&state);
    let convoy_enabled = nav.convoy_enabled;

    let mut active_tab = query.tab.as_str();
    let valid_tabs = ["preview", "status", "settings"];
    if !valid_tabs.contains(&active_tab) {
        active_tab = "preview";
    }
    if !convoy_enabled && active_tab != "settings" {
        active_tab = "settings";
    }

    let tab_content = match active_tab {
        "preview" => render_preview_tab(&state).await?,
        "status" => render_status_tab(&state).await?,
        "settings" => render_settings_tab(&state, &csrf_token).await?,
        _ => "<p>Unknown tab</p>".to_string(),
    };

    let tmpl = ConvoyTemplate {
        user,
        flash,
        csrf_token,
        nav,
        convoy_enabled,
        active_tab: active_tab.to_string(),
        tab_content,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

// ── Preview Tab ───────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "convoy/partials/preview.html")]
struct PreviewPartialTemplate {
    json: String,
    publication: &'static str,
}

// ── Status Tab ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "convoy/partials/status.html")]
struct StatusPartialTemplate {
    reports: Vec<crate::db::convoy::SyncReportSummary>,
    activity: Vec<crate::db::convoy::SyncActivity>,
}

async fn render_preview_tab(state: &AppState) -> Result<String, WebError> {
    let publisher = &state.convoy_publisher;
    let (json, publication) = match publisher.admit(None, None).await {
        Ok(Admission::Manifest { json, .. }) => (
            String::from_utf8_lossy(&json).into_owned(),
            publication_status(publisher.status()),
        ),
        Err(DeliveryError::Disabled) => (String::new(), "Disabled"),
        Err(DeliveryError::Preparing) => (String::new(), "Preparing"),
        Err(DeliveryError::Busy) => (String::new(), "Busy — retry the preview shortly"),
        _ => (
            String::new(),
            "Failed — check the server log, then rehash and republish",
        ),
    };
    let tmpl = PreviewPartialTemplate { json, publication };
    tmpl.render().map_err(WebError::from)
}

pub async fn preview_partial(
    state: web::Data<AppState>,
    req: HttpRequest,
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;

    let html = render_preview_tab(&state).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

async fn render_status_tab(state: &AppState) -> Result<String, WebError> {
    let db = state.db.clone();
    let (reports, activity) = web::block(move || {
        let db = db.lock();
        let reports = db.get_latest_sync_reports()?;
        let activity = db.get_recent_sync_activity(50)?;
        Ok::<_, anyhow::Error>((reports, activity))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = StatusPartialTemplate { reports, activity };
    tmpl.render().map_err(WebError::from)
}

pub async fn status_partial(
    state: web::Data<AppState>,
    req: HttpRequest,
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;

    let html = render_status_tab(&state).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

// ── API Endpoints ─────────────────────────────────────────────────────────

fn publication_status(status: PublicationStatus) -> &'static str {
    match status {
        PublicationStatus::Disabled => "Disabled",
        PublicationStatus::Preparing => "Preparing",
        PublicationStatus::Ready { .. } => "Ready",
        PublicationStatus::Failed { .. } => {
            "Failed — check the server log, then rehash and republish"
        }
    }
}

pub async fn retired(state: web::Data<AppState>) -> HttpResponse {
    let enabled = state.config().convoy.as_ref().is_some_and(|c| c.enabled);
    let (mut response, body) = if enabled {
        (
            HttpResponse::Gone(),
            serde_json::json!({
                "code": "upgrade_required",
                "message": "Install the standalone Convoy v2 client with the game closed, paired with a Convoy v2 Quartermaster server.",
                "instructions": "https://github.com/cebarks/convoy#installation"
            }),
        )
    } else {
        (
            HttpResponse::ServiceUnavailable(),
            serde_json::json!({"code": "disabled"}),
        )
    };
    response
        .insert_header(("Cache-Control", "no-store"))
        .json(body)
}

#[derive(Deserialize)]
pub struct SyncReportRequest {
    pub aid: String,
    pub result: String,
    pub mods: Option<Vec<SyncReportMod>>,
    pub client_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct SyncReportMod {
    pub id: i64,
    pub version: String,
}

/// POST /quma/convoy/download — batched mod archive download
pub async fn report(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: String,
) -> actix_web::Result<HttpResponse> {
    let body: SyncReportRequest =
        serde_json::from_str(&body).map_err(actix_web::error::ErrorBadRequest)?;

    let valid_results = ["up_to_date", "updated", "failed"];
    if !valid_results.contains(&body.result.as_str()) {
        return Ok(HttpResponse::BadRequest().body("invalid result value"));
    }

    if body.aid.is_empty() || body.aid.len() > 128 {
        return Ok(HttpResponse::BadRequest().body("invalid aid"));
    }

    let ip = req
        .connection_info()
        .realip_remote_addr()
        .map(str::to_owned);
    let mods_snapshot = body
        .mods
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok());
    let aid = body.aid.clone();
    let result = body.result.clone();
    let client_version = body.client_version.clone();
    let error = body.error.clone();

    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.insert_sync_report(
            &aid,
            &result,
            mods_snapshot.as_deref(),
            client_version.as_deref(),
            error.as_deref(),
            ip.as_deref(),
        )
    })
    .await
    .map_err(|e| {
        tracing::error!(err = %e, "failed to store convoy sync report");
        actix_web::error::ErrorInternalServerError(e)
    })?
    .map_err(|e| {
        tracing::error!(err = %e, "failed to store convoy sync report");
        actix_web::error::ErrorInternalServerError(e)
    })?;

    tracing::info!(
        aid = %body.aid,
        result = %body.result,
        client_version = body.client_version.as_deref().unwrap_or("unknown"),
        "convoy sync report received"
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({"ok": true})))
}

/// POST /quma/convoy/rehash — force rehash all tracked client file checksums
pub async fn rehash(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;

    if !crate::web::csrf::validate_token(
        &session,
        req.headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
    ) {
        return Err(WebError::Forbidden.into());
    }

    if state.convoy_publisher.status() == PublicationStatus::Disabled {
        return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({"code": "disabled"})));
    }
    state.convoy_publisher.refresh(PublishMode::Rehash);
    tracing::info!(user = %user.username, "convoy rehash and republish queued");
    set_flash(
        &session,
        "Convoy rehash and republish queued",
        FlashType::Success,
    );
    Ok(HttpResponse::Accepted()
        .json(serde_json::json!({"ok": true, "redirect": "/quma/convoy?tab=settings"})))
}

// ── Settings Tab ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ConvoySettingsForm {
    csrf_token: String,
    enabled: Option<String>,
    exclusions: String,
}

#[derive(Template)]
#[template(path = "convoy/partials/settings.html")]
struct SettingsPartialTemplate {
    csrf_token: String,
    enabled: bool,
    exclusions: String,
    bootstrap_forge_id: i64,
    publication: &'static str,
}

async fn render_settings_tab(state: &AppState, csrf_token: &str) -> Result<String, WebError> {
    let config = state.config();
    let convoy = config.convoy.clone().unwrap_or_default();

    let tmpl = SettingsPartialTemplate {
        csrf_token: csrf_token.to_string(),
        enabled: convoy.enabled,
        exclusions: convoy.exclusions.join("\n"),
        bootstrap_forge_id: 2806,
        publication: publication_status(state.convoy_publisher.status()),
    };
    tmpl.render().map_err(WebError::from)
}

pub async fn settings_partial(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let html = render_settings_tab(&state, &csrf_token).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

pub async fn save_settings(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: web::Form<ConvoySettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let enabled = form.enabled.is_some();
    let exclusions: Vec<String> = form
        .exclusions
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    {
        let _guard = state.config_lock.lock();
        let mut config = crate::config::Config::load(&state.config_path).map_err(WebError::from)?;
        let convoy = config
            .convoy
            .get_or_insert_with(crate::config::ConvoyConfig::default);
        convoy.enabled = enabled;
        convoy.exclusions = exclusions;
        state.persist_config(&config)?;
    }

    set_flash(&session, "Convoy settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/convoy?tab=settings"))
        .finish())
}
