use actix_session::Session;
use actix_web::web::{self, Data, Html};
use actix_web::HttpRequest;
use askama::Template;

use crate::health::{self, IntegrityHealth, ModsHealth, ServerHealth};
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "status.html")]
struct StatusPageTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "partials/status_server.html")]
struct StatusServerTemplate {
    report: ServerHealth,
}

#[derive(Template)]
#[template(path = "partials/status_mods.html")]
struct StatusModsTemplate {
    report: ModsHealth,
}

#[derive(Template)]
#[template(path = "partials/status_integrity.html")]
struct StatusIntegrityTemplate {
    report: IntegrityHealth,
}

pub async fn status_page(req: HttpRequest, session: Session) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let tmpl = StatusPageTemplate {
        user,
        flash,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn server_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port).map_err(WebError::from)?;
    let address = spt_client.base_url().to_string();
    let report = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;
    let tmpl = StatusServerTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn mods_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let db = state.db.clone();
    let installed_mods = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let loaded_mods = if let Ok(spt_client) = crate::spt::server::SptClient::new(&host, port) {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let report = health::check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &state.forge,
        &state.spt_info.spt_version,
    )
    .await;
    let tmpl = StatusModsTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn integrity_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let db = state.db.clone();
    let tracked_files = web::block(move || {
        let db = db.lock();
        db.get_all_tracked_files()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let report = web::block(move || health::check_integrity_from(&tracked_files, &spt_dir))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    let tmpl = StatusIntegrityTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
