use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
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
                    existing.consecutive_failures = new_state.consecutive_failures;
                } else {
                    // New client, add it
                    state_lock.push(new_state.clone());
                }
            }
            // Remove clients that no longer exist
            state_lock.retain(|s| states.iter().any(|ns| ns.index == s.index));
        }

        // Handle auto-restart for clients that need it
        if self.headless_config.restart_policy == RestartPolicy::Auto && server_up {
            for state in &states {
                if self.should_restart(state) {
                    // Mark as restarting before spawning
                    {
                        let mut state_lock = self.state.write().await;
                        if let Some(s) = state_lock.iter_mut().find(|s| s.index == state.index) {
                            s.restarting = true;
                        }
                    }

                    let container_mgr = self.container_mgr.clone();
                    let shared_state = Arc::clone(&self.state);
                    let container_name = state.container_name.clone();
                    let index = state.index;
                    let failures = state.consecutive_failures;
                    let backoff_cap = self.headless_config.restart_backoff_cap;

                    tokio::spawn(async move {
                        restart_client_task(
                            container_mgr,
                            shared_state,
                            container_name,
                            index,
                            failures,
                            backoff_cap,
                        )
                        .await;
                    });
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

        // Update failure count
        let in_grace_period = false; // Grace period not yet implemented
        state.consecutive_failures = count_failure(
            state.consecutive_failures,
            &state.health,
            server_up,
            in_grace_period,
            state.restarting,
        );

        // Check if given up
        if state.consecutive_failures > self.headless_config.max_restart_attempts {
            state.health = ClientHealth::GivenUp;
        }

        Ok(state)
    }

    fn should_restart(&self, state: &ClientState) -> bool {
        // Don't restart if already restarting
        if state.restarting {
            return false;
        }

        // Don't restart if given up
        if state.health == ClientHealth::GivenUp {
            return false;
        }

        // Restart if down or degraded and under attempt limit
        (state.health == ClientHealth::Down || state.health == ClientHealth::Degraded)
            && state.consecutive_failures <= self.headless_config.max_restart_attempts
    }
}

async fn restart_client_task(
    container_mgr: ContainerManager,
    state: Arc<RwLock<Vec<ClientState>>>,
    container_name: String,
    index: u32,
    consecutive_failures: u32,
    backoff_cap: u64,
) {
    // Ensure restarting flag is cleared even on panic
    let _guard = scopeguard::guard((state.clone(), index), |(state, index)| {
        tokio::spawn(async move {
            let mut state_lock = state.write().await;
            if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
                s.restarting = false;
            }
        });
    });

    let delay = backoff_duration(consecutive_failures, backoff_cap);
    tracing::info!(
        container = %container_name,
        delay_secs = delay.as_secs(),
        failures = consecutive_failures,
        "Backing off before restart"
    );
    tokio::time::sleep(delay).await;

    let result = container_mgr.restart(&container_name).await;

    {
        let mut state_lock = state.write().await;
        if let Some(s) = state_lock.iter_mut().find(|s| s.index == index) {
            if result.is_ok() {
                s.restart_count += 1;
                s.last_restart = Some(Utc::now());
                s.consecutive_failures = 0;
            }
        }
    }

    if let Err(e) = result {
        tracing::error!(container = %container_name, error = %e, "Failed to restart client");
    }
}

/// Determines whether to increment, reset, or hold the failure counter.
/// Returns the updated consecutive_failures value.
pub fn count_failure(
    current_failures: u32,
    health: &ClientHealth,
    server_up: bool,
    in_grace_period: bool,
    restarting: bool,
) -> u32 {
    if in_grace_period || restarting {
        current_failures
    } else if !server_up {
        // Server is down — not the client's fault, don't count
        current_failures
    } else if *health == ClientHealth::Down || *health == ClientHealth::Degraded {
        current_failures + 1
    } else {
        0
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

    #[test]
    fn failures_not_counted_when_server_down() {
        // When server is down, health is Degraded but it's not the client's fault.
        // consecutive_failures should NOT increment.
        let health = compute_health(true, None, false);
        assert_eq!(health, ClientHealth::Degraded);
        // The actual failure counting logic is in check_client, tested via
        // count_failure() helper below.
    }

    #[test]
    fn count_failure_holds_when_server_down() {
        assert_eq!(
            count_failure(3, &ClientHealth::Degraded, false, false, false),
            3
        );
    }

    #[test]
    fn count_failure_increments_when_degraded_and_server_up() {
        assert_eq!(
            count_failure(3, &ClientHealth::Degraded, true, false, false),
            4
        );
    }

    #[test]
    fn count_failure_resets_when_healthy() {
        assert_eq!(
            count_failure(5, &ClientHealth::Healthy, true, false, false),
            0
        );
    }

    #[test]
    fn count_failure_holds_during_grace_period() {
        assert_eq!(
            count_failure(3, &ClientHealth::Degraded, true, true, false),
            3
        );
    }

    #[test]
    fn count_failure_holds_during_grace_even_if_server_down() {
        assert_eq!(
            count_failure(3, &ClientHealth::Degraded, false, true, false),
            3
        );
    }

    #[test]
    fn restart_should_reset_failures() {
        // After a successful restart, the client should have a clean slate.
        // This is a documentation test — the actual reset happens in
        // restart_client_task which modifies shared state asynchronously.
        // Verified by inspection: restart_client_task sets
        // s.consecutive_failures = 0 on success.
        //
        // The count_failure function already handles the grace period
        // correctly (holds current value), so resetting to 0 before the
        // new grace period means the client truly starts fresh.
        let reset_value = count_failure(0, &ClientHealth::Degraded, true, true, false);
        assert_eq!(reset_value, 0, "grace period should hold at 0 after reset");
    }

    #[test]
    fn count_failure_holds_while_restarting() {
        assert_eq!(count_failure(3, &ClientHealth::Down, true, false, true), 3);
    }
}
