//! Headless-only orchestration. Generic containers never acquire policy semantics.
use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use super::{runtime, PolicyOperationGuard, PolicySnapshot};
use crate::{container::ContainerManager, db::Database, dirs::QumaDirs};

pub(crate) fn acquire(dirs: &QumaDirs, client: u32) -> Result<Arc<PolicyOperationGuard>> {
    Ok(Arc::new(PolicyOperationGuard::acquire(dirs, client)?))
}

/// Acquire the bounded managed namespace in ascending order before destructive
/// multi-client work. Failure drops the partial set, before any lifecycle write.
pub(crate) fn acquire_all(dirs: &QumaDirs) -> Result<Vec<Arc<PolicyOperationGuard>>> {
    (1..=crate::config::MAX_HEADLESS_CLIENTS)
        .map(|client| acquire(dirs, client))
        .collect()
}

pub(crate) fn current_policy(db: &Mutex<Database>) -> Result<PolicySnapshot> {
    PolicySnapshot::load(&db.lock())
}

/// Caller owns the lock across any preceding stop and this real start. There is
/// no detached start task: dropping this future during preparation cannot start.
pub(crate) async fn start_locked(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    db: &Mutex<Database>,
    client: u32,
    guard: Arc<PolicyOperationGuard>,
) -> Result<()> {
    let policy = current_policy(db)?;
    runtime::prepare_stopped_client(manager, dirs, policy, client, guard.clone()).await?;
    guard.validate(dirs, client)?;
    manager.start(&format!("fika-headless-{client}")).await
}

pub(crate) async fn restart_locked(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    db: &Mutex<Database>,
    client: u32,
    guard: Arc<PolicyOperationGuard>,
) -> Result<()> {
    guard.validate(dirs, client)?;
    let name = format!("fika-headless-{client}");
    if manager.is_running(&name).await? {
        manager.stop(&name).await?;
    }
    start_locked(manager, dirs, db, client, guard).await
}
