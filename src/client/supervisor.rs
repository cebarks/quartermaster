use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use crate::client::{ClientHealth, ClientState, ContainerStatus};
use crate::config::{Config, HeadlessConfig, RestartPolicy};
use crate::container::ContainerManager;
use crate::spt::headless::{EHeadlessStatus, GetHeadlessesResponse};
use crate::spt::server::SptClient;

struct RestartingGuard {
    state: Arc<RwLock<Vec<ClientState>>>,
    index: u32,
}

impl Drop for RestartingGuard {
    fn drop(&mut self) {
        let state = self.state.clone();
        let index = self.index;
        tokio::spawn(async move {
            let mut state_lock = state.write().await;
            if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
                s.restarting = false;
            }
        });
    }
}

pub struct ClientSupervisor {
    container_mgr: ContainerManager,
    spt_client: SptClient,
    config: Arc<parking_lot::RwLock<Config>>,
    config_path: PathBuf,
    db: Arc<parking_lot::Mutex<crate::db::Database>>,
    converging: Arc<AtomicBool>,
    cancel_token: CancellationToken,
    state: Arc<RwLock<Vec<ClientState>>>,
    tick_interval: Duration,
    watcher_handles: Arc<RwLock<HashMap<u32, CancellationToken>>>,
}

impl ClientSupervisor {
    pub fn new(
        container_mgr: ContainerManager,
        spt_client: SptClient,
        config: Arc<parking_lot::RwLock<Config>>,
        config_path: PathBuf,
        db: Arc<parking_lot::Mutex<crate::db::Database>>,
        converging: Arc<AtomicBool>,
        cancel_token: CancellationToken,
    ) -> Self {
        let state = Arc::new(RwLock::new(Vec::new()));
        Self {
            container_mgr,
            spt_client,
            config,
            config_path,
            db,
            converging,
            cancel_token,
            state,
            tick_interval: Duration::from_secs(15),
            watcher_handles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn state(&self) -> Arc<RwLock<Vec<ClientState>>> {
        Arc::clone(&self.state)
    }

    pub fn run(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            self.monitor_loop().await;
        })
    }

    async fn monitor_loop(self) {
        let mut ticker = interval(self.tick_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    tracing::info!("ClientSupervisor shutting down");
                    // Cancel all exit watchers
                    let handles = self.watcher_handles.read().await;
                    for token in handles.values() {
                        token.cancel();
                    }
                    break;
                }
                _ = ticker.tick() => {
                    if self.converging.load(Ordering::Relaxed) {
                        tracing::trace!("Skipping client monitor tick (convergence in progress)");
                        continue;
                    }
                    if let Err(e) = self.tick().await {
                        tracing::error!("Client monitor tick failed: {}", e);
                    }
                }
            }
        }
    }

    fn headless_config(&self) -> Option<HeadlessConfig> {
        self.config.read().headless.clone()
    }

    async fn tick(&self) -> anyhow::Result<()> {
        // Reload config from disk so out-of-band changes (e.g. CLI scale) are picked up.
        // This is the same Arc the web UI reads, so both stay in sync.
        match Config::load_with_env(&self.config_path) {
            Ok(fresh) => *self.config.write() = fresh,
            Err(e) => tracing::warn!(error = %e, "Failed to reload config on tick, using cached"),
        }

        let headless_config = match self.headless_config() {
            Some(hc) if hc.client_count() > 0 => hc,
            _ => {
                let mut state_lock = self.state.write().await;
                state_lock.clear();
                return Ok(());
            }
        };

        // Check server liveness first
        let server_up = match self.spt_client.ping().await {
            Ok(ping) => ping.ok,
            Err(e) => {
                tracing::warn!("Server ping failed: {}", e);
                false
            }
        };

        // Get Fika headless status if server is up
        let headlesses = if server_up {
            match self.spt_client.headless_clients().await {
                Ok(response) => Some(response),
                Err(e) => {
                    tracing::warn!("Failed to fetch headless clients: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Update each client's state
        let mut states = Vec::new();
        for i in 1..=headless_config.client_count() {
            let state = self
                .check_client(i, server_up, headlesses.as_ref(), &headless_config)
                .await?;
            states.push(state);
        }

        // Merge updated state, preserving restart-owned fields
        {
            let mut state_lock = self.state.write().await;
            for new_state in &states {
                if let Some(existing) = state_lock.iter_mut().find(|s| s.index == new_state.index) {
                    // Update fields owned by check_client
                    existing.container_status = new_state.container_status.clone();
                    existing.fika_status = new_state.fika_status.clone();
                    existing.players = new_state.players.clone();
                    existing.cpu_percent = new_state.cpu_percent;
                    existing.memory_mb = new_state.memory_mb;
                    existing.health = new_state.health.clone();
                    // Reset failure counter when container is running and healthy
                    // (handles recovery after manual restart outside of quartermaster)
                    if matches!(
                        new_state.health,
                        ClientHealth::Healthy | ClientHealth::Degraded
                    ) {
                        existing.consecutive_failures = 0;
                    }
                    // NOTE: consecutive_failures is reset above when container recovers.
                    // first_seen is owned by exit watchers — do NOT overwrite it here.
                } else {
                    // New client, add it
                    state_lock.push(new_state.clone());
                }
            }
            // Remove clients that no longer exist
            state_lock.retain(|s| states.iter().any(|ns| ns.index == s.index));
        }

        // Cancel watchers for removed clients
        {
            let mut handles = self.watcher_handles.write().await;
            handles.retain(|idx, token| {
                let still_exists = states.iter().any(|s| s.index == *idx);
                if !still_exists {
                    token.cancel();
                }
                still_exists
            });
        }

        // Spawn exit watchers for running containers that don't have one yet
        for state in &states {
            if state.container_status == ContainerStatus::Running {
                let has_watcher = self.watcher_handles.read().await.contains_key(&state.index);
                if !has_watcher {
                    ClientSupervisor::spawn_exit_watcher(
                        self.container_mgr.clone(),
                        Arc::clone(&self.state),
                        Arc::clone(&self.watcher_handles),
                        state.index,
                        state.container_name.clone(),
                        headless_config.restart_policy.clone(),
                        headless_config.max_restart_attempts,
                        headless_config.restart_backoff_cap,
                        self.converging.clone(),
                        self.cancel_token.clone(),
                        Arc::clone(&self.config),
                        Arc::clone(&self.db),
                    )
                    .await;
                }
            }
        }

        // Proactive memory restart: restart idle clients whose memory exceeds the threshold.
        // Only restarts clients that are Ready (not in a raid) to avoid disrupting players.
        let threshold = headless_config.memory_restart_threshold;
        if threshold > 0 {
            let state_lock = self.state.read().await;
            for s in state_lock.iter() {
                if s.container_status != ContainerStatus::Running || s.restarting {
                    continue;
                }
                if s.fika_status != Some(crate::spt::headless::EHeadlessStatus::Ready) {
                    continue;
                }
                let Some(mem_mb) = s.memory_mb else {
                    continue;
                };
                if mem_mb < threshold as f64 {
                    continue;
                }
                let container_name = s.container_name.clone();
                let mgr = self.container_mgr.clone();
                tracing::warn!(
                    container = %container_name,
                    memory_mb = mem_mb as u64,
                    threshold_mb = threshold,
                    "Idle client exceeds memory threshold, restarting"
                );
                drop(state_lock);
                if let Err(e) = mgr.restart(&container_name).await {
                    tracing::error!(container = %container_name, err = %e, "Failed to restart for memory threshold");
                }
                // Only restart one per tick to avoid thundering herd
                break;
            }
        }

        Ok(())
    }

    async fn check_client(
        &self,
        index: u32,
        server_up: bool,
        headlesses: Option<&GetHeadlessesResponse>,
        headless_config: &HeadlessConfig,
    ) -> anyhow::Result<ClientState> {
        let container_name = crate::client::converge::client_container_name(index);

        // Get current state or create new one
        let existing = {
            let state_lock = self.state.read().await;
            state_lock.iter().find(|s| s.index == index).cloned()
        };

        let mut state = existing.unwrap_or_else(|| ClientState {
            index,
            container_name: container_name.clone(),
            container_status: ContainerStatus::Unknown,
            fika_status: None,
            players: Vec::new(),
            cpu_percent: None,
            memory_mb: None,
            restart_count: 0,
            last_restart: None,
            health: ClientHealth::Down,
            restarting: false,
            consecutive_failures: 0,
            first_seen: Utc::now(),
            manually_stopped: false,
            profile_id: None,
            prev_fika_status: None,
        });

        // Check container status
        let container_running = match self.container_mgr.is_running(&container_name).await {
            Ok(running) => {
                state.container_status = if running {
                    ContainerStatus::Running
                } else {
                    ContainerStatus::Stopped
                };
                running
            }
            Err(_) => {
                state.container_status = ContainerStatus::Unknown;
                false
            }
        };

        // Get PROFILE_ID from container inspect if running
        let profile_id = if container_running {
            match self.container_mgr.inspect(&container_name).await {
                Ok(inspect) => inspect.config.and_then(|c| c.env).and_then(|env| {
                    env.iter()
                        .find(|e| e.starts_with("PROFILE_ID="))
                        .and_then(|e| e.strip_prefix("PROFILE_ID="))
                        .map(String::from)
                }),
                Err(_) => None,
            }
        } else {
            None
        };

        // Store profile_id on state
        state.profile_id = profile_id.clone();

        // Save previous fika_status before updating
        state.prev_fika_status = state.fika_status.clone();

        // Match against Fika API
        if let (Some(pid), Some(headless_data)) = (profile_id.as_ref(), headlesses) {
            if let Some(client_info) = headless_data.headlesses.get(pid) {
                state.fika_status = Some(client_info.state.clone());
                state.players = client_info.players.clone();
            } else {
                state.fika_status = None;
                state.players.clear();
            }
        } else {
            state.fika_status = None;
            state.players.clear();
        }

        // Detect IN_RAID → READY transition and capture session stats
        if state.prev_fika_status == Some(EHeadlessStatus::InRaid)
            && state.fika_status == Some(EHeadlessStatus::Ready)
        {
            let container_name = state.container_name.clone();
            let db = self.db.clone();
            let index = state.index;
            let pid = state.profile_id.clone().unwrap_or_default();
            let mgr = self.container_mgr.clone();
            tokio::spawn(async move {
                if let Ok(log_text) = tail_container_logs(&mgr, &container_name, 100).await {
                    if let Some(stats) = crate::fika::stats::parse_session_stats(&log_text) {
                        let db = db.lock();
                        if let Err(e) = db.insert_session_stats(&stats, index, &pid) {
                            tracing::warn!(err = %e, client_index = index, "failed to store session stats");
                        } else {
                            tracing::info!(
                                client_index = index,
                                raid_time_seconds = stats.time_in_raid_seconds,
                                "captured session stats"
                            );
                        }
                    }
                }
            });
        }

        // Query memory usage for running containers
        if container_running {
            if let Some(bytes) = self
                .container_mgr
                .container_memory_bytes(&container_name)
                .await
            {
                state.memory_mb = Some(bytes as f64 / (1024.0 * 1024.0));
            }
        } else {
            state.memory_mb = None;
        }

        // Compute health
        state.health = compute_health(container_running, state.fika_status.clone(), server_up);

        // consecutive_failures is managed exclusively by exit watchers —
        // check_client only reads the value to detect GivenUp state.

        // Check if given up
        if state.consecutive_failures >= headless_config.max_restart_attempts {
            state.health = ClientHealth::GivenUp;
        }

        Ok(state)
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    async fn spawn_exit_watcher(
        container_mgr: ContainerManager,
        state: Arc<RwLock<Vec<ClientState>>>,
        watcher_handles: Arc<RwLock<HashMap<u32, CancellationToken>>>,
        index: u32,
        container_name: String,
        restart_policy: RestartPolicy,
        max_restart_attempts: u32,
        backoff_cap: u64,
        converging: Arc<AtomicBool>,
        cancel_token: CancellationToken,
        config: Arc<parking_lot::RwLock<Config>>,
        db: Arc<parking_lot::Mutex<crate::db::Database>>,
    ) {
        // Child token: cancelled when either the supervisor shuts down
        // (cancel_token) or this specific watcher is replaced/removed.
        let watcher_cancel = cancel_token.child_token();

        // Register this watcher, cancelling any previous one for the same index
        {
            let mut handles = watcher_handles.write().await;
            if let Some(old) = handles.insert(index, watcher_cancel.clone()) {
                old.cancel();
            }
        }

        let watcher_cancel_clone = watcher_cancel.clone();
        let container_mgr_clone = container_mgr.clone();
        let state_clone = Arc::clone(&state);
        let watcher_handles_clone = Arc::clone(&watcher_handles);

        tokio::spawn(async move {
            exit_watcher_loop(
                container_mgr_clone,
                state_clone,
                watcher_handles_clone,
                index,
                container_name,
                restart_policy,
                max_restart_attempts,
                backoff_cap,
                converging,
                watcher_cancel_clone,
                config,
                db,
            )
            .await;
        });
    }
}

#[allow(clippy::too_many_arguments)]
async fn exit_watcher_loop(
    container_mgr: ContainerManager,
    state: Arc<RwLock<Vec<ClientState>>>,
    watcher_handles: Arc<RwLock<HashMap<u32, CancellationToken>>>,
    index: u32,
    container_name: String,
    restart_policy: RestartPolicy,
    max_restart_attempts: u32,
    backoff_cap: u64,
    converging: Arc<AtomicBool>,
    cancel_token: CancellationToken,
    config: Arc<parking_lot::RwLock<Config>>,
    db: Arc<parking_lot::Mutex<crate::db::Database>>,
) {
    let mut retry_delay = Duration::from_secs(1);
    let max_retry_delay = Duration::from_secs(30);

    loop {
        // Watch the container for exit
        let mut stream = container_mgr.wait_container(&container_name);

        let wait_result = tokio::select! {
            _ = cancel_token.cancelled() => {
                tracing::debug!(container = %container_name, "Exit watcher cancelled");
                // Clean up our handle
                watcher_handles.write().await.remove(&index);
                return;
            }
            result = stream.next() => result,
        };

        // Classify the result into (exit_code, is_clean_exit) or a transient
        // stream error that warrants a retry.
        //
        // Bollard quirk: `wait_container` converts non-zero exit codes into
        // `Err(DockerContainerWaitError { code, error })`, so `Ok(response)`
        // only fires for exit code 0.
        let (exit_code, mut is_clean_exit) = match wait_result {
            Some(Ok(response)) => {
                // Clean exit (code 0) — but may still be OOM killed
                retry_delay = Duration::from_secs(1);
                let code = response.status_code;
                tracing::info!(
                    container = %container_name,
                    exit_code = code,
                    "Container exited cleanly"
                );
                (code, true)
            }
            Some(Err(bollard::errors::Error::DockerContainerWaitError { code, error })) => {
                // Non-zero exit — this is a crash, not a stream error
                retry_delay = Duration::from_secs(1);
                tracing::info!(
                    container = %container_name,
                    exit_code = code,
                    error = %error,
                    "Container crashed"
                );
                (code, false)
            }
            Some(Err(e)) => {
                // Transient stream error (socket drop, etc.) — retry with backoff
                tracing::warn!(
                    container = %container_name,
                    error = %e,
                    "Exit watcher stream error, retrying in {:?}",
                    retry_delay
                );
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(max_retry_delay);
                continue;
            }
            None => {
                // Stream ended without a result — unexpected
                tracing::warn!(
                    container = %container_name,
                    "Exit watcher stream ended unexpectedly"
                );
                watcher_handles.write().await.remove(&index);
                return;
            }
        };

        // Check for OOM kill: the kernel kills the game process but the
        // entrypoint exits cleanly (code 0). Without this check, OOM kills
        // look like normal restarts and never trigger backoff.
        if is_clean_exit && container_mgr.was_oom_killed(&container_name).await {
            is_clean_exit = false;
            tracing::warn!(
                container = %container_name,
                "Container was OOM killed (exit code 0 but OOMKilled=true)"
            );
        }

        // Update state with exit info
        let should_restart = {
            let mut state_lock = state.write().await;
            if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
                s.container_status = ContainerStatus::Stopped;
                s.health = ClientHealth::Down;

                if !is_clean_exit {
                    let in_grace_period =
                        Utc::now().signed_duration_since(s.first_seen).num_seconds() < 180;
                    if !in_grace_period {
                        s.consecutive_failures += 1;
                    }
                }

                if s.consecutive_failures >= max_restart_attempts {
                    s.health = ClientHealth::GivenUp;
                }

                restart_policy == RestartPolicy::Auto
                    && !s.manually_stopped
                    && s.health != ClientHealth::GivenUp
                    && s.consecutive_failures < max_restart_attempts
            } else {
                false
            }
        };

        if converging.load(Ordering::Relaxed) {
            tracing::debug!(
                container = %container_name,
                "Skipping exit-watcher restart (convergence in progress)"
            );
            // Don't exit the loop — converge will restart the container,
            // and we need to keep watching for its next exit.
            continue;
        }

        if !should_restart {
            let reason = {
                let state_lock = state.read().await;
                if let Some(s) = state_lock.iter().find(|s| s.index == index) {
                    if s.manually_stopped {
                        "manually stopped"
                    } else {
                        "policy or given up"
                    }
                } else {
                    "client removed"
                }
            };
            tracing::info!(
                container = %container_name,
                exit_code,
                "Not restarting ({reason})"
            );
            watcher_handles.write().await.remove(&index);
            return;
        }

        // Mark as restarting
        {
            let mut state_lock = state.write().await;
            if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
                s.restarting = true;
            }
        }

        // Get failure count for backoff calculation
        let failures = {
            let state_lock = state.read().await;
            state_lock
                .iter()
                .find(|s| s.index == index)
                .map(|s| s.consecutive_failures)
                .unwrap_or(0)
        };

        let _guard = RestartingGuard {
            state: state.clone(),
            index,
        };

        // Apply backoff for crash restarts
        if !is_clean_exit && failures > 0 {
            let delay = backoff_duration(failures, backoff_cap);
            tracing::info!(
                container = %container_name,
                delay_secs = delay.as_secs(),
                failures,
                "Backing off before restart"
            );
            tokio::time::sleep(delay).await;
        }

        // Start the already-stopped container (don't use restart() —
        // calling stop() on an exited container errors with 304)
        match container_mgr.start(&container_name).await {
            Ok(()) => {
                let mut state_lock = state.write().await;
                if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
                    s.restart_count += 1;
                    s.last_restart = Some(Utc::now());
                    s.first_seen = Utc::now();
                    s.restarting = false;
                    if is_clean_exit {
                        s.consecutive_failures = 0;
                    }
                }
                tracing::info!(container = %container_name, "Restarted successfully");

                // Re-apply Fika overlay config after a delay — Fika overwrites
                // the seed config with a full BepInEx config on first boot,
                // resetting our Force IP, Last Version, etc. to defaults.
                let config_clone = Arc::clone(&config);
                let db_clone = Arc::clone(&db);
                let reapply_index = index;
                tokio::spawn(reapply_fika_config_after_boot(
                    config_clone,
                    db_clone,
                    reapply_index,
                ));

                // Continue loop to watch again
            }
            Err(e) => {
                tracing::error!(
                    container = %container_name,
                    err = %e,
                    "Failed to restart"
                );
                watcher_handles.write().await.remove(&index);
                return;
            }
        }
    }
}

async fn tail_container_logs(
    mgr: &ContainerManager,
    container: &str,
    lines: usize,
) -> anyhow::Result<String> {
    let mut stream = mgr.log_stream(container, lines, false);
    let mut output = Vec::new();

    while let Some(log) = stream.next().await {
        match log? {
            bollard::container::LogOutput::StdOut { message } => {
                output.extend_from_slice(&message);
            }
            bollard::container::LogOutput::StdErr { message } => {
                output.extend_from_slice(&message);
            }
            _ => {}
        }
    }

    Ok(String::from_utf8_lossy(&output).to_string())
}

pub fn compute_health(
    container_running: bool,
    fika_status: Option<EHeadlessStatus>,
    server_up: bool,
) -> ClientHealth {
    if !container_running {
        return ClientHealth::Down;
    }

    if !server_up {
        return ClientHealth::Degraded;
    }

    match fika_status {
        Some(EHeadlessStatus::Ready) | Some(EHeadlessStatus::InRaid) => ClientHealth::Healthy,
        Some(EHeadlessStatus::Unknown(_)) => ClientHealth::Degraded,
        None => ClientHealth::Degraded,
    }
}

pub fn backoff_duration(failures: u32, cap: u64) -> Duration {
    let base = 5u64;
    let power = 2u64.saturating_pow(failures);
    let exponential = base.saturating_mul(power);
    let capped = exponential.min(cap);
    Duration::from_secs(capped)
}

/// Re-apply Fika overlay config values after a container boots.
///
/// Fika's BepInEx config binding regenerates com.fika.core.cfg on first boot,
/// overwriting our seed values with defaults. We wait 15 seconds for Fika to
/// finish generating the config, then re-apply Force IP, Last Version, etc.
#[allow(deprecated)]
async fn reapply_fika_config_after_boot(
    config: Arc<parking_lot::RwLock<Config>>,
    db: Arc<parking_lot::Mutex<crate::db::Database>>,
    index: u32,
) {
    tokio::time::sleep(Duration::from_secs(15)).await;
    let cfg = config.read();
    if let Some(ref hc) = cfg.headless {
        let fika_version = {
            let db_guard = db.lock();
            db_guard
                .get_mod_by_forge_id(crate::config::FIKA_CLIENT_FORGE_ID)
                .ok()
                .flatten()
                .map(|m| m.version)
        };
        if let Err(e) = crate::client::converge::reapply_fika_config(
            &hc.install_dir,
            index,
            hc.force_ip.as_deref(),
            hc.use_upnp,
            fika_version.as_deref(),
        ) {
            tracing::warn!(err = %e, "Failed to re-apply Fika overlay config after restart");
        } else {
            tracing::debug!("Re-applied Fika overlay config for client {index}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn health_healthy_when_running_and_ready() {
        assert_eq!(
            compute_health(true, Some(EHeadlessStatus::Ready), true),
            ClientHealth::Healthy
        );
    }

    #[test]
    fn health_healthy_when_running_and_in_raid() {
        assert_eq!(
            compute_health(true, Some(EHeadlessStatus::InRaid), true),
            ClientHealth::Healthy
        );
    }

    #[test]
    fn health_degraded_when_running_but_not_connected() {
        assert_eq!(compute_health(true, None, true), ClientHealth::Degraded);
    }

    #[test]
    fn health_degraded_when_server_down() {
        assert_eq!(
            compute_health(true, Some(EHeadlessStatus::Ready), false),
            ClientHealth::Degraded
        );
    }

    #[test]
    fn health_down_when_container_stopped() {
        assert_eq!(compute_health(false, None, true), ClientHealth::Down);
    }

    #[test]
    fn backoff_exponential() {
        assert_eq!(backoff_duration(0, 300), Duration::from_secs(5));
        assert_eq!(backoff_duration(1, 300), Duration::from_secs(10));
        assert_eq!(backoff_duration(2, 300), Duration::from_secs(20));
        assert_eq!(backoff_duration(3, 300), Duration::from_secs(40));
    }

    #[test]
    fn backoff_capped() {
        assert_eq!(backoff_duration(10, 300), Duration::from_secs(300));
        assert_eq!(backoff_duration(100, 300), Duration::from_secs(300));
    }

    #[test]
    fn backoff_custom_cap() {
        assert_eq!(backoff_duration(10, 60), Duration::from_secs(60));
    }

    #[test]
    fn manually_stopped_prevents_restart_decision() {
        // The restart decision logic: policy == Auto && !manually_stopped && health != GivenUp
        let policy = RestartPolicy::Auto;
        let manually_stopped = true;
        let health = ClientHealth::Down;
        let consecutive_failures = 0u32;
        let max_restart_attempts = 5u32;

        let should_restart = policy == RestartPolicy::Auto
            && !manually_stopped
            && health != ClientHealth::GivenUp
            && consecutive_failures < max_restart_attempts;

        assert!(!should_restart);
    }
}
