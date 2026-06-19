use actix_session::Session;
use actix_web::web::{self, Data, Html};
use actix_web::HttpRequest;
use askama::Template;

use crate::cli::common::find_unmanaged_mod_dirs;
use crate::db::mods::InstalledMod;
use crate::health;
use crate::server_detect::resolve_server_addr;
use crate::spt::server::SptClient;
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    user: SessionUser,
    mods: Vec<InstalledMod>,
    pending_count: usize,
    unmanaged_dirs: Vec<(String, usize)>,
    spt_version: String,
    tarkov_version: String,
    flash: Option<FlashMessage>,
    csrf_token: String,
}

pub async fn dashboard(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();

    let (mods, pending_count, unmanaged_dirs) = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let pending = db.list_pending_ops()?;
        let (dirs, _total) = find_unmanaged_mod_dirs(&spt_dir, &db)?;
        let dirs_vec: Vec<(String, usize)> = dirs.into_iter().collect();
        Ok::<_, anyhow::Error>((mods, pending.len(), dirs_vec))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = DashboardTemplate {
        user,
        mods,
        pending_count,
        unmanaged_dirs,
        spt_version: state.spt_info.spt_version.clone(),
        tarkov_version: state.spt_info.tarkov_version.clone(),
        flash,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "partials/dashboard_server_status.html")]
struct DashboardServerStatusTemplate {
    reachable: bool,
    latency_ms: Option<u64>,
}

pub async fn server_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let (host, port) = resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = SptClient::new(&host, port).map_err(WebError::from)?;
    let address = spt_client.base_url().to_string();

    let server = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;

    let tmpl = DashboardServerStatusTemplate {
        reachable: server.reachable,
        latency_ms: server.latency_ms,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
