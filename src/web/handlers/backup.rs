use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::backups::BackupRecord;
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission};
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(Template)]
#[template(path = "mods/partials/backups.html")]
struct BackupsPartialTemplate {
    backups: Vec<BackupRecord>,
    mod_db_id: i64,
    csrf_token: String,
    can_update: bool,
}

pub async fn mod_backups_partial(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let mod_db_id = path.into_inner();
    let csrf_token = csrf::get_or_create_token(&session);
    let db = state.db.clone();

    let (installed, backups) = web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod(mod_db_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
        let backups = db.list_backups_for_mod(installed.forge_mod_id)?;
        Ok::<_, anyhow::Error>((installed, backups))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = BackupsPartialTemplate {
        backups,
        mod_db_id: installed.id,
        csrf_token,
        can_update: user.can("mods.update"),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn create_mod_backup(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsUpdate)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let mod_db_id = path.into_inner();
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let config = state.config_cloned();

    web::block(move || {
        let db = db.lock();
        crate::backup::backup_mod(&db, &spt_dir, &config, mod_db_id, "manual")
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    set_flash(&session, "Backup created", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
        .finish())
}

pub async fn restore_backup(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsUpdate)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let backup_db_id = path.into_inner();

    // Fetch backup type first so we can enforce permission before restoring
    let db = state.db.clone();
    let backup_type = web::block(move || {
        let db = db.lock();
        let backup = db
            .get_backup(backup_db_id)?
            .ok_or_else(|| anyhow::anyhow!("backup not found"))?;
        Ok::<_, anyhow::Error>(backup.backup_type.clone())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // Full restores overwrite config and profiles — require SettingsManage
    if backup_type == "full" {
        require_permission(&user, Permission::SettingsManage)?;
    }

    // Check server status
    let config = state.config_cloned();
    let running = crate::server_detect::is_server_running(
        &config,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);
    if running {
        set_flash(
            &session,
            "Stop the server before restoring a backup",
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    let is_full = backup_type == "full";
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();

    web::block(move || {
        let db = db.lock();
        match backup_type.as_str() {
            "mod" => crate::backup::restore_mod_backup(&db, &spt_dir, &config, backup_db_id)?,
            "full" => crate::backup::restore_full_backup(&db, &spt_dir, &config, backup_db_id)?,
            _ => anyhow::bail!("unknown backup type: {backup_type}"),
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let msg = if is_full {
        "Full backup restored — restart the web server to reload config"
    } else {
        "Backup restored"
    };
    set_flash(&session, msg, FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
        .finish())
}

// --- Admin backup management ---

#[derive(Template)]
#[template(path = "admin_backups.html")]
struct AdminBackupsTemplate {
    user: crate::web::auth::SessionUser,
    nav: NavContext,
    backups: Vec<BackupRecord>,
    total_size: i64,
    flash: Option<crate::web::flash::FlashMessage>,
    csrf_token: String,
}

pub async fn admin_backups_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let db = state.db.clone();

    let backups = web::block(move || {
        let db = db.lock();
        db.list_all_backups()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let total_size: i64 = backups.iter().filter_map(|b| b.backup_size).sum();

    let tmpl = AdminBackupsTemplate {
        user,
        nav: NavContext::from_state(&state),
        backups,
        total_size,
        flash,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn create_full_backup(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let config = state.config_cloned();

    web::block(move || {
        let db = db.lock();
        crate::backup::backup_full(&db, &spt_dir, &config)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    set_flash(&session, "Full backup created", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/admin/backups"))
        .finish())
}
