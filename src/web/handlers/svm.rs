use actix_session::Session;
use actix_web::web::{self, Data, Form, Json};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::rbac::Permission;
use crate::svm::metadata::{self, FieldMeta, InputType, SectionMeta, SECTIONS};
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
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
    nav: NavContext,
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

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Get a reference to the SVM manager, returning 404 if SVM is not installed.
fn require_svm(
    state: &AppState,
) -> actix_web::Result<&std::sync::Arc<parking_lot::RwLock<crate::svm::SvmManager>>> {
    state.svm.as_ref().ok_or_else(|| WebError::NotFound.into())
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn manager_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;

    // Users without SVM edit permission get redirected to the read-only view
    if !user.has_permission(Permission::SvmEdit) {
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/svm/view"))
            .finish());
    }

    let svm = require_svm(&state)?;
    let svm = svm.read();

    let tmpl = SvmManagerTemplate {
        user,
        flash: take_flash(&session),
        csrf_token: crate::web::csrf::get_or_create_token(&session),
        nav: NavContext::from_state(&state),
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
    require_permission(&user, Permission::SvmEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = require_svm(&state)?;
    {
        let mut svm = svm.write();
        svm.set_active_preset(&form.preset)
            .map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Switched to preset: {}", form.preset),
        FlashType::Success,
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
    require_permission(&user, Permission::SvmEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = require_svm(&state)?;
    {
        let mut svm = svm.write();
        svm.create_preset(&form.name).map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Created preset: {}", form.name),
        FlashType::Success,
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
    require_permission(&user, Permission::SvmEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = require_svm(&state)?;
    {
        let mut svm = svm.write();
        svm.duplicate_preset(&form.src, &form.dst)
            .map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Duplicated preset '{}' to '{}'", form.src, form.dst),
        FlashType::Success,
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
    require_permission(&user, Permission::SvmEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = require_svm(&state)?;
    {
        let mut svm = svm.write();
        svm.delete_preset(&form.name).map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Deleted preset: {}", form.name),
        FlashType::Success,
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
    require_permission(&user, Permission::SvmEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let svm = require_svm(&state)?;
    {
        let mut svm = svm.write();
        svm.reload_from_disk().map_err(WebError::from)?;
    }

    set_flash(&session, "Reloaded from disk", FlashType::Success);
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
    require_permission(&user, Permission::SvmEdit)?;

    let preset_name = path.into_inner();
    crate::svm::SvmManager::validate_preset_name(&preset_name)
        .map_err(|e| WebError::BadRequest(e.to_string()))?;

    let svm = require_svm(&state)?;
    let svm = svm.read();

    if !svm.list_presets().contains(&preset_name) {
        return Err(WebError::NotFound.into());
    }

    let preset_path = svm.preset_path(&preset_name);
    let json_content = std::fs::read_to_string(&preset_path)
        .map_err(|e| WebError::Internal(anyhow::anyhow!("Failed to read preset file: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{preset_name}.json\""),
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
    require_permission(&user, Permission::SvmEdit)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    crate::svm::SvmManager::validate_preset_name(&form.name)
        .map_err(|e| WebError::BadRequest(e.to_string()))?;

    let svm = require_svm(&state)?;

    let config: crate::svm::config::SvmConfig = serde_json::from_str(&form.json_content)
        .map_err(|e| WebError::BadRequest(format!("Invalid JSON: {e}")))?;

    {
        let mut svm = svm.write();
        svm.save_preset(&form.name, &config)
            .map_err(WebError::from)?;
    }

    set_flash(
        &session,
        &format!("Imported preset: {}", form.name),
        FlashType::Success,
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/svm"))
        .finish())
}

// ─── Config Editor (Task 7) ──────────────────────────────────────────────────

/// A group of fields sharing the same subgroup label, pre-grouped in the handler
/// so the template doesn't need mutable state tracking.
pub struct FieldGroup {
    pub label: Option<&'static str>,
    pub fields: Vec<&'static FieldMeta>,
}

fn group_fields(fields: &'static [FieldMeta]) -> Vec<FieldGroup> {
    let mut groups: Vec<FieldGroup> = Vec::new();
    for field in fields {
        let label = field.subgroup;
        match groups.last_mut() {
            Some(g) if g.label == label => g.fields.push(field),
            _ => groups.push(FieldGroup {
                label,
                fields: vec![field],
            }),
        }
    }
    groups
}

fn section_to_json(config: &crate::svm::SvmConfig, section: &str) -> Result<String, anyhow::Error> {
    let value = match section {
        "items" => serde_json::to_value(&config.items)?,
        "hideout" => serde_json::to_value(&config.hideout)?,
        "traders" => serde_json::to_value(&config.traders)?,
        "loot" => serde_json::to_value(&config.loot)?,
        "player" => serde_json::to_value(&config.player)?,
        "raids" => serde_json::to_value(&config.raids)?,
        "fleamarket" => serde_json::to_value(&config.fleamarket)?,
        "services" => serde_json::to_value(&config.services)?,
        "quests" => serde_json::to_value(&config.quests)?,
        "csm" => serde_json::to_value(&config.csm)?,
        "scav" => serde_json::to_value(&config.scav)?,
        "bots" => serde_json::to_value(&config.bots)?,
        "pmc" => serde_json::to_value(&config.pmc)?,
        "custom" => serde_json::to_value(&config.custom)?,
        _ => anyhow::bail!("unknown section: {section}"),
    };
    let json = serde_json::to_string(&value)?;
    // Escape </script> sequences to prevent XSS when injected into <script> blocks
    Ok(json.replace("</", "<\\/"))
}

#[derive(serde::Deserialize)]
pub struct EditorQuery {
    section: Option<String>,
}

#[derive(Template)]
#[template(path = "svm/editor.html")]
struct SvmEditorTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    active_preset: String,
    is_dirty: bool,
    sections: &'static [SectionMeta],
    section_key: String,
    field_groups: Vec<FieldGroup>,
    config_json: String,
}

pub async fn editor_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: web::Query<EditorQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SvmEdit)?;

    let svm = require_svm(&state)?;
    let svm = svm.read();

    let active_section = query.section.as_deref().unwrap_or("raids");
    let fields = metadata::fields_for_section(active_section).ok_or(WebError::NotFound)?;
    let field_groups = group_fields(fields);

    let config_json = section_to_json(svm.config(), active_section).map_err(WebError::from)?;

    let tmpl = SvmEditorTemplate {
        user,
        flash: take_flash(&session),
        csrf_token: crate::web::csrf::get_or_create_token(&session),
        nav: NavContext::from_state(&state),
        active_preset: svm.active_preset_name().to_string(),
        is_dirty: svm.is_dirty(),
        sections: SECTIONS,
        section_key: active_section.to_string(),
        field_groups,
        config_json,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "svm/partials/section.html")]
struct SvmSectionPartialTemplate {
    csrf_token: String,
    sections: &'static [SectionMeta],
    section_key: String,
    field_groups: Vec<FieldGroup>,
    config_json: String,
}

pub async fn section_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: web::Path<String>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SvmEdit)?;

    let section = path.into_inner();
    let svm = require_svm(&state)?;
    let svm = svm.read();

    let fields = metadata::fields_for_section(&section).ok_or(WebError::NotFound)?;
    let config_json = section_to_json(svm.config(), &section).map_err(WebError::from)?;

    let tmpl = SvmSectionPartialTemplate {
        csrf_token: crate::web::csrf::get_or_create_token(&session),
        sections: SECTIONS,
        section_key: section,
        field_groups: group_fields(fields),
        config_json,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn save_section(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: web::Path<String>,
    body: Json<serde_json::Value>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SvmEdit)?;

    let csrf = body
        .get("csrf_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !crate::web::csrf::validate_token(&session, csrf) {
        return Err(WebError::Forbidden.into());
    }

    let section = path.into_inner();
    let svm_lock = require_svm(&state)?.clone();

    // Clone current config + parse section update (no lock held during deser)
    let mut config = {
        let svm = svm_lock.read();
        svm.config().clone()
    };

    let mut section_data = body.into_inner();
    section_data.as_object_mut().map(|o| o.remove("csrf_token"));

    // Deserialize into the appropriate section struct
    match section.as_str() {
        "items" => config.items = serde_json::from_value(section_data)?,
        "hideout" => config.hideout = serde_json::from_value(section_data)?,
        "traders" => config.traders = serde_json::from_value(section_data)?,
        "loot" => config.loot = serde_json::from_value(section_data)?,
        "player" => config.player = serde_json::from_value(section_data)?,
        "raids" => config.raids = serde_json::from_value(section_data)?,
        "fleamarket" => config.fleamarket = serde_json::from_value(section_data)?,
        "services" => config.services = serde_json::from_value(section_data)?,
        "quests" => config.quests = serde_json::from_value(section_data)?,
        "csm" => config.csm = serde_json::from_value(section_data)?,
        "scav" => config.scav = serde_json::from_value(section_data)?,
        "bots" => config.bots = serde_json::from_value(section_data)?,
        "pmc" => config.pmc = serde_json::from_value(section_data)?,
        "custom" => config.custom = serde_json::from_value(section_data)?,
        _ => return Err(WebError::NotFound.into()),
    }

    // File I/O inside web::block to avoid blocking the async runtime
    web::block(move || {
        let mut svm = svm_lock.write();
        let preset = svm.active_preset_name().to_string();
        svm.save_preset(&preset, &config)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    set_flash(&session, "SVM config saved", FlashType::Success);
    Ok(HttpResponse::Ok().json(serde_json::json!({"ok": true})))
}

// ─── Player View (Task 8) ────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "svm/player_view.html")]
struct SvmPlayerViewTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    active_preset: String,
    sections: &'static [SectionMeta],
    active_section: String,
    field_groups: Vec<FieldGroup>,
    config_json: String,
}

pub async fn player_view(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: web::Query<EditorQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    // No capability check — any authenticated user can view

    let svm = require_svm(&state)?;
    let svm = svm.read();

    let active_section = query.section.as_deref().unwrap_or("raids");
    let fields = metadata::fields_for_section(active_section).ok_or(WebError::NotFound)?;
    let field_groups = group_fields(fields);
    let config_json = section_to_json(svm.config(), active_section).map_err(WebError::from)?;

    let tmpl = SvmPlayerViewTemplate {
        user,
        flash: take_flash(&session),
        csrf_token: crate::web::csrf::get_or_create_token(&session),
        nav: NavContext::from_state(&state),
        active_preset: svm.active_preset_name().to_string(),
        sections: SECTIONS,
        active_section: active_section.to_string(),
        field_groups,
        config_json,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}
