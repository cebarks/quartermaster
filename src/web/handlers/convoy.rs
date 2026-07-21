use actix_files::NamedFile;
use actix_session::Session;
use actix_web::{web, HttpRequest, HttpResponse};
use askama::Template;
use serde::{Deserialize, Serialize};

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
}

// ── Status Tab ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "convoy/partials/status.html")]
struct StatusPartialTemplate {
    reports: Vec<crate::db::convoy::SyncReportSummary>,
    activity: Vec<crate::db::convoy::SyncActivity>,
}

async fn render_preview_tab(state: &AppState) -> Result<String, WebError> {
    let Some((path, _etag)) = state.catalog_cache.get() else {
        return Ok("<p class=\"text-muted\">Catalog is being built...</p>".to_string());
    };

    let json = web::block(move || std::fs::read_to_string(path))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let tmpl = PreviewPartialTemplate { json };
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

/// GET /quma/convoy/catalog — serve cached catalog JSON with ETag
pub async fn catalog(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    if !state.config().convoy.as_ref().is_some_and(|c| c.enabled) {
        return Ok(HttpResponse::ServiceUnavailable().body("Convoy is not enabled"));
    }

    let Some((path, etag)) = state.catalog_cache.get() else {
        state.catalog_cache.invalidate();
        tracing::warn!("convoy catalog requested but cache not yet built, triggered rebuild");
        return Ok(HttpResponse::ServiceUnavailable()
            .body("Convoy catalog is being built, try again shortly"));
    };

    // Check If-None-Match for 304
    if let Some(if_none_match) = req.headers().get("if-none-match") {
        if let Ok(val) = if_none_match.to_str() {
            if val == etag {
                tracing::debug!("convoy catalog 304 (ETag match)");
                // Log catalog 304 event (fire-and-forget)
                let db = state.db.clone();
                let ip = req
                    .connection_info()
                    .realip_remote_addr()
                    .map(str::to_owned);
                tokio::task::spawn_blocking(move || {
                    let db = db.lock();
                    if let Err(e) = db.insert_sync_event("catalog_304", ip.as_deref(), None, None) {
                        tracing::warn!(err = %e, "failed to log convoy catalog 304 event");
                    }
                });
                return Ok(HttpResponse::NotModified().finish());
            }
        }
    }

    let body = web::block(move || std::fs::read(path))
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .map_err(actix_web::error::ErrorInternalServerError)?;

    tracing::debug!(bytes = body.len(), "served convoy catalog");

    let db_log = state.db.clone();
    let ip = req
        .connection_info()
        .realip_remote_addr()
        .map(str::to_owned);
    let body_len = body.len() as i64;
    tokio::task::spawn_blocking(move || {
        let db = db_log.lock();
        if let Err(e) = db.insert_sync_event("catalog_fetch", ip.as_deref(), None, Some(body_len)) {
            tracing::warn!(err = %e, "failed to log convoy catalog fetch event");
        }
    });

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header(("etag", etag))
        .insert_header(("cache-control", "no-cache"))
        .body(body))
}

#[derive(Deserialize)]
pub struct DownloadRequest {
    pub mods: Vec<i64>,
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
pub async fn download(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<DownloadRequest>,
) -> actix_web::Result<HttpResponse> {
    if body.mods.is_empty() {
        tracing::warn!("convoy download requested with empty mod list");
        return Ok(HttpResponse::BadRequest().body("no mods requested"));
    }

    tracing::info!(mod_ids = ?body.mods, count = body.mods.len(), "convoy batch download requested");

    let db = state.db.clone();
    let spt_dir = state.dirs.spt_server.clone();
    let mod_ids = body.mods.clone();

    let zip_path = web::block(move || {
        let db = db.lock();
        crate::convoy::download::get_or_build_convoy_zip(&db, &spt_dir, &mod_ids)
    })
    .await
    .map_err(|e| {
        tracing::error!(err = %e, "convoy batch download failed");
        actix_web::error::ErrorInternalServerError(e)
    })?
    .map_err(|e| {
        tracing::error!(err = %e, "convoy batch download failed");
        actix_web::error::ErrorInternalServerError(e)
    })?;

    let zip_len = std::fs::metadata(&zip_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    tracing::info!(bytes = zip_len, "convoy batch download served");

    let db_log = state.db.clone();
    let ip = req
        .connection_info()
        .realip_remote_addr()
        .map(str::to_owned);
    let mod_ids_json = serde_json::to_string(&body.mods).unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let db = db_log.lock();
        if let Err(e) = db.insert_sync_event(
            "download",
            ip.as_deref(),
            Some(&mod_ids_json),
            Some(zip_len),
        ) {
            tracing::warn!(err = %e, "failed to log convoy download event");
        }
    });

    Ok(NamedFile::open_async(&zip_path)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![actix_web::http::header::DispositionParam::Filename(
                "convoy-mods.zip".to_string(),
            )],
        })
        .into_response(&req))
}

/// GET /quma/convoy/mod/{mod_id}/archive — single mod download
pub async fn single_mod_archive(
    req: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let mod_id = path.into_inner();
    let db = state.db.clone();
    let spt_dir = state.dirs.spt_server.clone();

    tracing::info!(mod_id, "convoy single mod download requested");

    let zip_path = web::block(move || {
        let db = db.lock();
        crate::convoy::download::get_or_build_convoy_zip(&db, &spt_dir, &[mod_id])
    })
    .await
    .map_err(|e| {
        tracing::error!(mod_id, err = %e, "convoy single mod download failed");
        actix_web::error::ErrorInternalServerError(e)
    })?
    .map_err(|e| {
        tracing::error!(mod_id, err = %e, "convoy single mod download failed");
        actix_web::error::ErrorInternalServerError(e)
    })?;

    tracing::info!(mod_id, path = %zip_path.display(), "convoy single mod download served");

    Ok(NamedFile::open_async(&zip_path)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![actix_web::http::header::DispositionParam::Filename(
                format!("mod-{mod_id}.zip"),
            )],
        })
        .into_response(&req))
}

/// POST /quma/convoy/report — client sync report
/// Accepts raw body instead of web::Json because SPT's PostJson may not set Content-Type.
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

    let cache = state.catalog_cache.clone();
    web::block(move || cache.force_rehash())
        .await
        .map_err(WebError::from)?;

    tracing::info!(user = %user.username, "convoy force rehash triggered");

    set_flash(&session, "File checksums rehashed", FlashType::Success);
    Ok(HttpResponse::Ok()
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
}

async fn render_settings_tab(state: &AppState, csrf_token: &str) -> Result<String, WebError> {
    let config = state.config();
    let convoy = config.convoy.clone().unwrap_or_default();

    let tmpl = SettingsPartialTemplate {
        csrf_token: csrf_token.to_string(),
        enabled: convoy.enabled,
        exclusions: convoy.exclusions.join("\n"),
        bootstrap_forge_id: 2806,
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

    let was_enabled = state.config().convoy.as_ref().is_some_and(|c| c.enabled);

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

    if enabled && !was_enabled {
        tracing::info!(user = %user.username, "convoy enabled via settings");
        state.catalog_cache.invalidate();
    } else if !enabled && was_enabled {
        tracing::info!(user = %user.username, "convoy disabled via settings");
        state.catalog_cache.clear();
    } else if enabled {
        state.regenerate_convoy();
    }

    set_flash(&session, "Convoy settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/convoy?tab=settings"))
        .finish())
}
