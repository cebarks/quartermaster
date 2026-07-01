use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::logging::{LogBroadcast, ReloadHandles};
use crate::spt::detect::SptInfo;
use crate::spt::game_data::GameData;
use crate::svm::SvmManager;
use crate::web::proxy_metrics::ProxyMetrics;
use crate::web::sse::ServerEvent;
use crate::web::tasks::TaskTracker;
use crate::web::update_cache::UpdateCache;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Arc<parking_lot::RwLock<Config>>,
    pub config_path: PathBuf,
    pub config_lock: parking_lot::Mutex<()>,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
    pub update_cache: UpdateCache,
    pub events: broadcast::Sender<ServerEvent>,
    pub log_broadcast: Arc<LogBroadcast>,
    pub reload_handles: Arc<ReloadHandles>,
    pub container_mgr: Option<Arc<ContainerManager>>,
    pub client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    pub converging: Arc<AtomicBool>,
    pub fika_installed: bool,
    pub modsync_installed: AtomicBool,
    pub svm: Option<Arc<parking_lot::RwLock<SvmManager>>>,
    pub svm_installed: AtomicBool,
    pub server_transition: Arc<Mutex<Option<String>>>,
    pub game_data: Arc<GameData>,
    pub proxy_metrics: ProxyMetrics,
    pub proxy_client: reqwest::Client,
}

impl AppState {
    /// Read-lock the config. Drop the guard before any `.await`.
    pub fn config(&self) -> parking_lot::RwLockReadGuard<'_, Config> {
        self.config.read()
    }

    /// Clone the full config (useful for passing into background tasks or sync closures).
    pub fn config_cloned(&self) -> Config {
        self.config.read().clone()
    }

    /// Get a handle to the config RwLock for passing into background tasks
    /// that need to update config after disk writes.
    pub fn config_handle(&self) -> Arc<parking_lot::RwLock<Config>> {
        Arc::clone(&self.config)
    }

    /// Reload config from disk into the in-memory RwLock.
    /// Call this after saving config to disk (while still holding config_lock).
    pub fn update_config_from_disk(&self) -> anyhow::Result<()> {
        let fresh = Config::load_with_env(&self.config_path)?;
        *self.config.write() = fresh;
        Ok(())
    }

    pub fn get_server_transition(&self) -> Option<String> {
        self.server_transition.lock().clone()
    }

    pub fn set_server_transition(&self, transition: Option<&str>) {
        *self.server_transition.lock() = transition.map(|s| s.to_string());
    }

    pub fn is_modsync_installed(&self) -> bool {
        self.modsync_installed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn is_svm_installed(&self) -> bool {
        self.svm_installed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn regenerate_modsync(&self) {
        if !self.is_modsync_installed() {
            return;
        }
        let db = self.db.clone();
        let spt_dir = self.spt_dir.clone();
        let config = self.config_cloned();
        let result = actix_web::web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db)
        })
        .await;
        if let Err(e) = result {
            tracing::warn!(err = %e, "failed to regenerate NarcoNet config");
        }
    }
}
