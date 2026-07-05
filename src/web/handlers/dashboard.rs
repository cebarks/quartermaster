use actix_session::Session;
use actix_web::web::{self, Data, Html};
use actix_web::HttpRequest;
use askama::Template;

use crate::health::{self, ModsHealth, ServerHealth};
use crate::server_detect::resolve_server_addr;
use crate::spt::server::SptClient;
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    user: SessionUser,
    spt_version: String,
    tarkov_version: String,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    modsync_managed: bool,
}

pub async fn dashboard(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let nav = NavContext::from_state(&state);
    let modsync_managed = nav.modsync_installed && nav.modsync_enabled;
    let tmpl = DashboardTemplate {
        user,
        spt_version: state.spt_info.spt_version.clone(),
        tarkov_version: state.spt_info.tarkov_version.clone(),
        flash,
        csrf_token,
        nav,
        modsync_managed,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

// -- Partials --

#[derive(Template)]
#[template(path = "partials/dashboard_server.html")]
struct DashboardServerTemplate {
    report: ServerHealth,
    user: SessionUser,
    csrf_token: String,
}

pub async fn server_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let (host, port) = resolve_server_addr(&state.config(), &state.spt_dir);
    let spt_client = SptClient::new(&host, port).map_err(WebError::from)?;
    let address = spt_client.base_url().to_string();

    let mut report = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;

    let (started_at, transition) = fetch_server_context(&state).await;
    report.started_at = started_at;
    report.transition = transition;

    let tmpl = DashboardServerTemplate {
        report,
        user,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "partials/dashboard_mods.html")]
struct DashboardModsTemplate {
    report: ModsHealth,
    pending_count: usize,
}

pub async fn mods_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let db = state.db.clone();
    let (installed_mods, pending_count, server_mod_ids, spt_names) = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let pending = db.list_pending_ops()?;
        let server_ids = db.mods_with_server_files()?;
        let names = health::resolve_spt_names(&db, &server_ids);
        Ok::<_, anyhow::Error>((mods, pending.len(), server_ids, names))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (host, port) = resolve_server_addr(&state.config(), &state.spt_dir);
    let loaded_mods = if let Ok(spt_client) = SptClient::new(&host, port) {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let report = health::check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &state.forge,
        &state.spt_info.spt_version,
        &server_mod_ids,
        &spt_names,
    )
    .await;
    let tmpl = DashboardModsTemplate {
        report,
        pending_count,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "partials/dashboard_players.html")]
struct DashboardPlayersTemplate {
    players: Vec<crate::fika::client::FikaPlayerPresence>,
    available: bool,
}

pub async fn players_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;

    let (players, available) = match state.fika_client.as_ref() {
        Some(client) => match client.presence().await {
            Ok(p) => (p, true),
            Err(_) => (vec![], false),
        },
        None => (vec![], false),
    };

    let tmpl = DashboardPlayersTemplate { players, available };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub(crate) async fn fetch_server_context(state: &AppState) -> (Option<String>, Option<String>) {
    let container_name = state.config().server_container.clone();
    let started_at = if let (Some(container), Some(mgr)) =
        (container_name.as_deref(), state.container_mgr.as_ref())
    {
        mgr.container_started_at(container).await.ok().flatten()
    } else {
        None
    };
    let transition = state.get_server_transition();
    (started_at, transition)
}
