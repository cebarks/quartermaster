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
        anyhow::bail!("No admin user exists. Run `quma init` first to create an admin account.");
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

    let fika_installed = is_fika_installed(&spt_dir);
    let converging = Arc::new(AtomicBool::new(false));

    // Initialize client supervisor if Fika is installed and clients configured
    let client_states = if let Some(ref clients_config) = config.clients {
        if fika_installed && clients_config.count > 0 {
            if let Some(ref container_mgr_arc) = container_mgr {
                // Validate client config
                if let Err(e) = clients_config.validate(&config, &spt_dir) {
                    tracing::error!(error = %e, "Invalid client configuration — supervisor not started");
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
                    // Note: converge() uses Arc<RwLock<bool>> for its internal flag management,
                    // separate from the supervisor's Arc<AtomicBool> used for monitoring
                    tracing::info!(
                        "Running initial convergence for {} client(s)",
                        clients_config.count
                    );
                    let converge_flag = Arc::new(tokio::sync::RwLock::new(false));
                    let converge_result = crate::client::converge::converge(
                        container_mgr_arc,
                        clients_config,
                        &config,
                        &spt_dir,
                        &spt_client,
                        converge_flag,
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
                            clients_config.clone(),
                            Arc::clone(&converging),
                            cancel_token,
                        );

                        let states = supervisor.state();
                        supervisor.run();

                        tracing::info!(
                            "ClientSupervisor started for {} client(s)",
                            clients_config.count
                        );
                        Some(states)
                    }
                }
            } else {
                tracing::warn!(
                    "Fika clients configured but Podman unavailable — supervisor not started"
                );
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let forge = ForgeClient::new(config.forge_token.clone())?;

    crate::web::start_server(
        config,
        config_path,
        db,
        forge,
        spt_dir,
        spt_info,
        Arc::clone(&log_broadcast),
        container_mgr,
        client_states,
        converging,
        fika_installed,
    )
    .await
}
