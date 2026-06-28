use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::Cli;
use crate::config::{self, is_fika_installed, Config};
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

    // Step 1: create mpsc channel before subscriber
    let (log_tx, log_rx) = mpsc::unbounded_channel();

    // Step 2: init subscriber with sender
    let reload_handles = logging::init_subscriber(&log_broadcast, Some(log_tx));

    // Reconfigure logging now that config is loaded
    let filter =
        logging::resolve_log_filter(&config.logging, cli.verbose, cli.log_level.as_deref());

    let mut logging_config = config.logging.clone();
    if let Some(ref fmt) = cli.log_format {
        if let Ok(format) = fmt.parse::<config::ConsoleFormat>() {
            logging_config.console.format = format;
        }
    }

    reload_handles.reconfigure(&logging_config, &filter, Some(&spt_dir));

    config.ensure_session_secret();
    config
        .save(&config_path)
        .context("failed to save config with session secret")?;

    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    // Step 3: spawn LogWriter after DB is available
    let db_arc = Arc::new(Mutex::new(db));
    let (_log_writer_handle, log_writer_shutdown) = crate::logging::writer::spawn(
        Arc::clone(&db_arc),
        log_rx,
        config.logging.web.retention_days,
        config.logging.web.max_entries,
    );

    if !db_arc.lock().admin_exists()? {
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
                                tracing::warn!(container, err = %e, "failed to auto-start server container — web UI will start anyway");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(container, err = %e, "failed to check container status — skipping auto-start");
                        }
                    }
                }
            }

            Some(mgr)
        }
        Err(e) => {
            tracing::warn!(err = %e, "failed to connect to Podman — container features disabled");
            None
        }
    };

    let forge = ForgeClient::new(config.forge_token.clone())?;

    let fika_installed = is_fika_installed(&spt_dir);

    // Auto-install Fika client mod if Fika is installed but the client plugin isn't tracked
    if fika_installed
        && db_arc
            .lock()
            .get_mod_by_forge_id(crate::config::FIKA_CLIENT_FORGE_ID)?
            .is_none()
    {
        tracing::info!("Fika detected but client mod not installed — auto-installing");
        if let Err(e) = auto_install_bootstrap_mod(
            &forge,
            &db_arc,
            &spt_dir,
            &config,
            crate::config::FIKA_CLIENT_FORGE_ID,
            &spt_info.spt_version,
        )
        .await
        {
            tracing::warn!(err = %e, "failed to auto-install Fika client — bootstrap zip may be incomplete");
        }
    }

    let modsync_installed = crate::config::is_modsync_installed(&spt_dir);
    let converging = Arc::new(AtomicBool::new(false));
    let config_arc = Arc::new(parking_lot::RwLock::new(config));

    // Initialize client supervisor. Always start it when Fika is installed and
    // Podman is available so that clients added via the web UI are picked up
    // without requiring a server restart.
    let config = config_arc.read().clone();
    let client_states = if fika_installed {
        if let Some(ref container_mgr_arc) = container_mgr {
            let (host, port) = crate::server_detect::resolve_server_addr(&config, &spt_dir);
            match crate::spt::server::SptClient::new(&host, port) {
                Ok(spt_client) => {
                    // Run initial convergence if clients are already configured
                    if let Some(ref headless_config) = config.headless {
                        if headless_config.client_count() > 0 {
                            if let Err(e) = headless_config.validate(&config, &spt_dir) {
                                tracing::error!(err = %e, "Invalid headless configuration — skipping initial convergence");
                            } else {
                                tracing::info!(
                                    "Running initial convergence for {} headless client(s)",
                                    headless_config.client_count()
                                );
                                if let Err(e) = crate::client::converge::converge(
                                    container_mgr_arc,
                                    headless_config,
                                    &config,
                                    &spt_dir,
                                    &spt_client,
                                    &forge,
                                    &spt_info.spt_version,
                                    Arc::clone(&converging),
                                )
                                .await
                                {
                                    tracing::error!(err = %e, "Initial convergence failed");
                                }
                            }
                        }
                    }

                    let cancel_token = tokio_util::sync::CancellationToken::new();
                    let supervisor = crate::client::supervisor::ClientSupervisor::new(
                        container_mgr_arc.as_ref().clone(),
                        spt_client,
                        Arc::clone(&config_arc),
                        Arc::clone(&converging),
                        cancel_token,
                    );

                    let states = supervisor.state();
                    supervisor.run();

                    let client_count = config
                        .headless
                        .as_ref()
                        .map(|h| h.client_count())
                        .unwrap_or(0);
                    tracing::info!(
                        "ClientSupervisor started (managing {client_count} headless client(s))"
                    );
                    Some(states)
                }
                Err(e) => {
                    let has_clients = config
                        .headless
                        .as_ref()
                        .is_some_and(|h| h.client_count() > 0);
                    if has_clients {
                        tracing::error!(err = %e, "Failed to create SPT client — supervisor not started");
                        return Err(e);
                    }
                    tracing::warn!(err = %e, "Failed to create SPT client — supervisor not started");
                    None
                }
            }
        } else {
            if config
                .headless
                .as_ref()
                .is_some_and(|h| h.client_count() > 0)
            {
                tracing::warn!(
                    "Headless clients configured but Podman unavailable — supervisor not started"
                );
            }
            None
        }
    } else {
        None
    };

    let on_exit = config.on_exit.clone();
    let teardown_mgr = container_mgr.clone();

    let server_future = crate::web::start_server(crate::web::ServerContext {
        config,
        config_handle: config_arc,
        config_path,
        db: db_arc,
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

    // Shutdown log writer before tearing down containers
    log_writer_shutdown.shutdown().await;

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
            tracing::error!(err = %e, "failed to discover managed containers for teardown");
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
            tracing::warn!(container = %name, err = %e, "container teardown failed");
        }
    }

    tracing::info!(count = containers.len(), "container teardown complete");
}

async fn auto_install_bootstrap_mod(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
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

    crate::cli::install::download_and_install_with_arc(
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

    tracing::info!(mod_name = %forge_mod.name, version = %version.version, "auto-installed bootstrap mod");
    Ok(())
}
