use actix_session::Session;
use actix_web::web::{self, Data, Html};
use actix_web::HttpRequest;
use askama::Template;

use crate::container::ContainerStats;
use crate::health::{self, IntegrityHealth, ModsHealth, ServerHealth};
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
#[template(path = "status.html")]
struct StatusPageTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    transitioning: bool,
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

#[derive(Template)]
#[template(path = "partials/proxy_metrics.html")]
struct ProxyMetricsTemplate {
    proxy: crate::web::proxy_metrics::MetricsSnapshot,
}

#[derive(Template)]
#[template(path = "partials/container_stats.html")]
struct ContainerStatsTemplate {
    available: bool,
    cpu_percent: f64,
    mem_usage: u64,
    mem_limit: u64,
    mem_percent: f64,
    net_rx: u64,
    net_tx: u64,
    disk_read: u64,
    disk_write: u64,
}

impl ContainerStatsTemplate {
    fn unavailable() -> Self {
        Self {
            available: false,
            cpu_percent: 0.0,
            mem_usage: 0,
            mem_limit: 0,
            mem_percent: 0.0,
            net_rx: 0,
            net_tx: 0,
            disk_read: 0,
            disk_write: 0,
        }
    }

    fn from_stats(stats: ContainerStats) -> Self {
        Self {
            available: true,
            cpu_percent: stats.cpu_percent,
            mem_usage: stats.mem_usage,
            mem_limit: stats.mem_limit,
            mem_percent: stats.mem_percent,
            net_rx: stats.net_rx,
            net_tx: stats.net_tx,
            disk_read: stats.disk_read,
            disk_write: stats.disk_write,
        }
    }
}

pub async fn status_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let transitioning = state.get_server_transition().is_some();

    let tmpl = StatusPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        transitioning,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn server_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port).map_err(WebError::from)?;
    let address = spt_client.base_url().to_string();

    let mut report = health::check_server(&spt_client, &state.spt_info.spt_version, &address).await;

    let (started_at, transition) = fetch_server_context(&state).await;
    report.started_at = started_at;
    report.transition = transition;

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

pub async fn proxy_metrics_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let template = ProxyMetricsTemplate {
        proxy: state.proxy_metrics.snapshot(),
    };
    Ok(Html::new(template.render().map_err(WebError::from)?))
}

pub async fn container_stats_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    require_auth(&req)?;

    let tmpl = match (
        state.config.server_container.as_deref(),
        state.container_mgr.as_ref(),
    ) {
        (Some(container), Some(mgr)) => match mgr.stats(container).await {
            Ok(stats) => ContainerStatsTemplate::from_stats(stats),
            Err(e) => {
                tracing::trace!(error = %e, "container stats unavailable");
                ContainerStatsTemplate::unavailable()
            }
        },
        _ => ContainerStatsTemplate::unavailable(),
    };

    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub(crate) async fn fetch_server_context(state: &AppState) -> (Option<String>, Option<String>) {
    let started_at = if let (Some(container), Some(mgr)) = (
        state.config.server_container.as_deref(),
        state.container_mgr.as_ref(),
    ) {
        mgr.container_started_at(container).await.ok().flatten()
    } else {
        None
    };
    let transition = state.get_server_transition();
    (started_at, transition)
}
