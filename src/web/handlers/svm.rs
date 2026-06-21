use actix_session::Session;
use actix_web::web::{self, Data, Form};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage};
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

// ─── Template Structs ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "svm/manager.html")]
struct SvmManagerTemplate<'a> {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    fika_installed: bool,
    modsync_installed: bool,
    svm_installed: bool,
    active_preset: &'a str,
    presets: Vec<String>,
    is_dirty: bool,
    unknown_field_count: usize,
}

// ─── Form Structs ────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct PresetSwitchForm {
    csrf_token: String,
    preset: String,
}

#[derive(serde::Deserialize)]
pub struct PresetCreateForm {
    csrf_token: String,
    name: String,
}

#[derive(serde::Deserialize)]
pub struct PresetDuplicateForm {
    csrf_token: String,
    src: String,
    dst: String,
}

#[derive(serde::Deserialize)]
pub struct PresetDeleteForm {
    csrf_token: String,
    name: String,
}

#[derive(serde::Deserialize)]
pub struct PresetImportForm {
    csrf_token: String,
    name: String,
    json_content: String,
}

#[derive(serde::Deserialize)]
pub struct ReloadForm {
    csrf_token: String,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn manager_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    let svm = svm.read();

    let tmpl = SvmManagerTemplate {
        user,
        flash: take_flash(&session),
        csrf_token: crate::web::csrf::get_or_create_token(&session),
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        svm_installed: true,
        active_preset: svm.active_preset_name(),
        presets: svm.list_presets().to_vec(),
        is_dirty: svm.is_dirty(),
        unknown_field_count: svm.unknown_fields().len(),
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn switch_preset(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<PresetSwitchForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    {
        let mut svm = svm.write();
        svm.set_active_preset(&form.preset)
            .map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Switched to preset: {}", form.preset),
        "success",
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

pub async fn create_preset(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<PresetCreateForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    {
        let mut svm = svm.write();
        svm.create_preset(&form.name).map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Created preset: {}", form.name),
        "success",
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

pub async fn duplicate_preset(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<PresetDuplicateForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    {
        let mut svm = svm.write();
        svm.duplicate_preset(&form.src, &form.dst)
            .map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Duplicated preset '{}' to '{}'", form.src, form.dst),
        "success",
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

pub async fn delete_preset(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<PresetDeleteForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    {
        let mut svm = svm.write();
        svm.delete_preset(&form.name).map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Deleted preset: {}", form.name),
        "success",
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

pub async fn reload_from_disk(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ReloadForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    {
        let mut svm = svm.write();
        svm.reload_from_disk().map_err(WebError::from)?;
    }

    set_flash(&session, "Reloaded from disk", "success");
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

pub async fn export_preset(
    state: Data<AppState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;

    let preset_name = path.into_inner();
    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;
    let _svm = svm.read();

    // Read the preset file from disk
    let preset_path = state
        .spt_dir
        .join("user/mods/DrakiaXYZ-SVM/Presets")
        .join(format!("{}.json", preset_name));
    let json_content = std::fs::read_to_string(&preset_path)
        .map_err(|e| WebError::Internal(anyhow::anyhow!("Failed to read preset file: {}", e)))?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{}.json\"", preset_name),
        ))
        .body(json_content))
}

pub async fn import_preset(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<PresetImportForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = state.svm.as_ref().ok_or(WebError::NotFound)?;

    // Parse JSON to validate it
    let config: crate::svm::config::SvmConfig = serde_json::from_str(&form.json_content)
        .map_err(|e| WebError::Internal(anyhow::anyhow!("Invalid JSON: {}", e)))?;

    {
        let mut svm = svm.write();
        svm.save_preset(&form.name, &config)
            .map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Imported preset: {}", form.name),
        "success",
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

// ─── Stubs for Task 7 ────────────────────────────────────────────────────────

#[allow(dead_code)]
pub async fn editor_page(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    todo!("Task 7: editor page")
}

#[allow(dead_code)]
pub async fn section_partial(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _path: web::Path<String>,
) -> actix_web::Result<HttpResponse> {
    todo!("Task 7: section partial")
}

#[allow(dead_code)]
pub async fn save_section(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
    _path: web::Path<String>,
    _form: web::Form<std::collections::HashMap<String, String>>,
) -> actix_web::Result<HttpResponse> {
    todo!("Task 7: save section")
}

// ─── Stubs for Task 8 ────────────────────────────────────────────────────────

#[allow(dead_code)]
pub async fn player_view(
    _state: Data<AppState>,
    _req: HttpRequest,
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    todo!("Task 8: player view")
}
