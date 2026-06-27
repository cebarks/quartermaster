use actix_session::Session;
use actix_web::web::{self, Data, Form, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::client::{ClientHealth, ClientState};
use crate::db::rbac::Permission;
use crate::spt::headless::EHeadlessStatus;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "clients/detail.html")]
struct ClientDetailTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
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

pub async fn client_list(req: HttpRequest) -> actix_web::Result<HttpResponse> {
    require_auth(&req)?;
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=headless"))
        .finish())
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
        nav: NavContext::from_state(&state),
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
    require_permission(&user, Permission::HeadlessManage)?;

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
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
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
            set_flash(
                &session,
                &format!("Client {index} not found"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    if let Err(e) = mgr.restart(&container_name).await {
        tracing::error!(container = %container_name, err = %e, "failed to restart client");
        set_flash(
            &session,
            &format!("Failed to restart client {index}: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container = %container_name, "client restarted");
        set_flash(
            &session,
            &format!("Client {index} restarting"),
            FlashType::Success,
        );
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/headless/{index}")))
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

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
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
            set_flash(
                &session,
                &format!("Client {index} not found"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    if let Err(e) = mgr.stop(&container_name).await {
        tracing::error!(container = %container_name, err = %e, "failed to stop client");
        set_flash(
            &session,
            &format!("Failed to stop client {index}: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container = %container_name, "client stopped");
        set_flash(
            &session,
            &format!("Client {index} stopped"),
            FlashType::Success,
        );
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

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
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
            set_flash(
                &session,
                &format!("Client {index} not found"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    if let Err(e) = mgr.start(&container_name).await {
        tracing::error!(container = %container_name, err = %e, "failed to start client");
        set_flash(
            &session,
            &format!("Failed to start client {index}: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container = %container_name, "client started");
        set_flash(
            &session,
            &format!("Client {index} starting"),
            FlashType::Success,
        );
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

    // Check if we have headless clients configured
    if state.config().headless.is_none() {
        set_flash(
            &session,
            "No headless config in quartermaster.toml",
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/settings?tab=headless"))
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
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
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
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    let headless_config = state
        .config()
        .headless
        .as_ref()
        .expect("None case returned above")
        .clone();
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
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.spt_dir);
    let spt_client = match crate::spt::server::SptClient::new(&host, port) {
        Ok(client) => client,
        Err(e) => {
            set_flash(
                &session,
                &format!("Failed to create SPT client: {e}"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    // Run convergence in a background task
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config_cloned();
    let config_path = state.config_path.clone();
    let config_handle = state.config_handle();
    let spt_dir_clone = state.spt_dir.clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();

    tokio::spawn(async move {
        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &spt_dir_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
        )
        .await;

        if let Err(e) = result {
            tracing::error!(err = %e, "Client convergence failed during scale operation");
        } else {
            tracing::info!(target_count = target, "Client scaling completed");

            // Persist the updated client count to config file
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
    });

    set_flash(
        &session,
        &format!("Scaling to {target} client(s)..."),
        FlashType::Success,
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings?tab=headless"))
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
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    let container_mgr = match state.container_mgr.as_ref() {
        Some(mgr) => mgr.as_ref(),
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    // Build updated config with one new default client appended
    let mut updated_config = headless_config;
    updated_config
        .clients
        .push(crate::config::HeadlessClientDef::default());
    let new_count = updated_config.client_count();

    // Create SPT client
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.spt_dir);
    let spt_client = match crate::spt::server::SptClient::new(&host, port) {
        Ok(client) => client,
        Err(e) => {
            set_flash(
                &session,
                &format!("Failed to create SPT client: {e}"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    // Persist to config and spawn convergence
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config_cloned();
    let config_path = state.config_path.clone();
    let config_handle = state.config_handle();
    let spt_dir_clone = state.spt_dir.clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();

    tokio::spawn(async move {
        // Persist first
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

        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &spt_dir_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
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
        .insert_header(("Location", "/quma/settings?tab=headless"))
        .finish())
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

    let headless_config = match state.config().headless.clone() {
        Some(h) => h,
        None => {
            set_flash(
                &session,
                "No headless config in quartermaster.toml",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
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
            .insert_header(("Location", "/quma/settings?tab=headless"))
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
                    .insert_header(("Location", "/quma/settings?tab=headless"))
                    .finish());
            }
        }
    }

    let container_mgr = match state.container_mgr.as_ref() {
        Some(mgr) => mgr.as_ref(),
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    // Build updated config with client removed (0-based index into vec)
    let mut updated_config = headless_config;
    updated_config.clients.remove((index - 1) as usize);

    // Create SPT client
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.spt_dir);
    let spt_client = match crate::spt::server::SptClient::new(&host, port) {
        Ok(client) => client,
        Err(e) => {
            set_flash(
                &session,
                &format!("Failed to create SPT client: {e}"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/settings?tab=headless"))
                .finish());
        }
    };

    // Stop and remove the container, persist config, then converge
    let mgr_clone = container_mgr.clone();
    let config_clone = state.config_cloned();
    let config_path = state.config_path.clone();
    let config_handle = state.config_handle();
    let spt_dir_clone = state.spt_dir.clone();
    let converging_clone = state.converging.clone();
    let forge_clone = state.forge.clone();
    let spt_version_clone = state.spt_info.spt_version.clone();

    tokio::spawn(async move {
        // Stop/remove the container for the deleted index
        let container_name = crate::client::converge::client_container_name(index);
        if let Ok(true) = mgr_clone.is_running(&container_name).await {
            if let Err(e) = mgr_clone.stop(&container_name).await {
                tracing::warn!(err = %e, container = %container_name, "failed to stop container before delete");
            }
        }
        if let Err(e) = mgr_clone.remove_container(&container_name).await {
            tracing::warn!(err = %e, container = %container_name, "failed to remove container (may not exist)");
        }

        // Persist
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

        let result = crate::client::converge::converge(
            &mgr_clone,
            &updated_config,
            &config_clone,
            &spt_dir_clone,
            &spt_client,
            &forge_clone,
            &spt_version_clone,
            converging_clone,
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
        .insert_header(("Location", "/quma/settings?tab=headless"))
        .finish())
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
