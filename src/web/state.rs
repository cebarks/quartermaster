use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::SptInfo;

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub forge: ForgeClient,
    pub config: Config,
    pub config_path: PathBuf,
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
}
