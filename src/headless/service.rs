use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::RwLock;

use crate::client::ClientState;
use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::fika::client::FikaClient;
use crate::forge::client::ForgeClient;
use crate::headless::OperationTracker;

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
}
