use std::collections::HashMap;
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
use crate::config::{HeadlessConfig, RestartPolicy};
use crate::container::ContainerManager;
use crate::spt::headless::{EHeadlessStatus, GetHeadlessesResponse};
use crate::spt::server::SptClient;

pub struct ClientSupervisor {
    container_mgr: ContainerManager,
    spt_client: SptClient,
    headless_config: HeadlessConfig,
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
        headless_config: HeadlessConfig,
        converging: Arc<AtomicBool>,
        cancel_token: CancellationToken,
    ) -> Self {
        let state = Arc::new(RwLock::new(Vec::new()));
        Self {
            container_mgr,
            spt_client,
            headless_config,
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
                    for (_, token) in handles.iter() {
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

    async fn tick(&self) -> anyhow::Result<()> {
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
        for i in 1..=self.headless_config.client_count() {
            let state = self.check_client(i, server_up, headlesses.as_ref()).await?;
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
                    // NOTE: consecutive_failures and first_seen are owned by
                    // exit watchers — do NOT overwrite them here.
                } else {
                    // New client, add it
                    state_lock.push(new_state.clone());
                }
            }
            // Remove clients that no longer exist
            state_lock.retain(|s| states.iter().any(|ns| ns.index == s.index));
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
                        self.headless_config.restart_policy.clone(),
                        self.headless_config.max_restart_attempts,
                        self.headless_config.restart_backoff_cap,
                        self.cancel_token.clone(),
                    )
                    .await;
                }
            }
        }

        Ok(())
    }

    async fn check_client(
        &self,
        index: u32,
        server_up: bool,
        headlesses: Option<&GetHeadlessesResponse>,
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

        // Compute health
        state.health = compute_health(container_running, state.fika_status.clone(), server_up);

        // consecutive_failures is managed exclusively by exit watchers —
        // check_client only reads the value to detect GivenUp state.

        // Check if given up
        if state.consecutive_failures > self.headless_config.max_restart_attempts {
            state.health = ClientHealth::GivenUp;
        }

        Ok(state)
    }

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
        cancel_token: CancellationToken,
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
                watcher_cancel_clone,
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
    cancel_token: CancellationToken,
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
        let (exit_code, is_clean_exit) = match wait_result {
            Some(Ok(response)) => {
                // Clean exit (code 0)
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

                if s.consecutive_failures > max_restart_attempts {
                    s.health = ClientHealth::GivenUp;
                }

                restart_policy == RestartPolicy::Auto
                    && s.health != ClientHealth::GivenUp
                    && s.consecutive_failures <= max_restart_attempts
            } else {
                false
            }
        };

        if !should_restart {
            tracing::info!(
                container = %container_name,
                exit_code,
                "Not restarting (policy={restart_policy}, or given up)"
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

        let _guard = scopeguard::guard((state.clone(), index), |(state, index)| {
            tokio::spawn(async move {
                let mut state_lock = state.write().await;
                if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
                    s.restarting = false;
                }
            });
        });

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
                    if is_clean_exit {
                        s.consecutive_failures = 0;
                    }
                }
                tracing::info!(container = %container_name, "Restarted successfully");
                // Continue loop to watch again
            }
            Err(e) => {
                tracing::error!(
                    container = %container_name,
                    error = %e,
                    "Failed to restart"
                );
                watcher_handles.write().await.remove(&index);
                return;
            }
        }
    }
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
}
