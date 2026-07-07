use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use askama::Template;

use crate::config_mgmt::ModConfigSet;
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

// -- Templates --

#[derive(Template)]
#[template(path = "configs/list.html")]
struct ConfigListTemplate {
    user: SessionUser,
    nav: NavContext,
    flash: Option<FlashMessage>,
    csrf_token: String,
    config_sets: Vec<ModConfigSet>,
}

#[derive(Template)]
#[template(path = "configs/editor.html")]
struct ConfigEditorTemplate {
    user: SessionUser,
    nav: NavContext,
    flash: Option<FlashMessage>,
    csrf_token: String,
    mod_id: i64,
    mod_name: String,
    filename: String,
    content: String,
    can_edit: bool,
    server_running: bool,
}

#[derive(Template)]
#[template(path = "configs/history.html")]
struct ConfigHistoryTemplate {
    user: SessionUser,
    nav: NavContext,
    flash: Option<FlashMessage>,
    csrf_token: String,
    mod_id: i64,
    mod_name: String,
    filename: String,
    entries: Vec<crate::config_mgmt::HistoryEntry>,
    can_restore: bool,
}

#[derive(Template)]
#[template(path = "configs/history_view.html")]
struct ConfigHistoryViewTemplate {
    user: SessionUser,
    nav: NavContext,
    flash: Option<FlashMessage>,
    csrf_token: String,
    mod_id: i64,
    mod_name: String,
    filename: String,
    rev: String,
    content: String,
    can_restore: bool,
}

// -- Path extractors --

#[derive(serde::Deserialize)]
pub struct ConfigFilePath {
    pub id: i64,
    pub file: String,
}

#[derive(serde::Deserialize)]
pub struct RestoreForm {
    pub csrf_token: String,
    pub rev: String,
}

#[derive(serde::Deserialize)]
pub struct SaveForm {
    pub csrf_token: String,
    pub content: String,
}

#[derive(serde::Deserialize)]
pub struct HistoryViewQuery {
    pub rev: String,
}

// -- Helpers --

/// Validate config filename to prevent path traversal.
fn validate_filename(name: &str) -> Result<(), WebError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.is_empty() {
        return Err(WebError::BadRequest("Invalid config filename".to_string()));
    }
    Ok(())
}

/// Find the mod directory name on disk for a given mod ID.
fn find_mod_dir_for_id(state: &AppState, mod_id: i64) -> Result<(String, String), WebError> {
    let db = state.db.lock();
    let mod_info = db
        .get_mod(mod_id)
        .map_err(WebError::from)?
        .ok_or(WebError::NotFound)?;
    let mod_dir = state
        .config_mgmt
        .find_mod_dir(&mod_info.name)
        .map_err(WebError::from)?
        .ok_or(WebError::NotFound)?;
    Ok((mod_info.name, mod_dir))
}

// -- Handlers --

/// GET /configs — list all mods with config files
pub async fn configs_list(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);
    let db = state.db.clone();
    let config_mgmt = state.config_mgmt.clone();

    let config_sets = web::block(move || {
        let db = db.lock();
        config_mgmt.discover_configs(&db)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ConfigListTemplate {
        user,
        nav,
        flash,
        csrf_token,
        config_sets,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

/// GET /mods/{id}/config/{file} — config editor page
pub async fn config_editor(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<ConfigFilePath>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);
    let path = path.into_inner();

    validate_filename(&path.file)?;

    let (mod_name, mod_dir) = find_mod_dir_for_id(&state, path.id)?;
    let config_rel = std::path::Path::new(&path.file);
    let content = state
        .config_mgmt
        .read_config(&mod_dir, config_rel)
        .map_err(WebError::from)?;

    let can_edit = user.has_permission(Permission::ModsConfigEdit);

    let server_running = if let Some(ref mgr) = state.container_mgr {
        let config = state.config_cloned();
        if let Some(ref container) = config.server_container {
            mgr.is_running(container).await.unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    let tmpl = ConfigEditorTemplate {
        user,
        nav,
        flash,
        csrf_token,
        mod_id: path.id,
        mod_name,
        filename: path.file,
        content,
        can_edit,
        server_running,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

/// POST /mods/{id}/config/{file} — save config file
pub async fn config_save(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<ConfigFilePath>,
    form: Form<SaveForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsConfigEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let path = path.into_inner();
    validate_filename(&path.file)?;

    let (_, mod_dir) = find_mod_dir_for_id(&state, path.id)?;
    let config_rel = std::path::Path::new(&path.file).to_path_buf();
    let content = form.into_inner().content;
    let username = user.username.clone();
    let mod_id = path.id;
    let file = path.file.clone();
    let config_mgmt = state.config_mgmt.clone();

    let changed =
        web::block(move || config_mgmt.save_config(&mod_dir, &config_rel, &content, &username))
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;

    if changed {
        set_flash(
            &session,
            "Config saved. Restart the server for changes to take effect.",
            FlashType::Success,
        );
    } else {
        set_flash(&session, "No changes detected.", FlashType::Info);
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{mod_id}/config/{file}")))
        .finish())
}

/// GET /mods/{id}/config/{file}/history — config version history
pub async fn config_history(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<ConfigFilePath>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);
    let path = path.into_inner();

    validate_filename(&path.file)?;

    let (mod_name, mod_dir) = find_mod_dir_for_id(&state, path.id)?;
    let config_rel = std::path::Path::new(&path.file);
    let entries = state
        .config_mgmt
        .history(&mod_dir, config_rel)
        .map_err(WebError::from)?;

    let can_restore = user.has_permission(Permission::ModsConfigEdit);

    let tmpl = ConfigHistoryTemplate {
        user,
        nav,
        flash,
        csrf_token,
        mod_id: path.id,
        mod_name,
        filename: path.file,
        entries,
        can_restore,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

/// GET /mods/{id}/config/{file}/history/view?rev=abc123 — view content at revision
pub async fn config_history_view(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<ConfigFilePath>,
    query: Query<HistoryViewQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);
    let path = path.into_inner();

    validate_filename(&path.file)?;

    let (mod_name, mod_dir) = find_mod_dir_for_id(&state, path.id)?;
    let config_rel = std::path::Path::new(&path.file);
    let content = state
        .config_mgmt
        .content_at_rev(&mod_dir, config_rel, &query.rev)
        .map_err(WebError::from)?;

    let can_restore = user.has_permission(Permission::ModsConfigEdit);

    let tmpl = ConfigHistoryViewTemplate {
        user,
        nav,
        flash,
        csrf_token,
        mod_id: path.id,
        mod_name,
        filename: path.file,
        rev: query.rev.clone(),
        content,
        can_restore,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

/// POST /mods/{id}/config/{file}/restore — restore to previous version
pub async fn config_restore(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<ConfigFilePath>,
    form: Form<RestoreForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsConfigEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let path = path.into_inner();
    validate_filename(&path.file)?;

    let (_, mod_dir) = find_mod_dir_for_id(&state, path.id)?;
    let config_rel = std::path::Path::new(&path.file).to_path_buf();
    let rev = form.into_inner().rev;
    let username = user.username.clone();
    let mod_id = path.id;
    let file = path.file.clone();
    let config_mgmt = state.config_mgmt.clone();

    web::block(move || config_mgmt.restore_config(&mod_dir, &config_rel, &rev, &username))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    set_flash(
        &session,
        "Config restored. Restart the server for changes to take effect.",
        FlashType::Success,
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{mod_id}/config/{file}")))
        .finish())
}
