use actix_session::Session;
use actix_web::web::{self, Data, Form, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::client::{ClientHealth, ClientState};
use crate::db::users::Role;
use crate::spt::headless::EHeadlessStatus;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage};
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "clients/list.html")]
struct ClientsListTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    #[allow(dead_code)]
    fika_installed: bool,
    #[allow(dead_code)]
    modsync_installed: bool,
    clients: Vec<ClientState>,
    converging: bool,
    target_count: u32,
}

#[derive(Template)]
#[template(path = "clients/detail.html")]
struct ClientDetailTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    #[allow(dead_code)]
    fika_installed: bool,
    #[allow(dead_code)]
    modsync_installed: bool,
    client: ClientState,
}

#[derive(Template)]
#[template(path = "clients/partials/status.html")]
struct ClientsStatusPartialTemplate {
    clients: Vec<ClientState>,
}

#[derive(Template)]
#[template(path = "partials/dashboard_clients_status.html")]
struct DashboardClientsStatusTemplate {
    healthy_count: usize,
    degraded_count: usize,
    down_count: usize,
    total_count: usize,
}

pub async fn client_list(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<web::Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let converging = state.converging.load(std::sync::atomic::Ordering::Relaxed);
    let target_count = state.config.clients.as_ref().map(|c| c.count).unwrap_or(0);

    let tmpl = ClientsListTemplate {
        user,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        clients,
        converging,
        target_count,
    };
    Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn client_detail(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<u32>,
) -> actix_web::Result<web::Html> {
    let user = require_auth(&req)?;
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

    let tmpl = ClientDetailTemplate {
        user,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        client,
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
    require_capability(&user, Role::can_control_server)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

    // Find the container name for this client
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

    let container_name = match container_name {
        Some(name) => name,
        None => {
            set_flash(&session, &format!("Client {index} not found"), "error");
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

    if let Err(e) = mgr.restart(&container_name).await {
        tracing::error!(container = %container_name, error = %e, "failed to restart client");
        set_flash(
            &session,
            &format!("Failed to restart client {index}: {e}"),
            "error",
        );
    } else {
        tracing::info!(container = %container_name, "client restarted");
        set_flash(&session, &format!("Client {index} restarting"), "success");
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/clients/{index}")))
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
    require_capability(&user, Role::can_control_server)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

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

    let container_name = match container_name {
        Some(name) => name,
        None => {
            set_flash(&session, &format!("Client {index} not found"), "error");
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

    if let Err(e) = mgr.stop(&container_name).await {
        tracing::error!(container = %container_name, error = %e, "failed to stop client");
        set_flash(
            &session,
            &format!("Failed to stop client {index}: {e}"),
            "error",
        );
    } else {
        tracing::info!(container = %container_name, "client stopped");
        set_flash(&session, &format!("Client {index} stopped"), "success");
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/clients/{index}")))
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
    require_capability(&user, Role::can_control_server)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let index = path.into_inner();

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

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

    let container_name = match container_name {
        Some(name) => name,
        None => {
            set_flash(&session, &format!("Client {index} not found"), "error");
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

    if let Err(e) = mgr.start(&container_name).await {
        tracing::error!(container = %container_name, error = %e, "failed to start client");
        set_flash(
            &session,
            &format!("Failed to start client {index}: {e}"),
            "error",
        );
    } else {
        tracing::info!(container = %container_name, "client started");
        set_flash(&session, &format!("Client {index} starting"), "success");
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/clients/{index}")))
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
    require_capability(&user, Role::can_control_server)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target = form.count;
    let force = form.force;

    // Check if we have Fika clients configured
    if state.config.clients.is_none() {
        set_flash(
            &session,
            "No clients configured in quartermaster.toml",
            "error",
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/clients"))
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
                .filter(|c| {
                    matches!(
                        c.fika_status,
                        Some(EHeadlessStatus::InRaid | EHeadlessStatus::Ready)
                    )
                })
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
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    }

    // Get required dependencies
    let container_mgr = match state.container_mgr.as_ref() {
        Some(mgr) => mgr.as_ref(),
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

    let clients_config = state.config.clients.as_ref().unwrap(); // Already checked above
    let mut updated_config = clients_config.clone();
    updated_config.count = target;

    // Create SPT client
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = match crate::spt::server::SptClient::new(&host, port) {
        Ok(client) => client,
        Err(e) => {
            set_flash(
                &session,
                &format!("Failed to create SPT client: {e}"),
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/clients"))
                .finish());
        }
    };

    // Run convergence in a background task
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config.clone();
    let config_path = state.config_path.clone();
    let spt_dir_clone = state.spt_dir.clone();
    let converging_clone = state.converging.clone();

    tokio::spawn(async move {
        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &spt_dir_clone,
            &spt_client,
            converging_clone,
        )
        .await;

        if let Err(e) = result {
            tracing::error!(error = %e, "Client convergence failed during scale operation");
        } else {
            tracing::info!(target_count = target, "Client scaling completed");

            // Persist the new client count to config file
            match crate::config::Config::load_with_env(&config_path) {
                Ok(mut fresh_config) => {
                    if let Some(ref mut clients) = fresh_config.clients {
                        clients.count = target;
                    }
                    if let Err(e) = fresh_config.save(&config_path) {
                        tracing::error!(error = %e, "Failed to save updated client count to config");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to reload config for persisting client count");
                }
            }
        }
    });

    set_flash(
        &session,
        &format!("Scaling to {target} client(s)..."),
        "success",
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/clients"))
        .finish())
}

pub async fn client_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<web::Html> {
    require_auth(&req)?;

    let clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let tmpl = ClientsStatusPartialTemplate { clients };
    Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dashboard_clients_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<web::Html> {
    require_auth(&req)?;

    let clients = match &state.client_states {
        Some(states) => states.read().await.clone(),
        None => vec![],
    };

    let healthy_count = clients
        .iter()
        .filter(|c| c.health == ClientHealth::Healthy)
        .count();
    let degraded_count = clients
        .iter()
        .filter(|c| c.health == ClientHealth::Degraded)
        .count();
    let down_count = clients
        .iter()
        .filter(|c| matches!(c.health, ClientHealth::Down | ClientHealth::GivenUp))
        .count();
    let total_count = clients.len();

    let tmpl = DashboardClientsStatusTemplate {
        healthy_count,
        degraded_count,
        down_count,
        total_count,
    };
    Ok(web::Html::new(tmpl.render().map_err(WebError::from)?))
}
