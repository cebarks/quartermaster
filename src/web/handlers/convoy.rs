use std::collections::HashMap;

use actix_session::Session;
use actix_web::{web, HttpRequest, HttpResponse};
use askama::Template;
use serde::{Deserialize, Serialize};

use crate::config::slugify;
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
    "groups".to_string()
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
    let valid_tabs = ["groups", "mods", "preview", "status"];
    if !valid_tabs.contains(&active_tab) {
        active_tab = "groups";
    }
    if !convoy_enabled && active_tab != "groups" {
        active_tab = "groups";
    }

    let tab_content = match active_tab {
        "groups" => render_groups_tab(&state, &csrf_token).await?,
        "mods" => render_mods_tab(&state).await?,
        "preview" => render_preview_tab(&state).await?,
        "status" => render_status_tab(&state).await?,
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

// ── Groups Tab ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AvailableMod {
    id: i64,
    name: String,
}

struct GroupMember {
    id: i64,
    name: String,
    has_client_files: bool,
}

#[derive(Template)]
#[template(path = "convoy/partials/group_card.html")]
struct GroupCardTemplate {
    index: usize,
    predefined: String,
    name: String,
    slug: String,
    tier: String,
    exclude_headless: bool,
    members: Vec<GroupMember>,
    available_mods: Vec<AvailableMod>,
}

#[derive(Template)]
#[template(path = "convoy/partials/groups.html")]
struct GroupsPartialTemplate {
    csrf_token: String,
    groups: Vec<GroupCardTemplate>,
    next_index: usize,
}

fn mods_with_client_files(
    db: &crate::db::Database,
) -> Result<Vec<(i64, String, bool)>, anyhow::Error> {
    let mods = db.list_mods()?;
    let mut result = Vec::new();
    for m in &mods {
        let files = db.get_files_for_mod(m.id)?;
        let has_client = files.iter().any(|f| f.file_path.starts_with("BepInEx/"));
        result.push((m.id, m.name.clone(), has_client));
    }
    Ok(result)
}

async fn fetch_mods_with_client_files(
    state: &AppState,
) -> Result<Vec<(i64, String, bool)>, WebError> {
    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        mods_with_client_files(&db)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)
}

async fn render_groups_tab(state: &AppState, csrf_token: &str) -> Result<String, WebError> {
    let all_mods = fetch_mods_with_client_files(state).await?;

    let db = state.db.clone();
    let db_groups = web::block(move || {
        let db = db.lock();
        db.list_groups()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let mut assigned: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for group in &db_groups {
        let db = state.db.clone();
        let group_id = group.id;
        let members = web::block(move || {
            let db = db.lock();
            db.get_mods_in_group(group_id)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
        for m in members {
            assigned.insert(m.id);
        }
    }

    let mod_lookup: HashMap<i64, (&str, bool)> = all_mods
        .iter()
        .map(|(id, name, has_client)| (*id, (name.as_str(), *has_client)))
        .collect();

    let mut group_cards: Vec<GroupCardTemplate> = Vec::new();
    let mut card_index: usize = 0;

    // Virtual "default" card
    {
        let default_members: Vec<GroupMember> = all_mods
            .iter()
            .filter(|(id, _, has_client)| *has_client && !assigned.contains(id))
            .map(|(id, name, _)| GroupMember {
                id: *id,
                name: name.clone(),
                has_client_files: true,
            })
            .collect();

        group_cards.push(GroupCardTemplate {
            index: card_index,
            predefined: "default".to_string(),
            name: "Default".to_string(),
            slug: "default".to_string(),
            tier: "required".to_string(),
            exclude_headless: false,
            members: default_members,
            available_mods: Vec::new(),
        });
        card_index += 1;
    }

    // DB groups
    for group in db_groups {
        let db = state.db.clone();
        let group_id = group.id;
        let db_members = web::block(move || {
            let db = db.lock();
            db.get_mods_in_group(group_id)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        let members: Vec<GroupMember> = db_members
            .iter()
            .map(|m| {
                if let Some(&(name, has_client)) = mod_lookup.get(&m.id) {
                    GroupMember {
                        id: m.id,
                        name: name.to_string(),
                        has_client_files: has_client,
                    }
                } else {
                    GroupMember {
                        id: m.id,
                        name: format!("Mod #{}", m.id),
                        has_client_files: false,
                    }
                }
            })
            .collect();

        let member_ids: Vec<i64> = db_members.iter().map(|m| m.id).collect();

        let card_available: Vec<AvailableMod> = all_mods
            .iter()
            .filter(|(id, _, has_client)| {
                *has_client && (!assigned.contains(id) || member_ids.contains(id))
            })
            .map(|(id, name, _)| AvailableMod {
                id: *id,
                name: name.clone(),
            })
            .collect();

        group_cards.push(GroupCardTemplate {
            index: card_index,
            predefined: String::new(),
            name: group.name.clone(),
            slug: group.slug.clone(),
            tier: group.tier.clone(),
            exclude_headless: group.exclude_headless,
            members,
            available_mods: card_available,
        });
        card_index += 1;
    }

    let next_index = card_index;
    let tmpl = GroupsPartialTemplate {
        csrf_token: csrf_token.to_string(),
        groups: group_cards,
        next_index,
    };
    tmpl.render().map_err(WebError::from)
}

pub async fn groups_partial(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let html = render_groups_tab(&state, &csrf_token).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

#[derive(Deserialize)]
pub struct NewGroupQuery {
    #[serde(default)]
    index: usize,
}

pub async fn new_group_card(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: web::Query<NewGroupQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;
    let _csrf_token = crate::web::csrf::get_or_create_token(&session);

    let all_mods = fetch_mods_with_client_files(&state).await?;

    let db = state.db.clone();
    let db_groups = web::block(move || {
        let db = db.lock();
        db.list_groups()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let mut assigned: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for group in &db_groups {
        let db = state.db.clone();
        let group_id = group.id;
        let members = web::block(move || {
            let db = db.lock();
            db.get_mods_in_group(group_id)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
        for m in members {
            assigned.insert(m.id);
        }
    }

    let available: Vec<AvailableMod> = all_mods
        .iter()
        .filter(|(id, _, has_client)| *has_client && !assigned.contains(id))
        .map(|(id, name, _)| AvailableMod {
            id: *id,
            name: name.clone(),
        })
        .collect();

    let tmpl = GroupCardTemplate {
        index: query.index,
        predefined: String::new(),
        name: String::new(),
        slug: String::new(),
        tier: "required".to_string(),
        exclude_headless: false,
        members: Vec::new(),
        available_mods: available,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

#[derive(Deserialize)]
pub struct SaveGroupsRequest {
    csrf_token: String,
    groups: Vec<GroupData>,
}

#[derive(Deserialize)]
pub struct GroupData {
    name: String,
    slug: String,
    tier: String,
    exclude_headless: bool,
    members: Vec<i64>,
}

pub async fn save_groups(
    state: web::Data<AppState>,
    req: HttpRequest,
    session: Session,
    body: web::Json<SaveGroupsRequest>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;

    if !crate::web::csrf::validate_token(&session, &body.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let all_mods = fetch_mods_with_client_files(&state).await?;
    let client_mod_ids: std::collections::HashSet<i64> = all_mods
        .iter()
        .filter(|(_, _, has_client)| *has_client)
        .map(|(id, _, _)| *id)
        .collect();

    let mut processed_groups: Vec<(String, String, String, bool, Vec<i64>)> = Vec::new();

    for g in &body.groups {
        let name = g.name.trim();
        if name.is_empty() {
            continue;
        }

        let slug = if g.slug.trim().is_empty() {
            slugify(name)
        } else {
            g.slug.trim().to_string()
        };

        let tier = if g.tier == "optional" {
            "optional".to_string()
        } else {
            "required".to_string()
        };

        let members: Vec<i64> = g
            .members
            .iter()
            .copied()
            .filter(|id| client_mod_ids.contains(id))
            .collect();

        processed_groups.push((name.to_string(), slug, tier, g.exclude_headless, members));
    }

    // Dedup members
    {
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for (_, _, _, _, members) in &mut processed_groups {
            members.retain(|id| seen.insert(*id));
        }
    }

    let group_count = processed_groups.len();

    // Save to DB atomically (members are already DB ids)
    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.save_groups_atomic(&processed_groups)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    tracing::info!(
        user = %user.username,
        group_count,
        "convoy groups saved"
    );

    // Regenerate catalog
    state.regenerate_convoy();

    set_flash(&session, "Convoy groups saved", FlashType::Success);
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "redirect": "/quma/convoy?tab=groups"
    })))
}

// ── Mods Tab ──────────────────────────────────────────────────────────────

struct ModSummaryRow {
    name: String,
    group: Option<String>,
    tier: Option<String>,
    exclude_headless: bool,
}

#[derive(Template)]
#[template(path = "convoy/partials/mods.html")]
struct ModsPartialTemplate {
    mods: Vec<ModSummaryRow>,
}

async fn render_mods_tab(state: &AppState) -> Result<String, WebError> {
    let all_mods = fetch_mods_with_client_files(state).await?;

    let db = state.db.clone();
    let rows = web::block(move || {
        let db = db.lock();
        let groups = db.list_groups()?;

        let mut rows = Vec::new();
        for (forge_id, name, has_client) in &all_mods {
            if !has_client {
                continue;
            }

            // Find mod in DB
            let mod_opt = db
                .list_mods()?
                .into_iter()
                .find(|m| m.forge_mod_id == Some(*forge_id));

            let (group_name, tier, exclude_headless) = if let Some(m) = mod_opt {
                if let Some(group_id) = m.group_id {
                    if let Some(group) = groups.iter().find(|g| g.id == group_id) {
                        (
                            Some(group.name.clone()),
                            Some(group.tier.clone()),
                            group.exclude_headless,
                        )
                    } else {
                        (None, None, false)
                    }
                } else {
                    (None, None, false)
                }
            } else {
                (None, None, false)
            };

            rows.push(ModSummaryRow {
                name: name.clone(),
                group: group_name,
                tier,
                exclude_headless,
            });
        }

        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok::<_, anyhow::Error>(rows)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ModsPartialTemplate { mods: rows };
    tmpl.render().map_err(WebError::from)
}

pub async fn mods_partial(
    state: web::Data<AppState>,
    req: HttpRequest,
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ConvoyManage)?;

    let html = render_mods_tab(&state).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
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
    let Some((path, etag)) = state.catalog_cache.get() else {
        tracing::warn!("convoy catalog requested but cache not yet built");
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
                let ip = req.peer_addr().map(|a| a.ip().to_string());
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
    let ip = req.peer_addr().map(|a| a.ip().to_string());
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
    let spt_dir = state.spt_dir.clone();
    let mod_ids = body.mods.clone();

    let zip_bytes = web::block(move || {
        let db = db.lock();
        crate::convoy::download::build_download_zip(&db, &spt_dir, &mod_ids)
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

    tracing::info!(bytes = zip_bytes.len(), "convoy batch download served");

    let db_log = state.db.clone();
    let ip = req.peer_addr().map(|a| a.ip().to_string());
    let mod_ids_json = serde_json::to_string(&body.mods).unwrap_or_default();
    let zip_len = zip_bytes.len() as i64;
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

    Ok(HttpResponse::Ok()
        .content_type("application/zip")
        .insert_header((
            "content-disposition",
            "attachment; filename=\"convoy-mods.zip\"",
        ))
        .body(zip_bytes))
}

/// GET /quma/convoy/mod/{mod_id}/archive — single mod download
pub async fn single_mod_archive(
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let mod_id = path.into_inner();
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();

    tracing::info!(mod_id, "convoy single mod download requested");

    let zip_bytes = web::block(move || {
        let db = db.lock();
        crate::convoy::download::build_download_zip(&db, &spt_dir, &[mod_id])
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

    tracing::info!(
        mod_id,
        bytes = zip_bytes.len(),
        "convoy single mod download served"
    );

    Ok(HttpResponse::Ok()
        .content_type("application/zip")
        .insert_header((
            "content-disposition",
            format!("attachment; filename=\"mod-{mod_id}.zip\""),
        ))
        .body(zip_bytes))
}

/// POST /quma/convoy/report — client sync report
pub async fn report(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<SyncReportRequest>,
) -> actix_web::Result<HttpResponse> {
    let valid_results = ["up_to_date", "updated", "failed"];
    if !valid_results.contains(&body.result.as_str()) {
        return Ok(HttpResponse::BadRequest().body("invalid result value"));
    }

    if body.aid.is_empty() || body.aid.len() > 128 {
        return Ok(HttpResponse::BadRequest().body("invalid aid"));
    }

    let ip = req.peer_addr().map(|a| a.ip().to_string());
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
