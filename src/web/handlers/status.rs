use actix_session::Session;
use actix_web::web::{self, Data, Html};
use askama::Template;

use crate::health::{self, HealthReport};
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "status.html")]
struct StatusPageTemplate {
    user: SessionUser,
}

#[derive(Template)]
#[template(path = "partials/status_detail.html")]
struct StatusDetailTemplate {
    report: HealthReport,
}

pub async fn status_page(session: Session) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let tmpl = StatusPageTemplate { user };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn status_partial(state: Data<AppState>) -> actix_web::Result<Html> {
    let report = build_health_report(&state).await.map_err(WebError::from)?;
    let tmpl = StatusDetailTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

async fn build_health_report(state: &AppState) -> anyhow::Result<HealthReport> {
    use crate::server_detect::resolve_server_addr;
    use crate::spt::server::SptClient;

    let (host, port) = resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    let server = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;

    let loaded_mods = if server.reachable {
        spt_client.loaded_server_mods().await.ok()
    } else {
        None
    };

    let db = state.db.clone();
    let (installed_mods, tracked_files) = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let files = db.get_all_tracked_files()?;
        Ok::<_, anyhow::Error>((mods, files))
    })
    .await??;

    let mods = health::check_mods_health(
        &installed_mods,
        loaded_mods.as_ref(),
        &state.forge,
        &state.spt_info.spt_version,
    )
    .await;

    let spt_dir = state.spt_dir.clone();
    let integrity =
        web::block(move || health::check_integrity_from(&tracked_files, &spt_dir)).await??;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}
