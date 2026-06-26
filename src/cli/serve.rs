use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};

use super::Cli;
use crate::config::{is_fika_installed, Config};
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::logging;
use crate::spt::detect::{detect_spt_dir, read_spt_version};

pub async fn run(bind: Option<&str>, port: Option<u16>, cli: &Cli) -> Result<()> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let spt_info = read_spt_version(&spt_dir)?;

    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = Config::load_with_env(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    if let Some(b) = bind {
        config.web_bind = b.to_string();
    }
    if let Some(p) = port {
        config.web_port = p;
    }

    // Create LogBroadcast with configured buffer size
    let log_broadcast = Arc::new(logging::LogBroadcast::new(config.logging.web.buffer_size));
    let reload_handles = logging::init_subscriber(&log_broadcast);

    // Reconfigure logging now that config is loaded
    let filter =
        logging::resolve_log_filter(&config.logging, cli.verbose, cli.log_level.as_deref());
    reload_handles.reconfigure(&config.logging, &filter, Some(&spt_dir));

    config.ensure_session_secret();
    config
        .save(&config_path)
        .context("failed to save config with session secret")?;

    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    if !db.admin_exists()? {
        anyhow::bail!("No admin user exists. Run `quma setup` first to create an admin account.");
    }

    // Create ContainerManager if available
    let container_mgr = match crate::container::ContainerManager::new() {
        Ok(mgr) => {
            let mgr = Arc::new(mgr);

            // Auto-start server container if configured
            if config.auto_start_server {
                if let Some(ref container) = config.server_container {
                    match mgr.is_running(container).await {
                        Ok(true) => {
                            tracing::info!(container, "server container already running");
                        }
                        Ok(false) => {
                            tracing::info!(container, "auto-starting server container");
                            if let Err(e) = mgr.start(container).await {
                                tracing::warn!(container, error = %e, "failed to auto-start server container — web UI will start anyway");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(container, error = %e, "failed to check container status — skipping auto-start");
                        }
                    }
                }
            }

            Some(mgr)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to connect to Podman — container features disabled");
            None
        }
    };

    let forge = ForgeClient::new(config.forge_token.clone())?;

    let fika_installed = is_fika_installed(&spt_dir);

    // Auto-install Fika client mod if Fika is installed but the client plugin isn't tracked
    if fika_installed
        && db
            .get_mod_by_forge_id(crate::config::FIKA_CLIENT_FORGE_ID)?
            .is_none()
    {
        // TODO(debt): download_and_install uses println! which bypasses tracing — consider
        // a quiet/logging mode for non-CLI callers.
        tracing::info!("Fika detected but client mod not installed — auto-installing");
        if let Err(e) = auto_install_bootstrap_mod(
            &forge,
            &db,
            &spt_dir,
            &config,
            crate::config::FIKA_CLIENT_FORGE_ID,
            &spt_info.spt_version,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to auto-install Fika client — bootstrap zip may be incomplete");
        }
    }

    let modsync_installed = crate::config::is_modsync_installed(&spt_dir);
    let converging = Arc::new(AtomicBool::new(false));

    // Initialize client supervisor if Fika is installed and headless clients configured
    let client_states = if let Some(ref headless_config) = config.headless {
        if fika_installed && headless_config.client_count() > 0 {
            if let Some(ref container_mgr_arc) = container_mgr {
                // Validate headless config
                if let Err(e) = headless_config.validate(&config, &spt_dir) {
                    tracing::error!(error = %e, "Invalid headless configuration — supervisor not started");
                    None
                } else {
                    // Resolve SPT server address
                    let (host, port) = crate::server_detect::resolve_server_addr(&config, &spt_dir);
                    let spt_client = match crate::spt::server::SptClient::new(&host, port) {
                        Ok(client) => client,
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to create SPT client — supervisor not started");
                            return Err(e);
                        }
                    };

                    // Run initial convergence
                    tracing::info!(
                        "Running initial convergence for {} headless client(s)",
                        headless_config.client_count()
                    );
                    let converge_result = crate::client::converge::converge(
                        container_mgr_arc,
                        headless_config,
                        &config,
                        &spt_dir,
                        &spt_client,
                        &forge,
                        &spt_info.spt_version,
                        Arc::clone(&converging),
                    )
                    .await;

                    if let Err(e) = converge_result {
                        tracing::error!(error = %e, "Initial convergence failed — supervisor not started");
                        None
                    } else {
                        // Create and spawn supervisor
                        let cancel_token = tokio_util::sync::CancellationToken::new();
                        let supervisor = crate::client::supervisor::ClientSupervisor::new(
                            container_mgr_arc.as_ref().clone(),
                            spt_client,
                            headless_config.clone(),
                            Arc::clone(&converging),
                            cancel_token,
                        );

                        let states = supervisor.state();
                        supervisor.run();

                        tracing::info!(
                            "ClientSupervisor started for {} headless client(s)",
                            headless_config.client_count()
                        );
                        Some(states)
                    }
                }
            } else {
                tracing::warn!(
                    "Headless clients configured but Podman unavailable — supervisor not started"
                );
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let on_exit = config.on_exit.clone();
    let teardown_mgr = container_mgr.clone();

    let server_future = crate::web::start_server(crate::web::ServerContext {
        config,
        config_path,
        db,
        forge,
        spt_dir,
        spt_info,
        log_broadcast: Arc::clone(&log_broadcast),
        reload_handles: Arc::new(reload_handles),
        container_mgr,
        client_states,
        converging,
        fika_installed,
        modsync_installed,
    });

    // Actix-web handles SIGINT/SIGTERM internally. For SIGHUP, we race
    // the server future against the signal — dropping the future triggers
    // actix's cleanup. When on_exit is Nothing, skip the signal listener
    // entirely so there's zero overhead.
    let server_result = if on_exit != crate::config::OnExit::Nothing {
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to register SIGHUP handler");
        tokio::select! {
            result = server_future => result,
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP, shutting down");
                Ok(())
            }
        }
    } else {
        server_future.await
    };

    if let Some(ref mgr) = teardown_mgr {
        teardown_containers(mgr, &on_exit).await;
    }

    server_result
}

async fn teardown_containers(
    container_mgr: &crate::container::ContainerManager,
    on_exit: &crate::config::OnExit,
) {
    use crate::config::OnExit;

    if *on_exit == OnExit::Nothing {
        return;
    }

    tracing::info!(mode = %on_exit, "tearing down managed containers");

    let containers = match container_mgr
        .detect_containers_by_label("managed-by", "quma")
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to discover managed containers for teardown");
            return;
        }
    };

    if containers.is_empty() {
        tracing::debug!("no managed containers found");
        return;
    }

    for name in &containers {
        let result = match on_exit {
            OnExit::Stop => {
                tracing::info!(container = %name, "stopping container");
                container_mgr.stop(name).await
            }
            OnExit::Remove => {
                tracing::info!(container = %name, "removing container");
                container_mgr.remove_container(name).await
            }
            OnExit::Nothing => return,
        };
        if let Err(e) = result {
            tracing::warn!(container = %name, error = %e, "container teardown failed");
        }
    }

    tracing::info!(count = containers.len(), "container teardown complete");
}

async fn auto_install_bootstrap_mod(
    forge: &ForgeClient,
    db: &Database,
    spt_dir: &std::path::Path,
    config: &Config,
    forge_mod_id: i64,
    spt_version: &str,
) -> Result<()> {
    let forge_mod = forge.get_mod(forge_mod_id, false).await?;
    let versions = forge.get_versions(forge_mod_id, Some(spt_version)).await?;
    let version = versions.into_iter().max_by_key(|v| v.id).ok_or_else(|| {
        anyhow::anyhow!(
            "no compatible version of {} for SPT {}",
            forge_mod.name,
            spt_version
        )
    })?;
    let download_url = version.link.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "no download link for {} v{}",
            forge_mod.name,
            version.version
        )
    })?;

    crate::cli::install::download_and_install(
        forge,
        db,
        spt_dir,
        config,
        &crate::cli::install::ModInstallParams {
            forge_mod_id,
            forge_version_id: version.id,
            download_url,
            name: &forge_mod.name,
            slug: forge_mod.slug.as_deref(),
            version: &version.version,
        },
    )
    .await?;

    tracing::info!(name = %forge_mod.name, version = %version.version, "auto-installed bootstrap mod");
    Ok(())
}
