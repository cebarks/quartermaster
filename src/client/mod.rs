pub mod converge;
pub mod supervisor;

use crate::spt::headless::EHeadlessStatus;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerStatus {
    Running,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientHealth {
    Healthy,
    Degraded,
    Down,
    GivenUp,
}

#[derive(Debug, Clone)]
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
}
