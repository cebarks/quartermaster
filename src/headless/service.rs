use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bollard::container::LogOutput;
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{broadcast, RwLock};

use crate::client::{ClientHealth, ClientState};
use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::fika::client::FikaClient;
use crate::forge::client::ForgeClient;
use crate::headless::{HeadlessError, OperationTracker};
use crate::spt::headless::EHeadlessStatus;
use crate::web::sse::ServerEvent;

#[derive(Debug, Clone, Copy)]
pub enum LifecycleAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GracefulResult {
    Exited,
    Timeout,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub stream: String,
    pub message: String,
}

pub struct HeadlessService {
    pub(crate) container_mgr: ContainerManager,
    pub(crate) config: Arc<parking_lot::RwLock<Config>>,
    pub(crate) config_path: PathBuf,
    pub(crate) config_lock: Arc<Mutex<()>>,
    pub(crate) dirs: Arc<QumaDirs>,
    pub(crate) db: Arc<Mutex<Database>>,
    pub(crate) converging: Arc<AtomicBool>,
    pub(crate) client_states: Arc<RwLock<Vec<ClientState>>>,
    pub(crate) fika_client: Option<Arc<FikaClient>>,
    pub(crate) fika_config_lock: Arc<Mutex<()>>,
    pub(crate) forge: ForgeClient,
    pub(crate) operations: OperationTracker,
    pub(crate) events: broadcast::Sender<ServerEvent>,
}

impl HeadlessService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        container_mgr: ContainerManager,
        config: Arc<parking_lot::RwLock<Config>>,
        config_path: PathBuf,
        config_lock: Arc<Mutex<()>>,
        dirs: Arc<QumaDirs>,
        db: Arc<Mutex<Database>>,
        converging: Arc<AtomicBool>,
        client_states: Arc<RwLock<Vec<ClientState>>>,
        fika_client: Option<Arc<FikaClient>>,
        fika_config_lock: Arc<Mutex<()>>,
        forge: ForgeClient,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self {
            container_mgr,
            config,
            config_path,
            config_lock,
            dirs,
            db,
            converging,
            client_states,
            fika_client,
            fika_config_lock,
            forge,
            operations: OperationTracker::new(),
            events,
        }
    }

    pub fn client_states(&self) -> Arc<RwLock<Vec<ClientState>>> {
        Arc::clone(&self.client_states)
    }

    pub fn operations(&self) -> &OperationTracker {
        &self.operations
    }

    fn spt_client(&self) -> crate::spt::server::SptClient {
        let config = self.config.read();
        let (host, port) = crate::server_detect::resolve_server_addr(&config, &self.dirs);
        crate::spt::server::SptClient::new(&host, port)
            .expect("SptClient construction should not fail")
    }

    fn persist_config(&self, mutate: impl FnOnce(&mut Config)) -> Result<(), HeadlessError> {
        let _guard = self.config_lock.lock();
        let mut fresh = Config::load_with_env(&self.config_path)
            .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;
        mutate(&mut fresh);
        fresh
            .save(&self.config_path)
            .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;
        *self.config.write() = fresh;
        Ok(())
    }

    fn spawn_convergence(
        &self,
        headless_config: crate::config::HeadlessConfig,
    ) -> crate::headless::OperationId {
        let op_id = self.operations.start();
        let mgr = self.container_mgr.clone();
        let config = self.config.read().clone();
        let dirs = Arc::clone(&self.dirs);
        let spt_client = self.spt_client();
        let forge = self.forge.clone();
        let converging = Arc::clone(&self.converging);
        let db = Arc::clone(&self.db);
        let ops = self.operations.clone();
        let events = self.events.clone();

        tokio::spawn(async move {
            let ctx = crate::client::converge::ConvergeContext {
                container_mgr: &mgr,
                headless_config: &headless_config,
                config: &config,
                dirs: &dirs,
                spt_client: &spt_client,
                forge: &forge,
                converging: &converging,
                db: &db,
            };
            match crate::client::converge::converge(&ctx).await {
                Ok(()) => {
                    ops.complete(&op_id);
                    let _ = events.send(ServerEvent::HeadlessChanged);
                }
                Err(e) => {
                    ops.fail(&op_id, e.to_string());
                    let _ = events.send(ServerEvent::HeadlessChanged);
                }
            }
        });
        op_id
    }

    pub async fn status(&self) -> Vec<ClientState> {
        self.client_states.read().await.clone()
    }

    pub async fn client_lifecycle(
        &self,
        index: u32,
        action: LifecycleAction,
    ) -> Result<(), HeadlessError> {
        let container_name = {
            let states = self.client_states.read().await;
            states
                .iter()
                .find(|c| c.index == index)
                .map(|c| c.container_name.clone())
                .ok_or(HeadlessError::ClientNotFound(index))?
        };

        let guard = crate::headless::exclusions::lifecycle::acquire(&self.dirs, index)
            .map_err(HeadlessError::Internal)?;

        let result = match action {
            LifecycleAction::Start => {
                crate::headless::exclusions::lifecycle::start_locked(
                    &self.container_mgr,
                    &self.dirs,
                    &self.db,
                    index,
                    guard.clone(),
                )
                .await
            }
            LifecycleAction::Stop => self.container_mgr.stop(&container_name).await,
            LifecycleAction::Restart => {
                crate::headless::exclusions::lifecycle::restart_locked(
                    &self.container_mgr,
                    &self.dirs,
                    &self.db,
                    index,
                    guard.clone(),
                )
                .await
            }
        };

        result.map_err(|e| HeadlessError::ContainerError(e.to_string()))?;

        // Update client state
        let mut states = self.client_states.write().await;
        if let Some(client) = states.iter_mut().find(|c| c.index == index) {
            match action {
                LifecycleAction::Start | LifecycleAction::Restart => {
                    client.consecutive_failures = 0;
                    client.health = ClientHealth::Degraded;
                    client.manually_stopped = false;
                }
                LifecycleAction::Stop => {
                    client.manually_stopped = true;
                }
            }
        }

        Ok(())
    }

    pub async fn graceful_restart(&self, index: u32) -> Result<GracefulResult, HeadlessError> {
        let guard = crate::headless::exclusions::lifecycle::acquire(&self.dirs, index)
            .map_err(HeadlessError::Internal)?;
        let (profile_id, fika_status) = {
            let states = self.client_states.read().await;
            let client = states
                .iter()
                .find(|c| c.index == index)
                .ok_or(HeadlessError::ClientNotFound(index))?;
            (client.profile_id.clone(), client.fika_status.clone())
        };

        if fika_status == Some(EHeadlessStatus::InRaid) {
            return Err(HeadlessError::ClientInRaid {
                clients: vec![index],
            });
        }

        let profile_id = profile_id
            .filter(|p| !p.is_empty())
            .ok_or_else(|| HeadlessError::Internal(anyhow::anyhow!("No profile ID")))?;

        let fika_client = self
            .fika_client
            .as_ref()
            .ok_or(HeadlessError::NoFikaClient)?;

        fika_client
            .shutdown_headless(&profile_id)
            .await
            .map_err(|e| HeadlessError::FikaError(e.to_string()))?;

        // Poll for exit (2s interval, 30s timeout)
        let container_name = crate::client::converge::client_container_name(index);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if !self
                .container_mgr
                .is_running(&container_name)
                .await
                .unwrap_or(true)
            {
                // This endpoint historically delegated the restart to the Auto
                // exit watcher. Keep that policy/manual-stop decision, but own
                // the complete guarded sequence instead of racing that watcher.
                let auto = self
                    .config
                    .read()
                    .headless
                    .as_ref()
                    .is_some_and(|h| h.restart_policy == crate::config::RestartPolicy::Auto);
                let manually_stopped = self
                    .client_states
                    .read()
                    .await
                    .iter()
                    .find(|c| c.index == index)
                    .is_none_or(|c| c.manually_stopped);
                if !auto || manually_stopped {
                    return Ok(GracefulResult::Exited);
                }
                crate::headless::exclusions::lifecycle::start_locked(
                    &self.container_mgr,
                    &self.dirs,
                    &self.db,
                    index,
                    guard.clone(),
                )
                .await
                .map_err(HeadlessError::Internal)?;
                if let Some(client) = self
                    .client_states
                    .write()
                    .await
                    .iter_mut()
                    .find(|c| c.index == index)
                {
                    client.consecutive_failures = 0;
                    client.health = ClientHealth::Degraded;
                    client.manually_stopped = false;
                }
                return Ok(GracefulResult::Exited);
            }
        }

        Ok(GracefulResult::Timeout)
    }

    pub async fn rename(&self, index: u32, name: &str) -> Result<(), HeadlessError> {
        let profile_id = {
            let states = self.client_states.read().await;
            states
                .iter()
                .find(|c| c.index == index)
                .and_then(|c| c.profile_id.clone())
                .ok_or(HeadlessError::ClientNotFound(index))?
        };

        let dirs = Arc::clone(&self.dirs);
        let new_name = name.trim().to_string();
        let fika_config_lock = Arc::clone(&self.fika_config_lock);

        tokio::task::spawn_blocking(move || {
            let _guard = fika_config_lock.lock();
            let path = crate::fika::config::fika_config_path(&dirs)
                .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;

            let config = crate::fika::config::read_fika_config(&path)
                .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;
            let mut aliases = config.headless.profiles.aliases;

            if new_name.is_empty() {
                aliases.remove(&profile_id);
            } else {
                aliases.insert(profile_id.clone(), new_name);
            }

            let cst = crate::fika::config::read_fika_cst(&path)
                .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;
            let root = cst.object_value_or_set();
            if let Some(headless) = root.object_value("headless") {
                if let Some(profiles) = headless.object_value("profiles") {
                    let alias_entries: Vec<(String, jsonc_parser::cst::CstInputValue)> = aliases
                        .into_iter()
                        .map(|(k, v)| (k, jsonc_parser::cst::CstInputValue::String(v)))
                        .collect();
                    match profiles.get("aliases") {
                        Some(prop) => {
                            prop.set_value(jsonc_parser::cst::CstInputValue::Object(alias_entries))
                        }
                        None => {
                            profiles.append(
                                "aliases",
                                jsonc_parser::cst::CstInputValue::Object(alias_entries),
                            );
                        }
                    }
                }
            }
            crate::fika::config::write_fika_cst(&cst, &path)
                .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;

            Ok::<(), HeadlessError>(())
        })
        .await
        .map_err(|e| HeadlessError::Internal(e.into()))?
    }

    pub async fn set_image(&self, index: u32, image: Option<String>) -> Result<(), HeadlessError> {
        let _guard = self.config_lock.lock();
        let mut config = Config::load_with_env(&self.config_path)
            .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;

        let headless = config
            .headless
            .as_mut()
            .ok_or(HeadlessError::NotConfigured)?;

        let client_index = (index as usize)
            .checked_sub(1)
            .ok_or(HeadlessError::ClientNotFound(index))?;
        let client_def = headless
            .clients
            .get_mut(client_index)
            .ok_or(HeadlessError::ClientNotFound(index))?;

        client_def.image = match image {
            Some(img) if !img.is_empty() && img != headless.image => Some(img),
            _ => None,
        };

        config
            .save(&self.config_path)
            .map_err(|e| HeadlessError::ConfigError(e.to_string()))?;

        *self.config.write() = config;

        Ok(())
    }

    pub async fn start_raid(
        &self,
        index: u32,
        location_id: &str,
        time: i32,
        use_event: bool,
    ) -> Result<(), HeadlessError> {
        let profile_id = {
            let states = self.client_states.read().await;
            let client = states
                .iter()
                .find(|c| c.index == index)
                .ok_or(HeadlessError::ClientNotFound(index))?;

            if client.fika_status != Some(EHeadlessStatus::Ready) {
                return Err(HeadlessError::Internal(anyhow::anyhow!(
                    "Client {index} is not READY"
                )));
            }

            client
                .profile_id
                .clone()
                .ok_or_else(|| HeadlessError::Internal(anyhow::anyhow!("No profile ID")))?
        };

        let fika_client = self
            .fika_client
            .as_ref()
            .ok_or(HeadlessError::NoFikaClient)?;

        let req = crate::fika::client::StartHeadlessRaidRequest {
            headless_session_id: profile_id,
            location_id: location_id.to_string(),
            time,
            time_and_weather_settings: None,
            use_event,
            side: 0,
            spawn_place: 0,
            metabolism_disabled: false,
            bot_settings: None,
            waves_settings: None,
            custom_raid_settings: None,
        };

        let resp = fika_client
            .start_headless_raid(&req)
            .await
            .map_err(|e| HeadlessError::FikaError(e.to_string()))?;

        if let Some(err) = resp.error {
            return Err(HeadlessError::FikaError(err));
        }

        Ok(())
    }

    pub fn logs(
        &self,
        index: u32,
        tail: usize,
        follow: bool,
    ) -> impl futures_util::Stream<Item = LogLine> {
        let container_name = crate::client::converge::client_container_name(index);
        self.container_mgr
            .log_stream(&container_name, tail, follow)
            .filter_map(|result| async {
                match result {
                    Ok(LogOutput::StdOut { message }) => Some(LogLine {
                        stream: "stdout".into(),
                        message: String::from_utf8_lossy(&message).into_owned(),
                    }),
                    Ok(LogOutput::StdErr { message }) => Some(LogLine {
                        stream: "stderr".into(),
                        message: String::from_utf8_lossy(&message).into_owned(),
                    }),
                    _ => None,
                }
            })
    }

    pub async fn scale(
        &self,
        target: u32,
        force: bool,
    ) -> Result<crate::headless::OperationId, HeadlessError> {
        if target > crate::config::MAX_HEADLESS_CLIENTS {
            return Err(HeadlessError::MaxClientsReached);
        }

        let mut headless_config = self
            .config
            .read()
            .headless
            .clone()
            .ok_or(HeadlessError::NotConfigured)?;

        let current = headless_config.client_count();
        let current_state_count = self.client_states.read().await.len() as u32;

        if target < current_state_count && !force {
            let in_raid_clients: Vec<u32> = self
                .client_states
                .read()
                .await
                .iter()
                .filter(|c| matches!(c.fika_status, Some(EHeadlessStatus::InRaid)))
                .filter(|c| c.index >= target)
                .map(|c| c.index)
                .collect();

            if !in_raid_clients.is_empty() {
                return Err(HeadlessError::ClientInRaid {
                    clients: in_raid_clients,
                });
            }
        }

        if target > current {
            for _ in 0..(target - current) {
                headless_config
                    .clients
                    .push(crate::config::HeadlessClientDef::default());
            }
        } else if target < current {
            headless_config.clients.truncate(target as usize);
        }

        let updated_config = headless_config.clone();
        self.persist_config(|cfg| {
            if let Some(ref mut headless) = cfg.headless {
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
        })?;

        Ok(self.spawn_convergence(updated_config))
    }

    pub async fn create(&self) -> Result<crate::headless::OperationId, HeadlessError> {
        let mut headless_config = self
            .config
            .read()
            .headless
            .clone()
            .ok_or(HeadlessError::NotConfigured)?;

        if headless_config.client_count() >= crate::config::MAX_HEADLESS_CLIENTS {
            return Err(HeadlessError::MaxClientsReached);
        }

        headless_config
            .clients
            .push(crate::config::HeadlessClientDef::default());

        let updated_config = headless_config.clone();
        self.persist_config(|cfg| {
            if let Some(ref mut headless) = cfg.headless {
                headless
                    .clients
                    .push(crate::config::HeadlessClientDef::default());
            }
        })?;

        Ok(self.spawn_convergence(updated_config))
    }

    pub async fn delete(
        &self,
        index: u32,
        force: bool,
    ) -> Result<crate::headless::OperationId, HeadlessError> {
        let headless_config = self
            .config
            .read()
            .headless
            .clone()
            .ok_or(HeadlessError::NotConfigured)?;

        if index == 0 || index > headless_config.client_count() {
            return Err(HeadlessError::ClientNotFound(index));
        }

        if !force {
            let states = self.client_states.read().await;
            if let Some(client) = states.iter().find(|c| c.index == index) {
                if matches!(client.fika_status, Some(EHeadlessStatus::InRaid))
                    && !client.players.is_empty()
                {
                    return Err(HeadlessError::ClientInRaid {
                        clients: vec![index],
                    });
                }
            }
        }

        let op_id = self.operations.start();
        let mgr = self.container_mgr.clone();
        let config = self.config.read().clone();
        let config_path = self.config_path.clone();
        let config_lock = Arc::clone(&self.config_lock);
        let config_handle = Arc::clone(&self.config);
        let dirs = Arc::clone(&self.dirs);
        let spt_client = self.spt_client();
        let forge = self.forge.clone();
        let converging = Arc::clone(&self.converging);
        let db = Arc::clone(&self.db);
        let ops = self.operations.clone();
        let events = self.events.clone();

        let mut updated_config = headless_config;
        updated_config.clients.remove((index - 1) as usize);

        tokio::spawn(async move {
            let guards = match crate::headless::exclusions::lifecycle::acquire_all(&dirs) {
                Ok(guards) => guards,
                Err(e) => {
                    ops.fail(&op_id, format!("Headless lifecycle is busy or unsafe: {e}"));
                    return;
                }
            };
            if let Err(e) =
                crate::client::converge::remove_all_managed_containers(&mgr, &dirs, &guards).await
            {
                ops.fail(&op_id, format!("Failed to remove containers: {e}"));
                let _ = events.send(ServerEvent::HeadlessChanged);
                return;
            }

            // Keep the configured selection available for retry if teardown fails.
            if let Err(e) = crate::headless::exclusions::destroy_private_overlay(
                &dirs,
                index,
                &guards[(index - 1) as usize],
            ) {
                ops.fail(&op_id, format!("Failed to retire selected overlay: {e}"));
                let _ = events.send(ServerEvent::HeadlessChanged);
                return;
            }

            {
                let _guard = config_lock.lock();
                match Config::load_with_env(&config_path) {
                    Ok(mut fresh_config) => {
                        if let Some(ref mut headless) = fresh_config.headless {
                            if (index as usize) <= headless.clients.len() && index > 0 {
                                headless.clients.remove((index - 1) as usize);
                            }
                        }
                        if let Err(e) = fresh_config.save(&config_path) {
                            ops.fail(&op_id, format!("Failed to save config: {e}"));
                            let _ = events.send(ServerEvent::HeadlessChanged);
                            return;
                        }
                        *config_handle.write() = fresh_config;
                    }
                    Err(e) => {
                        ops.fail(&op_id, format!("Failed to reload config: {e}"));
                        let _ = events.send(ServerEvent::HeadlessChanged);
                        return;
                    }
                }
            }

            drop(guards);

            let ctx = crate::client::converge::ConvergeContext {
                container_mgr: &mgr,
                headless_config: &updated_config,
                config: &config,
                dirs: &dirs,
                spt_client: &spt_client,
                forge: &forge,
                converging: &converging,
                db: &db,
            };
            match crate::client::converge::converge(&ctx).await {
                Ok(()) => {
                    ops.complete(&op_id);
                    let _ = events.send(ServerEvent::HeadlessChanged);
                }
                Err(e) => {
                    ops.fail(&op_id, e.to_string());
                    let _ = events.send(ServerEvent::HeadlessChanged);
                }
            }
        });

        Ok(op_id)
    }

    pub async fn rebuild(
        &self,
        force: bool,
    ) -> Result<crate::headless::OperationId, HeadlessError> {
        let headless_config = self
            .config
            .read()
            .headless
            .clone()
            .ok_or(HeadlessError::NotConfigured)?;

        if headless_config.client_count() == 0 {
            return Err(HeadlessError::NotConfigured);
        }

        if !force {
            let in_raid_clients: Vec<u32> = self
                .client_states
                .read()
                .await
                .iter()
                .filter(|c| matches!(c.fika_status, Some(EHeadlessStatus::InRaid)))
                .map(|c| c.index)
                .collect();

            if !in_raid_clients.is_empty() {
                return Err(HeadlessError::ClientInRaid {
                    clients: in_raid_clients,
                });
            }
        }

        let op_id = self.operations.start();
        let mgr = self.container_mgr.clone();
        let config = self.config.read().clone();
        let dirs = Arc::clone(&self.dirs);
        let spt_client = self.spt_client();
        let forge = self.forge.clone();
        let converging = Arc::clone(&self.converging);
        let db = Arc::clone(&self.db);
        let ops = self.operations.clone();
        let events = self.events.clone();

        tokio::spawn(async move {
            if converging
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::Acquire,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_err()
            {
                ops.fail(&op_id, "Convergence already in progress".into());
                let _ = events.send(ServerEvent::HeadlessChanged);
                return;
            }

            let guards = match crate::headless::exclusions::lifecycle::acquire_all(&dirs) {
                Ok(guards) => guards,
                Err(e) => {
                    converging.store(false, std::sync::atomic::Ordering::Release);
                    ops.fail(&op_id, format!("Headless lifecycle is busy or unsafe: {e}"));
                    return;
                }
            };
            if let Err(e) =
                crate::client::converge::remove_all_managed_containers(&mgr, &dirs, &guards).await
            {
                converging.store(false, std::sync::atomic::Ordering::Release);
                ops.fail(&op_id, format!("Failed to remove containers: {e}"));
                let _ = events.send(ServerEvent::HeadlessChanged);
                return;
            }

            for (offset, guard) in guards.iter().enumerate() {
                if let Err(e) = crate::headless::exclusions::destroy_private_overlay(
                    &dirs,
                    offset as u32 + 1,
                    guard,
                ) {
                    converging.store(false, std::sync::atomic::Ordering::Release);
                    ops.fail(&op_id, format!("Failed to retire rebuilt overlay: {e}"));
                    let _ = events.send(ServerEvent::HeadlessChanged);
                    return;
                }
            }
            drop(guards);

            converging.store(false, std::sync::atomic::Ordering::Release);

            let ctx = crate::client::converge::ConvergeContext {
                container_mgr: &mgr,
                headless_config: &headless_config,
                config: &config,
                dirs: &dirs,
                spt_client: &spt_client,
                forge: &forge,
                converging: &converging,
                db: &db,
            };
            match crate::client::converge::converge(&ctx).await {
                Ok(()) => {
                    ops.complete(&op_id);
                    let _ = events.send(ServerEvent::HeadlessChanged);
                }
                Err(e) => {
                    ops.fail(&op_id, e.to_string());
                    let _ = events.send(ServerEvent::HeadlessChanged);
                }
            }
        });

        Ok(op_id)
    }

    pub async fn converge(&self) -> Result<crate::headless::OperationId, HeadlessError> {
        let headless_config = self
            .config
            .read()
            .headless
            .clone()
            .ok_or(HeadlessError::NotConfigured)?;

        Ok(self.spawn_convergence(headless_config))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn destructive_service_paths_retire_only_destroyed_generations_before_reuse() {
        for case in ["generation-delete", "generation-rebuild"] {
            crate::headless::exclusions::test_support::run(
                "headless::service::lifecycle_tests::destructive_service_paths_retire_only_destroyed_generations_before_reuse",
                case,
                |f| async move {
                    use std::os::unix::fs::MetadataExt;
                    crate::headless::exclusions::test_support::seed_generation(&f, &[1, 2]);
                    let peer_journal = f.dirs.root.join("headless-policy/client-2/journal.json");
                    let peer_before = std::fs::read(&peer_journal).unwrap();
                    let lock = f.dirs.root.join("headless-policy/client-1.lock");
                    let lock_inode = std::fs::metadata(&lock).unwrap().ino();
                    let peer_lock = f.dirs.root.join("headless-policy/client-2.lock");
                    let peer_inode = std::fs::metadata(&peer_lock).unwrap().ino();
                    let hc = crate::config::HeadlessConfig {
                        clients: vec![Default::default(), Default::default()],
                        ..Default::default()
                    };
                    let config = Config { headless: Some(hc), ..Default::default() };
                    config.save(&f.dirs.config_path()).unwrap();
                    let (events, _) = broadcast::channel(8);
                    let svc = HeadlessService::new(
                        f.manager.clone(), Arc::new(parking_lot::RwLock::new(config)),
                        f.dirs.config_path(), Arc::new(Mutex::new(())), Arc::new(f.dirs.clone()),
                        f.db.clone(), Default::default(), Default::default(), None,
                        Arc::new(Mutex::new(())), ForgeClient::new().unwrap(), events,
                    );
                    let operation = if std::env::var("QUMA_TASK2_CASE").unwrap() == "generation-delete" {
                        svc.delete(1, false).await.unwrap()
                    } else {
                        svc.rebuild(false).await.unwrap()
                    };
                    tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        loop {
                            match svc.operations.poll(operation.0) {
                                Some(crate::headless::operations::OperationStatus::Failed { error }) => {
                                    assert!(error.contains("Podman 3.0"), "unexpected teardown failure: {error}");
                                    break;
                                }
                                Some(crate::headless::operations::OperationStatus::Completed) => panic!("fixture must bound external convergence"),
                                _ => tokio::task::yield_now().await,
                            }
                        }
                    }).await.unwrap();
                    assert!(!f.dirs.headless_overlay(1).exists());
                    if std::env::var("QUMA_TASK2_CASE").unwrap() == "generation-delete" {
                        assert!(f.dirs.headless_upper(2).exists());
                        assert_eq!(std::fs::read(peer_journal).unwrap(), peer_before);
                    } else {
                        assert!(!f.dirs.headless_overlay(2).exists());
                    }
                    assert_eq!(std::fs::metadata(peer_lock).unwrap().ino(), peer_inode);
                    assert_eq!(std::fs::metadata(lock).unwrap().ino(), lock_inode);
                    for client in [1, 2] {
                        let quarantine = f.dirs.root.join(format!("headless-policy/client-{client}/quarantine/BepInEx/plugins/Excluded.dll"));
                        let backup = std::fs::read_dir(quarantine).unwrap().next().unwrap().unwrap().path();
                        assert_eq!(std::fs::read(backup).unwrap(), b"published");
                    }
                    std::fs::create_dir_all(f.dirs.headless_upper(1)).unwrap();
                    std::fs::create_dir_all(f.dirs.headless_work(1)).unwrap();
                    let guard = crate::headless::exclusions::lifecycle::acquire(&f.dirs, 1).unwrap();
                    crate::headless::exclusions::lifecycle::start_locked(&f.manager, &f.dirs, &f.db, 1, guard).await.unwrap();
                },
            );
        }
    }

    #[test]
    fn graceful_restart_prepares_and_preserves_raid_and_manual_stop_checks() {
        for case in [
            "graceful-valid",
            "graceful-invalid",
            "graceful-raid",
            "graceful-manual",
        ] {
            crate::headless::exclusions::test_support::run("headless::service::lifecycle_tests::graceful_restart_prepares_and_preserves_raid_and_manual_stop_checks",case,|f| async move {
                let case=std::env::var("QUMA_TASK2_CASE").unwrap();
                let server=wiremock::MockServer::start().await;
                wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/fika/api/restartheadless")).respond_with(wiremock::ResponseTemplate::new(200)).expect(if case=="graceful-raid" {0}else{1}).mount(&server).await;
                if case=="graceful-invalid" {f.db.lock().conn().execute("DROP TABLE mod_groups",[]).unwrap();}
                let cfg=Config{headless:Some(crate::config::HeadlessConfig::default()),..Default::default()};
                let states=Arc::new(RwLock::new(vec![crate::client::ClientState{index:1,container_name:"fika-headless-1".into(),container_status:crate::client::ContainerStatus::Running,fika_status:Some(if case=="graceful-raid"{EHeadlessStatus::InRaid}else{EHeadlessStatus::Ready}),players:vec![],cpu_percent:None,memory_mb:None,restart_count:0,last_restart:None,health:ClientHealth::Healthy,restarting:false,consecutive_failures:0,first_seen:chrono::Utc::now(),manually_stopped:case=="graceful-manual",profile_id:Some("profile".into()),prev_fika_status:None}]));
                let (events,_)=broadcast::channel(8);
                let fika=Arc::new(FikaClient::new(&server.uri(),"fixture".into()).unwrap());
                let svc=HeadlessService::new(f.manager,Arc::new(parking_lot::RwLock::new(cfg)),f.dirs.config_path(),Arc::new(Mutex::new(())),Arc::new(f.dirs),f.db,Default::default(),states.clone(),Some(fika),Arc::new(Mutex::new(())),ForgeClient::new().unwrap(),events);
                let result=svc.graceful_restart(1).await;
                if case=="graceful-raid" {assert!(matches!(result,Err(HeadlessError::ClientInRaid{..})));}
                else if case=="graceful-invalid" {assert!(result.is_err());}
                else {assert!(matches!(result,Ok(GracefulResult::Exited)));}
                if case=="graceful-manual" {assert!(states.read().await[0].manually_stopped);}
            });
        }
    }

    #[test]
    fn busy_preparation_blocks_scale_delete_rebuild_and_preserves_stop_intent() {
        for case in ["busy-scale", "busy-delete", "busy-rebuild", "busy-stop"] {
            crate::headless::exclusions::test_support::run("headless::service::lifecycle_tests::busy_preparation_blocks_scale_delete_rebuild_and_preserves_stop_intent",case,|f| async move {
                let case=std::env::var("QUMA_TASK2_CASE").unwrap();
                let hc=crate::config::HeadlessConfig{clients:vec![Default::default()],..Default::default()};
                let cfg=Config{headless:Some(hc),..Default::default()};cfg.save(&f.dirs.config_path()).unwrap();
                let sentinel=f.dirs.headless_upper(1).join("preserve");std::fs::write(&sentinel,b"unchanged").unwrap();
                let states=Arc::new(RwLock::new(vec![crate::client::ClientState{index:1,container_name:"fika-headless-1".into(),container_status:crate::client::ContainerStatus::Stopped,fika_status:None,players:vec![],cpu_percent:None,memory_mb:None,restart_count:0,last_restart:None,health:ClientHealth::Down,restarting:false,consecutive_failures:0,first_seen:chrono::Utc::now(),manually_stopped:true,profile_id:Some("profile".into()),prev_fika_status:None}]));
                let (events,_)=broadcast::channel(8);
                let svc=HeadlessService::new(f.manager,Arc::new(parking_lot::RwLock::new(cfg)),f.dirs.config_path(),Arc::new(Mutex::new(())),Arc::new(f.dirs.clone()),f.db,Default::default(),states.clone(),None,Arc::new(Mutex::new(())),ForgeClient::new().unwrap(),events);
                // Multi-client acquisition must roll back client 1 when client 2
                // is busy, before any destructive API or upper-directory change.
                let index=if case=="busy-stop" {1}else{2};
                let held=crate::headless::exclusions::PolicyOperationGuard::acquire(&f.dirs,index).unwrap();
                if case=="busy-stop" {
                    assert!(svc.client_lifecycle(1,LifecycleAction::Stop).await.is_err());
                    assert!(states.read().await[0].manually_stopped);
                }else{
                    let op=match case.as_str(){"busy-scale"=>svc.scale(0,false).await.unwrap(),"busy-delete"=>svc.delete(1,false).await.unwrap(),_=>svc.rebuild(false).await.unwrap()};
                    tokio::time::timeout(std::time::Duration::from_secs(5),async {loop {match svc.operations.poll(op.0){Some(crate::headless::operations::OperationStatus::Failed{error})=>{assert!(error.contains("busy"));break},Some(crate::headless::operations::OperationStatus::Completed)=>panic!("destructive operation bypassed busy guard"),_=>tokio::task::yield_now().await}}}).await.unwrap();
                    let first=crate::headless::exclusions::PolicyOperationGuard::acquire(&f.dirs,1).unwrap();drop(first);
                }
                assert_eq!(std::fs::read(sentinel).unwrap(),b"unchanged");drop(held);
            });
        }
    }
}
