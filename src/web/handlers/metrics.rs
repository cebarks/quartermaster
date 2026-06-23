use actix_session::Session;
use actix_web::web::{Data, Html};
use actix_web::HttpRequest;
use askama::Template;

use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(Template)]
#[template(path = "metrics.html")]
struct MetricsPageTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
}

pub async fn metrics_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerMetrics)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let tmpl = MetricsPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "partials/proxy_metrics.html")]
struct ProxyMetricsTemplate {
    proxy: crate::web::proxy_metrics::MetricsSnapshot,
}

pub async fn proxy_metrics_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerMetrics)?;
    let template = ProxyMetricsTemplate {
        proxy: state.proxy_metrics.snapshot(),
    };
    Ok(Html::new(template.render().map_err(WebError::from)?))
}
