use actix_session::Session;
use actix_web::web::{self, Data, Form};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::config::Config;
use crate::db::mods::InstalledMod;
use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

struct ModSyncModEntry {
    mod_info: InstalledMod,
    has_client_files: bool,
    override_enforced: Option<bool>,
    override_silent: Option<bool>,
    override_restart_required: Option<bool>,
    override_enabled: Option<bool>,
}

#[derive(Template)]
#[template(path = "modsync.html")]
struct ModSyncTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    fika_installed: bool,
    modsync_installed: bool,
    #[allow(dead_code)]
    svm_installed: bool,
    modsync_managed: bool,
    enforced: bool,
    silent: bool,
    restart_required: bool,
    extra_sync_paths: String,
    exclusions: String,
    mods: Vec<ModSyncModEntry>,
    has_client_mods: bool,
}

pub async fn modsync_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    // Re-read config from disk to pick up changes saved by save_settings
    // (state.config is immutable behind Arc, so it goes stale after saves)
    let live_config = crate::config::Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config.or_else(|| state.config.modsync.clone());

    let db = state.db.clone();
    let ms_config_for_block = ms_config.clone();
    let mods = web::block(move || {
        let db = db.lock();
        let all_mods = db.list_mods()?;
        let mut entries = Vec::new();
        for m in all_mods {
            let files = db.get_files_for_mod(m.id)?;
            let has_client = files.iter().any(|f| f.file_path.starts_with("BepInEx/"));
            let forge_id_str = m.forge_mod_id.to_string();
            let overrides = ms_config_for_block
                .as_ref()
                .and_then(|c| c.overrides.get(&forge_id_str));
            entries.push(ModSyncModEntry {
                mod_info: m,
                has_client_files: has_client,
                override_enforced: overrides.and_then(|o| o.enforced),
                override_silent: overrides.and_then(|o| o.silent),
                override_restart_required: overrides.and_then(|o| o.restart_required),
                override_enabled: overrides.and_then(|o| o.enabled),
            });
        }
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (enforced, silent, restart_required, extra_sync_paths, exclusions) =
        if let Some(ref ms) = ms_config {
            (
                ms.enforced,
                ms.silent,
                ms.restart_required,
                ms.extra_sync_paths.join("\n"),
                ms.exclusions.join("\n"),
            )
        } else {
            (true, false, true, String::new(), String::new())
        };

    let tmpl = ModSyncTemplate {
        user,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        svm_installed: state.is_svm_installed(),
        modsync_managed: state.is_modsync_installed() && ms_config.is_some(),
        enforced,
        silent,
        restart_required,
        extra_sync_paths,
        exclusions,
        has_client_mods: mods.iter().any(|m| m.has_client_files),
        mods,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct ModSyncSettingsForm {
    csrf_token: String,
    enforced: Option<String>,
    silent: Option<String>,
    restart_required: Option<String>,
    extra_sync_paths: String,
    exclusions: String,
}

pub async fn save_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ModSyncSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let extra_paths: Vec<String> = form
        .extra_sync_paths
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let exclusion_list: Vec<String> = form
        .exclusions
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let _guard = state.config_lock.lock();
    let mut new_config = Config::load(&state.config_path).map_err(WebError::from)?;
    // Preserve existing overrides
    let existing_overrides = new_config
        .modsync
        .as_ref()
        .map(|ms| ms.overrides.clone())
        .unwrap_or_default();

    let ms_config = crate::config::ModSyncConfig {
        enforced: form.enforced.is_some(),
        silent: form.silent.is_some(),
        restart_required: form.restart_required.is_some(),
        extra_sync_paths: extra_paths,
        exclusions: exclusion_list,
        overrides: existing_overrides,
    };

    // Update config and save
    new_config.modsync = Some(ms_config);
    new_config
        .save(&state.config_path)
        .map_err(WebError::from)?;
    drop(_guard);

    // Regenerate NarcoNet config.yaml
    if state.is_modsync_installed() {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let config = new_config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db)
        })
        .await;
    }

    set_flash(&session, "NarcoNet settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/modsync"))
        .finish())
}
