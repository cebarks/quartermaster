use std::collections::HashMap;
use std::sync::Arc;

use actix_session::Session;

use actix_web::web::{self, Data, Form, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;
use jsonc_parser::cst::CstInputValue;

use crate::client::{ClientHealth, ClientState};
use crate::config::{Config, RestartPolicy};
use crate::container::ContainerManager;
use crate::db::rbac::Permission;
use crate::dirs::QumaDirs;
use crate::numa::NumaTopology;
use crate::spt::headless::EHeadlessStatus;
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

fn require_container_mgr<'a>(
    state: &'a AppState,
    session: &Session,
) -> Result<&'a Arc<ContainerManager>, HttpResponse> {
    state.container_mgr.as_ref().ok_or_else(|| {
        set_flash(
            session,
            "Podman socket not available. Ensure podman.socket is enabled.",
            FlashType::Error,
        );
        HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish()
    })
}

async fn resolve_client_container(
    state: &AppState,
    session: &Session,
    index: u32,
) -> Result<String, HttpResponse> {
    let container_name = match &state.client_states {
        Some(states) => {
            let clients = states.read().await;
            clients
                .iter()
                .find(|c| c.index == index)
                .map(|c| c.container_name.clone())
        }
        None => None,
    };

    container_name.ok_or_else(|| {
        set_flash(
            session,
            &format!("Client {index} not found"),
            FlashType::Error,
        );
        HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish()
    })
}

async fn client_lifecycle(
    state: &Data<AppState>,
    req: &HttpRequest,
    session: &Session,
    csrf_token: &str,
    index: u32,
    action: &str,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(req)?;
    require_permission(&user, Permission::HeadlessManage)?;

    if !crate::web::csrf::validate_token(session, csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let mgr = match require_container_mgr(state, session) {
        Ok(m) => m,
        Err(resp) => return Ok(resp),
    };

    let container_name = match resolve_client_container(state, session, index).await {
        Ok(n) => n,
        Err(resp) => return Ok(resp),
    };

    let result = match action {
        "start" => mgr.start(&container_name).await,
        "stop" => mgr.stop(&container_name).await,
        "restart" => mgr.restart(&container_name).await,
        _ => unreachable!(),
    };

    let verb_past = match action {
        "start" => "starting",
        "stop" => "stopped",
        "restart" => "restarting",
        _ => unreachable!(),
    };

    if let Err(e) = result {
        tracing::error!(container = %container_name, action, err = %e, "client lifecycle action failed");
        set_flash(
            session,
            &format!("Failed to {action} client {index}: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container = %container_name, action, "client lifecycle action succeeded");
        set_flash(
            session,
            &format!("Client {index} {verb_past}"),
            FlashType::Success,
        );
        // Reset failure state on manual start/restart so GivenUp can recover
        if action == "start" || action == "restart" {
            if let Some(states) = &state.client_states {
                let mut clients = states.write().await;
                if let Some(client) = clients.iter_mut().find(|c| c.index == index) {
                    client.consecutive_failures = 0;
                    client.health = ClientHealth::Degraded;
                    client.manually_stopped = false;
                }
            }
        }
        if action == "stop" {
            if let Some(states) = &state.client_states {
                let mut clients = states.write().await;
                if let Some(client) = clients.iter_mut().find(|c| c.index == index) {
                    client.manually_stopped = true;
                }
            }
        }
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

fn create_spt_client(
    state: &AppState,
    session: &Session,
) -> Result<crate::spt::server::SptClient, HttpResponse> {
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.dirs);
    crate::spt::server::SptClient::new(&host, port).map_err(|e| {
        set_flash(
            session,
            &format!("Failed to create SPT client: {e}"),
            FlashType::Error,
        );
        HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish()
    })
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

    let headless_clients = match &state.client_states {
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

    let dirs = (*state.dirs).clone();
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

    let clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let client = clients
        .into_iter()
        .find(|c| c.index == index)
        .ok_or(WebError::NotFound)?;

    let dirs = (*state.dirs).clone();
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
    client_lifecycle(
        &state,
        &req,
        &session,
        &form.csrf_token,
        path.into_inner(),
        "restart",
    )
    .await
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

    let fika_client = match state.fika_client.as_ref() {
        Some(c) => c.clone(),
        None => {
            set_flash(&session, "Fika integration not available", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    // Read client state
    let (profile_id, fika_status) = {
        let states = match state.client_states.as_ref() {
            Some(s) => s.read().await,
            None => {
                set_flash(
                    &session,
                    "Headless clients not configured",
                    FlashType::Error,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish());
            }
        };
        let client = states.iter().find(|c| c.index == index);
        match client {
            Some(c) => (c.profile_id.clone(), c.fika_status.clone()),
            None => {
                set_flash(
                    &session,
                    &format!("Client {index} not found"),
                    FlashType::Error,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish());
            }
        }
    };

    // Block if IN_RAID
    if fika_status == Some(EHeadlessStatus::InRaid) {
        set_flash(
            &session,
            &format!("Client {index} is in a raid — use force restart or wait for raid to end"),
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish());
    }

    let profile_id = match profile_id {
        Some(pid) if !pid.is_empty() => pid,
        _ => {
            set_flash(
                &session,
                &format!("Client {index} has no profile ID"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    // Send shutdown
    match fika_client.shutdown_headless(&profile_id).await {
        Ok(()) => {
            // Wait for container exit (poll every 2s, 30s timeout)
            let mgr = match require_container_mgr(&state, &session) {
                Ok(m) => m,
                Err(resp) => return Ok(resp),
            };
            let container_name = match resolve_client_container(&state, &session, index).await {
                Ok(n) => n,
                Err(resp) => return Ok(resp),
            };
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            let mut exited = false;
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !mgr.is_running(&container_name).await.unwrap_or(true) {
                    exited = true;
                    break;
                }
            }
            if exited {
                set_flash(
                    &session,
                    &format!("Client {index} shut down gracefully"),
                    FlashType::Success,
                );
            } else {
                set_flash(
                    &session,
                    &format!("Client {index} did not shut down within 30s — use force restart"),
                    FlashType::Warning,
                );
            }
        }
        Err(e) => {
            set_flash(
                &session,
                &format!("Graceful restart failed: {e}"),
                FlashType::Error,
            );
        }
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
    client_lifecycle(
        &state,
        &req,
        &session,
        &form.csrf_token,
        path.into_inner(),
        "stop",
    )
    .await
}

pub async fn client_start(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    client_lifecycle(
        &state,
        &req,
        &session,
        &form.csrf_token,
        path.into_inner(),
        "start",
    )
    .await
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

    if target > crate::config::MAX_HEADLESS_CLIENTS {
        set_flash(
            &session,
            &format!(
                "Maximum {} headless clients allowed",
                crate::config::MAX_HEADLESS_CLIENTS
            ),
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish());
    }

    // Check if we have headless clients configured
    if state.config().headless.is_none() {
        set_flash(
            &session,
            "No headless config in quartermaster.toml",
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish());
    }

    // Check if we're scaling down and need to check in-raid status
    let current_count = match &state.client_states {
        Some(states) => states.read().await.len(),
        None => 0,
    };

    if target < current_count as u32 && !force {
        // Check for in-raid clients
        let in_raid_clients: Vec<u32> = match &state.client_states {
            Some(states) => states
                .read()
                .await
                .iter()
                .filter(|c| matches!(c.fika_status, Some(EHeadlessStatus::InRaid)))
                .filter(|c| c.index >= target)
                .map(|c| c.index)
                .collect(),
            None => vec![],
        };

        if !in_raid_clients.is_empty() {
            let client_list = in_raid_clients
                .iter()
                .map(|i| format!("Client {i}"))
                .collect::<Vec<_>>()
                .join(", ");
            set_flash(
                &session,
                &format!(
                    "Cannot scale down: {} in raid. Use force to override.",
                    client_list
                ),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    }

    // Get required dependencies
    let container_mgr = match require_container_mgr(&state, &session) {
        Ok(m) => m,
        Err(resp) => return Ok(resp),
    };

    let headless_config = match state.config().headless.as_ref() {
        Some(cfg) => cfg.clone(),
        None => {
            set_flash(
                &session,
                "Headless config was removed during operation",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };
    let mut updated_config = headless_config;
    let current = updated_config.client_count();
    if target > current {
        for _ in 0..(target - current) {
            updated_config
                .clients
                .push(crate::config::HeadlessClientDef::default());
        }
    } else if target < current {
        updated_config.clients.truncate(target as usize);
    }

    // Create SPT client
    let spt_client = match create_spt_client(&state, &session) {
        Ok(c) => c,
        Err(resp) => return Ok(resp),
    };

    // Run convergence in a background task
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config_cloned();
    let config_path = state.config_path.clone();
    let config_handle = state.config_handle();
    let dirs_clone = (*state.dirs).clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();
    let db_clone = state.db.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &dirs_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
            &db_clone,
        )
        .await;

        if let Err(e) = result {
            tracing::error!(err = %e, "Client convergence failed during scale operation");
        } else {
            tracing::info!(target_count = target, "Client scaling completed");

            // Persist the updated client count to config file
            {
                let _guard = state_clone.config_lock.lock();
                match crate::config::Config::load_with_env(&config_path) {
                    Ok(mut fresh_config) => {
                        if let Some(ref mut headless) = fresh_config.headless {
                            let current = headless.client_count();
                            if target > current {
                                for _ in 0..(target - current) {
                                    headless
                                        .clients
                                        .push(crate::config::HeadlessClientDef::default());
                                }
                            } else if target < current {
                                headless.clients.truncate(target as usize);
                            }
                        }
                        if let Err(e) = fresh_config.save(&config_path) {
                            tracing::error!(err = %e, "Failed to save updated headless config");
                        } else {
                            *config_handle.write() = fresh_config;
                        }
                    }
                    Err(e) => {
                        tracing::error!(err = %e, "Failed to reload config for persisting headless changes");
                    }
                }
            }
        }
    });

    set_flash(
        &session,
        &format!("Scaling to {target} client(s)..."),
        FlashType::Success,
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/headless"))
        .finish())
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

    let headless_config = match state.config().headless.clone() {
        Some(h) => h,
        None => {
            set_flash(
                &session,
                "No headless config in quartermaster.toml",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    let mgr = match require_container_mgr(&state, &session) {
        Ok(m) => m,
        Err(resp) => return Ok(resp),
    };

    let spt_client = match create_spt_client(&state, &session) {
        Ok(c) => c,
        Err(resp) => return Ok(resp),
    };

    let mgr_clone = mgr.clone();
    let config_clone = state.config_cloned();
    let dirs_clone = (*state.dirs).clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();
    let db_clone = state.db.clone();

    tokio::spawn(async move {
        let result = crate::client::converge::converge(
            &mgr_clone,
            &headless_config,
            &config_clone,
            &dirs_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
            &db_clone,
        )
        .await;

        if let Err(e) = result {
            tracing::error!(err = %e, "Manual convergence failed");
        } else {
            tracing::info!("Manual convergence completed");
        }
    });

    set_flash(&session, "Convergence started...", FlashType::Success);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/headless"))
        .finish())
}

pub async fn client_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<web::Html> {
    let user = require_auth(&req)?;

    let clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let dirs = (*state.dirs).clone();
    let profile_names = build_profile_names(&dirs);
    let aliases = build_client_aliases(&state.dirs.spt_server);
    let clients = resolve_clients(clients, &profile_names, &aliases);
    let tmpl = ClientsStatusPartialTemplate {
        clients,
        user,
        csrf_token,
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

    // Headless must be configured
    let headless_config = match state.config().headless.clone() {
        Some(h) => h,
        None => {
            set_flash(
                &session,
                "No headless config in quartermaster.toml",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    if headless_config.client_count() >= crate::config::MAX_HEADLESS_CLIENTS {
        set_flash(
            &session,
            &format!(
                "Maximum {} headless clients allowed",
                crate::config::MAX_HEADLESS_CLIENTS
            ),
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish());
    }

    let container_mgr = match require_container_mgr(&state, &session) {
        Ok(m) => m,
        Err(resp) => return Ok(resp),
    };

    // Build updated config with one new default client appended
    let mut updated_config = headless_config;
    updated_config
        .clients
        .push(crate::config::HeadlessClientDef::default());
    let new_count = updated_config.client_count();

    // Create SPT client
    let spt_client = match create_spt_client(&state, &session) {
        Ok(c) => c,
        Err(resp) => return Ok(resp),
    };

    // Persist to config and spawn convergence
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config_cloned();
    let config_path = state.config_path.clone();
    let config_handle = state.config_handle();
    let dirs_clone = (*state.dirs).clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();
    let db_clone = state.db.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        // Persist first
        {
            let _guard = state_clone.config_lock.lock();
            match crate::config::Config::load_with_env(&config_path) {
                Ok(mut fresh_config) => {
                    if let Some(ref mut headless) = fresh_config.headless {
                        headless
                            .clients
                            .push(crate::config::HeadlessClientDef::default());
                    }
                    if let Err(e) = fresh_config.save(&config_path) {
                        tracing::error!(err = %e, "Failed to save new client to config");
                        return;
                    } else {
                        *config_handle.write() = fresh_config;
                    }
                }
                Err(e) => {
                    tracing::error!(err = %e, "Failed to reload config for adding client");
                    return;
                }
            }
        }

        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &dirs_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
            &db_clone,
        )
        .await;

        if let Err(e) = result {
            tracing::error!(err = %e, "Convergence failed after creating client");
        } else {
            tracing::info!(count = new_count, "Client created successfully");
        }
    });

    set_flash(
        &session,
        &format!("Creating client {new_count}..."),
        FlashType::Success,
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/headless"))
        .finish())
}

#[allow(deprecated)]
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

    let headless_config = match state.config().headless.clone() {
        Some(h) => h,
        None => {
            set_flash(
                &session,
                "No headless config in quartermaster.toml",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    // Validate index (1-based)
    if index == 0 || index > headless_config.client_count() {
        set_flash(
            &session,
            &format!("Client {index} does not exist"),
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/headless"))
            .finish());
    }

    // Check if this client is in raid
    if let Some(states) = &state.client_states {
        let clients = states.read().await;
        if let Some(client) = clients.iter().find(|c| c.index == index) {
            if matches!(client.fika_status, Some(EHeadlessStatus::InRaid))
                && !client.players.is_empty()
            {
                set_flash(
                    &session,
                    &format!("Client {index} is in raid. Wait for it to finish or stop it first."),
                    FlashType::Error,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/headless"))
                    .finish());
            }
        }
    }

    let container_mgr = match require_container_mgr(&state, &session) {
        Ok(m) => m,
        Err(resp) => return Ok(resp),
    };

    // Build updated config with client removed (0-based index into vec)
    let mut updated_config = headless_config;
    updated_config.clients.remove((index - 1) as usize);

    // Create SPT client
    let spt_client = match create_spt_client(&state, &session) {
        Ok(c) => c,
        Err(resp) => return Ok(resp),
    };

    // Stop and remove the container, persist config, then converge
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config_cloned();
    let config_path = state.config_path.clone();
    let config_handle = state.config_handle();
    let dirs_clone = (*state.dirs).clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();
    let db_clone = state.db.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        // Remove ALL managed containers so converge recreates with correct indices
        if let Err(e) = crate::client::converge::remove_all_managed_containers(&mgr_clone).await {
            tracing::error!(err = %e, "Failed to remove managed containers before re-convergence");
            return;
        }

        // Persist
        {
            let _guard = state_clone.config_lock.lock();
            match crate::config::Config::load_with_env(&config_path) {
                Ok(mut fresh_config) => {
                    if let Some(ref mut headless) = fresh_config.headless {
                        if (index as usize) <= headless.clients.len() && index > 0 {
                            headless.clients.remove((index - 1) as usize);
                        }
                    }
                    if let Err(e) = fresh_config.save(&config_path) {
                        tracing::error!(err = %e, "Failed to save config after deleting client");
                        return;
                    } else {
                        *config_handle.write() = fresh_config;
                    }
                }
                Err(e) => {
                    tracing::error!(err = %e, "Failed to reload config for deleting client");
                    return;
                }
            }
        }

        // Clean up overlay for the deleted index
        let overlay =
            crate::client::converge::client_overlay_dir(&updated_config.install_dir, index);
        if overlay.exists() {
            if let Err(e) = std::fs::remove_dir_all(&overlay) {
                tracing::warn!(err = %e, "Failed to clean overlay dir for deleted client {index}");
            }
        }

        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &dirs_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
            &db_clone,
        )
        .await;

        if let Err(e) = result {
            tracing::error!(err = %e, "Convergence failed after deleting client");
        } else {
            tracing::info!(deleted_index = index, "Client deleted successfully");
        }
    });

    set_flash(
        &session,
        &format!("Deleting client {index}..."),
        FlashType::Success,
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/headless"))
        .finish())
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
    let fika_client = match state.fika_client.as_ref() {
        Some(c) => c.clone(),
        None => {
            set_flash(&session, "Fika integration not available", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
    };

    // Get profile_id and check READY status
    let profile_id = {
        let states = state
            .client_states
            .as_ref()
            .ok_or(WebError::Internal(anyhow::anyhow!(
                "Headless not configured"
            )))?
            .read()
            .await;
        let client = states
            .iter()
            .find(|c| c.index == index)
            .ok_or(WebError::NotFound)?;
        if client.fika_status != Some(EHeadlessStatus::Ready) {
            set_flash(
                &session,
                &format!("Client {index} is not READY"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/headless"))
                .finish());
        }
        client
            .profile_id
            .clone()
            .ok_or(WebError::Internal(anyhow::anyhow!("No profile ID")))?
    };

    let req_body = crate::fika::client::StartHeadlessRaidRequest {
        headless_session_id: profile_id,
        location_id: form.location_id.clone(),
        time: form.time,
        time_and_weather_settings: None,
        use_event: form.use_event.is_some(),
        side: 0,
        spawn_place: 0,
        metabolism_disabled: false,
        bot_settings: None,
        waves_settings: None,
        custom_raid_settings: None,
    };

    match fika_client.start_headless_raid(&req_body).await {
        Ok(resp) => {
            if let Some(err) = resp.error {
                set_flash(
                    &session,
                    &format!("Start raid failed: {err}"),
                    FlashType::Error,
                );
            } else {
                set_flash(
                    &session,
                    &format!("Raid started on client {index}"),
                    FlashType::Success,
                );
            }
        }
        Err(e) => {
            set_flash(
                &session,
                &format!("Start raid failed: {e}"),
                FlashType::Error,
            );
        }
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

    let profile_id = match &state.client_states {
        Some(states) => {
            let clients = states.read().await;
            clients
                .iter()
                .find(|c| c.index == index)
                .and_then(|c| c.profile_id.clone())
        }
        None => None,
    };

    let profile_id = match profile_id {
        Some(pid) => pid,
        None => {
            set_flash(
                &session,
                "Cannot rename: client has no profile assigned yet. Start the SPT server first.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/headless/{index}")))
                .finish());
        }
    };

    let spt_dir = state.dirs.spt_server.clone();
    let new_name = form.into_inner().name.trim().to_string();

    let result = actix_web::web::block(move || {
        let _guard = state.fika_config_lock.lock();
        let path = crate::fika::config::fika_config_path(&spt_dir);

        // Read current aliases from typed config
        let config = crate::fika::config::read_fika_config(&path)?;
        let mut aliases = config.headless.profiles.aliases;

        if new_name.is_empty() {
            aliases.remove(&profile_id);
        } else {
            aliases.insert(profile_id, new_name);
        }

        // Read CST, set aliases, write back
        let cst = crate::fika::config::read_fika_cst(&path)?;
        let root = cst.object_value_or_set();
        if let Some(headless) = root.object_value("headless") {
            if let Some(profiles) = headless.object_value("profiles") {
                let alias_entries: Vec<(String, CstInputValue)> = aliases
                    .into_iter()
                    .map(|(k, v)| (k, CstInputValue::String(v)))
                    .collect();
                match profiles.get("aliases") {
                    Some(prop) => prop.set_value(CstInputValue::Object(alias_entries)),
                    None => {
                        profiles.append("aliases", CstInputValue::Object(alias_entries));
                    }
                }
            }
        }
        crate::fika::config::write_fika_cst(&cst, &path)?;

        Ok::<(), anyhow::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) => set_flash(
            &session,
            "Client renamed. Restart the SPT server for the in-game name to take effect.",
            FlashType::Success,
        ),
        Ok(Err(e)) => {
            tracing::warn!("failed to rename client: {e:#}");
            set_flash(
                &session,
                &format!("Failed to rename: {e}"),
                FlashType::Error,
            );
        }
        Err(e) => {
            tracing::error!(err = %e, "task failed renaming client");
            set_flash(&session, "Failed to rename client", FlashType::Error);
        }
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
    let new_image = form.into_inner().image.trim().to_string();

    let _guard = state.config_lock.lock();
    let mut config =
        crate::config::Config::load_with_env(&state.config_path).map_err(WebError::from)?;

    let headless = match config.headless.as_mut() {
        Some(h) => h,
        None => {
            set_flash(&session, "No headless config", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/headless/{index}")))
                .finish());
        }
    };

    let client_index = (index as usize).checked_sub(1).ok_or(WebError::NotFound)?;
    let client_def = headless
        .clients
        .get_mut(client_index)
        .ok_or(WebError::NotFound)?;

    client_def.image = if new_image.is_empty() || new_image == headless.image {
        None
    } else {
        Some(new_image)
    };

    state.persist_config(&config)?;

    set_flash(
        &session,
        "Client image updated. Re-converge to apply.",
        FlashType::Success,
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
        .finish())
}

pub async fn dashboard_clients_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<web::Html> {
    require_auth(&req)?;

    let raw_clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let dirs = (*state.dirs).clone();
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
