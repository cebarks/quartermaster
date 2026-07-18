pub mod converge;
pub mod supervisor;

use crate::spt::headless::EHeadlessStatus;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerStatus {
    Running,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientHealth {
    Healthy,
    Degraded,
    Down,
    GivenUp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientState {
    pub index: u32,
    pub container_name: String,
    pub container_status: ContainerStatus,
    pub fika_status: Option<EHeadlessStatus>,
    pub players: Vec<String>,
    pub cpu_percent: Option<f64>,
    pub memory_mb: Option<f64>,
    pub restart_count: u32,
    pub last_restart: Option<DateTime<Utc>>,
    pub health: ClientHealth,
    pub restarting: bool,
    pub consecutive_failures: u32,
    pub first_seen: DateTime<Utc>,
    pub manually_stopped: bool,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub profile_id: Option<String>,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub prev_fika_status: Option<EHeadlessStatus>,
}
