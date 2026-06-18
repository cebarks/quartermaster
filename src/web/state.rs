use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::logging::LogBroadcast;
use crate::spt::detect::SptInfo;
use crate::web::tasks::TaskTracker;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub tasks: TaskTracker,
    pub log_broadcast: Arc<LogBroadcast>,
}
