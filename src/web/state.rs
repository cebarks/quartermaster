use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::logging::LogBroadcast;
use crate::spt::detect::SptInfo;
use crate::spt::game_data::GameData;
use crate::web::proxy_metrics::ProxyMetrics;
use crate::web::sse::ServerEvent;
use crate::web::tasks::TaskTracker;
use crate::web::update_cache::UpdateCache;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub config_path: PathBuf,
    pub config_lock: parking_lot::Mutex<()>,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
    pub update_cache: UpdateCache,
    pub events: broadcast::Sender<ServerEvent>,
    pub log_broadcast: Arc<LogBroadcast>,
    pub container_mgr: Option<Arc<ContainerManager>>,
    pub client_states: Option<Arc<tokio::sync::RwLock<Vec<crate::client::ClientState>>>>,
    pub converging: Arc<AtomicBool>,
    pub fika_installed: bool,
    pub modsync_installed: AtomicBool,
    pub server_transition: Arc<Mutex<Option<String>>>,
    pub game_data: Arc<GameData>,
    pub proxy_metrics: ProxyMetrics,
    pub proxy_client: reqwest::Client,
}

impl AppState {
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
}
