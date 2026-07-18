use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bollard::container::LogOutput;
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::client::{ClientHealth, ClientState};
use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::fika::client::FikaClient;
use crate::forge::client::ForgeClient;
use crate::headless::{HeadlessError, OperationTracker};
use crate::spt::headless::EHeadlessStatus;

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

#[derive(Debug, Clone)]
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
        }
    }

    pub fn client_states(&self) -> Arc<RwLock<Vec<ClientState>>> {
        Arc::clone(&self.client_states)
    }

    pub fn operations(&self) -> &OperationTracker {
        &self.operations
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

        let result = match action {
            LifecycleAction::Start => self.container_mgr.start(&container_name).await,
            LifecycleAction::Stop => self.container_mgr.stop(&container_name).await,
            LifecycleAction::Restart => self.container_mgr.restart(&container_name).await,
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

        let spt_dir = self.dirs.spt_server.clone();
        let new_name = name.trim().to_string();
        let fika_config_lock = Arc::clone(&self.fika_config_lock);

        tokio::task::spawn_blocking(move || {
            let _guard = fika_config_lock.lock();
            let path = crate::fika::config::fika_config_path(&spt_dir);

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
}
