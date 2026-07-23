use std::collections::HashMap;
use std::sync::Arc;

use actix_session::Session;

use actix_web::web::{self, Data, Form, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::client::{ClientHealth, ClientState};
use crate::config::{Config, RestartPolicy};
// ponytail: removed ContainerManager import - HeadlessService handles container management
use crate::db::rbac::Permission;
use crate::dirs::QumaDirs;
use crate::headless::service::LifecycleAction;
use crate::numa::NumaTopology;
// ponytail: removed EHeadlessStatus import - now handled in service layer
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(Clone)]
struct PlayerInfo {
    id: String,
    name: String,
}

struct ClientView {
    state: ClientState,
    players: Vec<PlayerInfo>,
    alias: Option<String>,
}

fn resolve_clients(
    clients: Vec<ClientState>,
    names: &HashMap<String, String>,
    aliases: &HashMap<String, String>,
) -> Vec<ClientView> {
    clients
        .into_iter()
        .map(|state| {
            let players = state
                .players
                .iter()
                .map(|id| PlayerInfo {
                    name: names.get(id).cloned().unwrap_or_else(|| id.clone()),
                    id: id.clone(),
                })
                .collect();
            let alias = state
                .profile_id
                .as_ref()
                .and_then(|pid| aliases.get(pid).cloned());
            ClientView {
                state,
                players,
                alias,
            }
        })
        .collect()
}

#[derive(Template)]
#[template(path = "headless.html")]
struct HeadlessPageTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    config: Config,
    restart_policy: String,
    headless_clients: Vec<ClientView>,
    headless_converging: bool,
    headless_target_count: u32,
    numa_nodes: Vec<(u32, String)>,
    numa_policy: String,
    numa_node: Option<u32>,
}

#[derive(Template)]
#[template(path = "clients/detail.html")]
struct ClientDetailTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    client: ClientView,
    session_stats: Vec<crate::db::headless_stats::HeadlessSessionRow>,
    headless_image: String,
}

#[derive(Template)]
#[template(path = "clients/partials/status.html")]
struct ClientsStatusPartialTemplate {
    clients: Vec<ClientView>,
    user: SessionUser,
    csrf_token: String,
    headless_converging: bool,
}

#[derive(Template)]
#[template(path = "partials/dashboard_clients_status.html")]
struct DashboardClientsStatusTemplate {
    healthy_count: usize,
    degraded_count: usize,
    down_count: usize,
    total_count: usize,
    clients: Vec<ClientView>,
}

#[derive(Template)]
#[template(path = "headless/operation_polling.html")]
struct OperationPollingTemplate {
    operation_id: u64,
    message: String,
}

#[derive(Template)]
#[template(path = "headless/operation_result.html")]
struct OperationResultTemplate {
    success: bool,
    message: String,
}

fn build_profile_names(dirs: &QumaDirs) -> HashMap<String, String> {
    crate::spt::profiles::list_profiles(dirs)
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.aid, p.username))
        .collect()
}

fn build_client_aliases(spt_dir: &std::path::Path) -> HashMap<String, String> {
    let path = crate::fika::config::fika_config_path(spt_dir);
    crate::fika::config::read_fika_config(&path)
        .map(|c| c.headless.profiles.aliases)
        .unwrap_or_default()
}

// ponytail: removed require_container_mgr - no longer needed, HeadlessService handles dependencies
// ponytail: removed create_spt_client - HeadlessService constructs SptClient on-demand
// ponytail: removed resolve_client_container - replaced by HeadlessService methods
// ponytail: removed client_lifecycle helper - logic moved to HeadlessService

pub async fn operation_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<u64>,
) -> actix_web::Result<web::Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    let operation_id = path.into_inner();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(_) => {
            let tmpl = OperationResultTemplate {
                success: false,
                message: "Operation tracker unavailable".to_string(),
            };
            return Ok(web::Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    match service.operations().poll(operation_id) {
        Some(crate::headless::operations::OperationStatus::Running) => {
            let tmpl = OperationPollingTemplate {
                operation_id,
                message: "Operation in progress...".to_string(),
            };
            Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
        }
        Some(crate::headless::operations::OperationStatus::Completed) => {
            let tmpl = OperationResultTemplate {
                success: true,
                message: "Operation completed successfully".to_string(),
            };
            Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
        }
        Some(crate::headless::operations::OperationStatus::Failed { error }) => {
            let tmpl = OperationResultTemplate {
                success: false,
                message: error,
            };
            Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
        }
        None => {
            let tmpl = OperationResultTemplate {
                success: true,
                message: "Operation completed".to_string(),
            };
            Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn headless_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let config = Config::load(&state.config_path).map_err(WebError::from)?;

    let restart_policy = config
        .headless
        .as_ref()
        .map(|c| c.restart_policy.to_string())
        .unwrap_or_else(|| RestartPolicy::Auto.to_string());

    let headless_clients = match state.client_states() {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };
    let headless_converging = state.converging.load(std::sync::atomic::Ordering::Relaxed);
    let headless_target_count = config
        .headless
        .as_ref()
        .map(|h| h.client_count())
        .unwrap_or(0);

    let numa_nodes: Vec<(u32, String)> = NumaTopology::detect()
        .map(|t| {
            t.nodes()
                .iter()
                .map(|n| (n.id, n.cpulist.clone()))
                .collect()
        })
        .unwrap_or_default();

    let (numa_policy, numa_node) = config
        .headless
        .as_ref()
        .map(|h| {
            if h.numa_auto {
                ("auto".to_string(), None)
            } else if h.numa_node.is_some() {
                ("node".to_string(), h.numa_node)
            } else {
                ("none".to_string(), None)
            }
        })
        .unwrap_or_else(|| ("none".to_string(), None));

    let dirs = Arc::clone(&state.dirs);
    let profile_names = build_profile_names(&dirs);
    let aliases = build_client_aliases(&state.dirs.spt_server);
    let headless_clients = resolve_clients(headless_clients, &profile_names, &aliases);
    let tmpl = HeadlessPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        config,
        restart_policy,
        headless_clients,
        headless_converging,
        headless_target_count,
        numa_nodes,
        numa_policy,
        numa_node,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn client_detail(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
) -> actix_web::Result<web::Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let index = path.into_inner();

    let clients = match state.client_states() {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let client = clients
        .into_iter()
        .find(|c| c.index == index)
        .ok_or(WebError::NotFound)?;

    let dirs = Arc::clone(&state.dirs);
    let profile_names = build_profile_names(&dirs);
    let aliases = build_client_aliases(&state.dirs.spt_server);
    let client = resolve_clients(vec![client], &profile_names, &aliases)
        .into_iter()
        .next()
        .ok_or(WebError::NotFound)?;

    let db = state.db.clone();
    let client_index = index;
    let session_stats = web::block(move || {
        let db = db.lock();
        db.get_recent_session_stats(client_index, 20)
    })
    .await
    .map_err(WebError::from)?
    .unwrap_or_default();

    let headless_image = state
        .config()
        .headless
        .as_ref()
        .map(|h| h.resolve_image((index - 1) as usize).to_string())
        .unwrap_or_default();

    let tmpl = ClientDetailTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        client,
        session_stats,
        headless_image,
    };
    Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn client_restart(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let is_htmx = req.headers().get("HX-Request").is_some();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            if is_htmx {
                return Ok(HttpResponse::NoContent().finish());
            }
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service
        .client_lifecycle(index, LifecycleAction::Restart)
        .await
    {
        Ok(()) => set_flash(
            &session,
            &format!("Client {index} restarting"),
            FlashType::Success,
        ),
        Err(e) => set_flash(&session, &e.to_string(), FlashType::Error),
    }

    if is_htmx {
        return Ok(HttpResponse::NoContent().finish());
    }
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

pub async fn client_graceful_restart(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let is_htmx = req.headers().get("HX-Request").is_some();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            if is_htmx {
                return Ok(HttpResponse::NoContent().finish());
            }
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service.graceful_restart(index).await {
        Ok(result) => {
            use crate::headless::service::GracefulResult;
            match result {
                GracefulResult::Exited => {
                    set_flash(
                        &session,
                        &format!("Client {index} shut down gracefully"),
                        FlashType::Success,
                    );
                }
                GracefulResult::Timeout => {
                    set_flash(
                        &session,
                        &format!("Client {index} did not shut down within 30s — use force restart"),
                        FlashType::Warning,
                    );
                }
            }
        }
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
        }
    }

    if is_htmx {
        return Ok(HttpResponse::NoContent().finish());
    }
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/headless"))
        .finish())
}

pub async fn client_stop(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let is_htmx = req.headers().get("HX-Request").is_some();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            if is_htmx {
                return Ok(HttpResponse::NoContent().finish());
            }
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service.client_lifecycle(index, LifecycleAction::Stop).await {
        Ok(()) => set_flash(
            &session,
            &format!("Client {index} stopped"),
            FlashType::Success,
        ),
        Err(e) => set_flash(&session, &e.to_string(), FlashType::Error),
    }

    if is_htmx {
        return Ok(HttpResponse::NoContent().finish());
    }
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

pub async fn client_start(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let is_htmx = req.headers().get("HX-Request").is_some();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            if is_htmx {
                return Ok(HttpResponse::NoContent().finish());
            }
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service
        .client_lifecycle(index, LifecycleAction::Start)
        .await
    {
        Ok(()) => set_flash(
            &session,
            &format!("Client {index} started"),
            FlashType::Success,
        ),
        Err(e) => set_flash(&session, &e.to_string(), FlashType::Error),
    }

    if is_htmx {
        return Ok(HttpResponse::NoContent().finish());
    }
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

#[derive(serde::Deserialize)]
pub struct ScaleForm {
    csrf_token: String,
    count: u32,
    #[serde(default)]
    force: bool,
}

pub async fn client_scale(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ScaleForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target = form.count;
    let force = form.force;
    let is_htmx = req.headers().get("HX-Request").is_some();

    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?));
            }
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service.scale(target, force).await {
        Ok(op_id) => {
            if is_htmx {
                let tmpl = OperationPollingTemplate {
                    operation_id: op_id.0,
                    message: format!("Scaling to {target} client(s)..."),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(
                    &session,
                    &format!("Scaling to {target} client(s)..."),
                    FlashType::Success,
                );
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(crate::headless::HeadlessError::ClientInRaid { clients }) => {
            let client_list = clients
                .iter()
                .map(|i| format!("Client {i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let msg = format!(
                "Cannot scale down: {} in raid. Use force to override.",
                client_list
            );
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: msg,
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &msg, FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &e.to_string(), FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
    }
}

pub async fn client_converge(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let is_htmx = req.headers().get("HX-Request").is_some();

    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?));
            }
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service.converge().await {
        Ok(op_id) => {
            if is_htmx {
                let tmpl = OperationPollingTemplate {
                    operation_id: op_id.0,
                    message: "Convergence started...".to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, "Convergence started...", FlashType::Success);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &e.to_string(), FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
    }
}

pub async fn client_rebuild(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let is_htmx = req.headers().get("HX-Request").is_some();

    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?));
            }
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    // ponytail: force=false for now, add force param to form in later tasks if needed
    match service.rebuild(false).await {
        Ok(op_id) => {
            if is_htmx {
                let tmpl = OperationPollingTemplate {
                    operation_id: op_id.0,
                    message: "Rebuilding all headless clients...".to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(
                    &session,
                    "Rebuilding all headless clients...",
                    FlashType::Success,
                );
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(crate::headless::HeadlessError::ClientInRaid { clients }) => {
            let client_list = clients
                .iter()
                .map(|i| format!("Client {i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let msg = format!("Cannot rebuild: {} in raid.", client_list);
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: msg,
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &msg, FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &e.to_string(), FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
    }
}

pub async fn client_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<web::Html> {
    let user = require_auth(&req)?;

    let clients = match state.client_states() {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let headless_converging = state.converging.load(std::sync::atomic::Ordering::Relaxed);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let dirs = Arc::clone(&state.dirs);
    let profile_names = build_profile_names(&dirs);
    let aliases = build_client_aliases(&state.dirs.spt_server);
    let clients = resolve_clients(clients, &profile_names, &aliases);
    let tmpl = ClientsStatusPartialTemplate {
        clients,
        user,
        csrf_token,
        headless_converging,
    };
    Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn client_create(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let is_htmx = req.headers().get("HX-Request").is_some();

    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?));
            }
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    let new_count = state
        .config()
        .headless
        .as_ref()
        .map(|h| h.client_count() + 1)
        .unwrap_or(1);

    match service.create().await {
        Ok(op_id) => {
            if is_htmx {
                let tmpl = OperationPollingTemplate {
                    operation_id: op_id.0,
                    message: format!("Creating client {new_count}..."),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(
                    &session,
                    &format!("Creating client {new_count}..."),
                    FlashType::Success,
                );
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &e.to_string(), FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
    }
}

pub async fn client_delete(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let is_htmx = req.headers().get("HX-Request").is_some();

    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?));
            }
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    // ponytail: force=false for now, add force param to form in later tasks if needed
    match service.delete(index, false).await {
        Ok(op_id) => {
            if is_htmx {
                let tmpl = OperationPollingTemplate {
                    operation_id: op_id.0,
                    message: format!("Deleting client {index}..."),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(
                    &session,
                    &format!("Deleting client {index}..."),
                    FlashType::Success,
                );
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(crate::headless::HeadlessError::ClientInRaid { .. }) => {
            let msg = format!("Client {index} is in raid. Wait for it to finish or stop it first.");
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: msg,
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &msg, FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(crate::headless::HeadlessError::ClientNotFound(_)) => {
            let msg = format!("Client {index} does not exist");
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: msg,
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &msg, FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
        Err(e) => {
            if is_htmx {
                let tmpl = OperationResultTemplate {
                    success: false,
                    message: e.to_string(),
                };
                Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(tmpl.render().map_err(WebError::from)?))
            } else {
                set_flash(&session, &e.to_string(), FlashType::Error);
                Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish())
            }
        }
    }
}

#[derive(serde::Deserialize)]
pub struct StartRaidForm {
    pub csrf_token: String,
    pub location_id: String,
    pub time: i32,
    pub use_event: Option<String>,
}

pub async fn client_start_raid(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<StartRaidForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service
        .start_raid(
            index,
            &form.location_id,
            form.time,
            form.use_event.is_some(),
        )
        .await
    {
        Ok(()) => set_flash(
            &session,
            &format!("Raid started on client {index}"),
            FlashType::Success,
        ),
        Err(e) => set_flash(&session, &e.to_string(), FlashType::Error),
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

#[derive(serde::Deserialize)]
pub struct RenameForm {
    csrf_token: String,
    name: String,
}

pub async fn client_rename(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<RenameForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    match service.rename(index, &form.name).await {
        Ok(()) => set_flash(
            &session,
            "Client renamed. Restart the SPT server for the in-game name to take effect.",
            FlashType::Success,
        ),
        Err(e) => set_flash(&session, &e.to_string(), FlashType::Error),
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

#[derive(serde::Deserialize)]
pub struct SetImageForm {
    csrf_token: String,
    image: String,
}

pub async fn client_set_image(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<SetImageForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::HeadlessManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();
    let service = match state.headless_service() {
        Ok(s) => s,
        Err(e) => {
            set_flash(&session, &e.to_string(), FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };
    let new_image = form.into_inner().image.trim().to_string();
    let image_opt = if new_image.is_empty() {
        None
    } else {
        Some(new_image)
    };

    match service.set_image(index, image_opt).await {
        Ok(()) => set_flash(
            &session,
            "Client image updated. Re-converge to apply.",
            FlashType::Success,
        ),
        Err(e) => set_flash(&session, &e.to_string(), FlashType::Error),
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

pub async fn dashboard_clients_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<web::Html> {
    require_auth(&req)?;

    let raw_clients = match state.client_states() {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let dirs = Arc::clone(&state.dirs);
    let names = build_profile_names(&dirs);
    let aliases = build_client_aliases(&state.dirs.spt_server);
    let clients = resolve_clients(raw_clients.clone(), &names, &aliases);

    let healthy_count = raw_clients
        .iter()
        .filter(|c| c.health == ClientHealth::Healthy)
        .count();
    let degraded_count = raw_clients
        .iter()
        .filter(|c| c.health == ClientHealth::Degraded)
        .count();
    let down_count = raw_clients
        .iter()
        .filter(|c| matches!(c.health, ClientHealth::Down | ClientHealth::GivenUp))
        .count();
    let total_count = raw_clients.len();

    let tmpl = DashboardClientsStatusTemplate {
        healthy_count,
        degraded_count,
        down_count,
        total_count,
        clients,
    };
    Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
}
