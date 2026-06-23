use actix_session::Session;
use actix_web::web::{self, Data, Form, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::config::Config;
use crate::db::mods::InstalledMod;
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

#[derive(serde::Deserialize)]
pub struct ModsyncQuery {
    #[serde(default = "default_tab")]
    tab: String,
}

fn default_tab() -> String {
    "settings".to_string()
}

#[derive(Template)]
#[template(path = "modsync.html")]
struct ModSyncTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    modsync_managed: bool,
    active_tab: String,
    tab_content: String,
}

#[derive(Template)]
#[template(path = "modsync/partials/settings.html")]
struct SettingsPartialTemplate {
    csrf_token: String,
    enforced: bool,
    silent: bool,
    restart_required: bool,
    extra_sync_paths: String,
    exclusions: String,
}

pub async fn modsync_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<ModsyncQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    // Re-read config from disk to pick up changes saved by save_settings
    // (state.config is immutable behind Arc, so it goes stale after saves)
    let live_config = crate::config::Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config.or_else(|| state.config.modsync.clone());

    let nav = NavContext::from_state(&state);
    let modsync_managed = nav.modsync_installed && ms_config.is_some();

    // Determine active tab (validate and constrain based on state)
    let mut active_tab = query.tab.as_str();
    let valid_tabs = ["settings", "groups", "mods", "preview"];
    if !valid_tabs.contains(&active_tab) {
        active_tab = "settings";
    }
    // If not managed, force to settings
    if !modsync_managed && active_tab != "settings" {
        active_tab = "settings";
    }

    // Render the appropriate tab partial
    let tab_content = match active_tab {
        "settings" => {
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
            let partial = SettingsPartialTemplate {
                csrf_token: csrf_token.clone(),
                enforced,
                silent,
                restart_required,
                extra_sync_paths,
                exclusions,
            };
            partial.render().map_err(WebError::from)?
        }
        "groups" => "<p>Groups tab coming soon</p>".to_string(),
        "mods" => "<p>Mods tab coming soon</p>".to_string(),
        "preview" => "<p>Preview tab coming soon</p>".to_string(),
        _ => "<p>Unknown tab</p>".to_string(),
    };

    let tmpl = ModSyncTemplate {
        user,
        flash,
        csrf_token,
        nav,
        modsync_managed,
        active_tab: active_tab.to_string(),
        tab_content,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn settings_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let live_config = crate::config::Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config.or_else(|| state.config.modsync.clone());

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

    let tmpl = SettingsPartialTemplate {
        csrf_token,
        enforced,
        silent,
        restart_required,
        extra_sync_paths,
        exclusions,
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
    require_permission(&user, Permission::ModsyncManage)?;
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
    let existing_groups = new_config
        .modsync
        .as_ref()
        .map(|ms| ms.groups.clone())
        .unwrap_or_default();

    let ms_config = crate::config::ModSyncConfig {
        enforced: form.enforced.is_some(),
        silent: form.silent.is_some(),
        restart_required: form.restart_required.is_some(),
        extra_sync_paths: extra_paths,
        exclusions: exclusion_list,
        overrides: existing_overrides,
        groups: existing_groups,
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
        .insert_header(("Location", "/quma/modsync?tab=settings"))
        .finish())
}
