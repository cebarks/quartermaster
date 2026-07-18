use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

const EXPIRY_SECS: u64 = 60;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct OperationId(pub u64);

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OperationStatus {
    Running,
    Completed,
    Failed { error: String },
}

struct OperationEntry {
    status: OperationStatus,
    completed_at: Option<Instant>,
}

pub struct OperationTracker {
    ops: parking_lot::Mutex<HashMap<u64, OperationEntry>>,
    next_id: AtomicU64,
}

impl OperationTracker {
    pub fn new() -> Self {
        Self {
            ops: parking_lot::Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn start(&self) -> OperationId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut ops = self.ops.lock();
        self.expire(&mut ops);
        ops.insert(
            id,
            OperationEntry {
                status: OperationStatus::Running,
                completed_at: None,
            },
        );
        OperationId(id)
    }

    pub fn complete(&self, id: &OperationId) {
        let mut ops = self.ops.lock();
        if let Some(entry) = ops.get_mut(&id.0) {
            entry.status = OperationStatus::Completed;
            entry.completed_at = Some(Instant::now());
        }
    }

    pub fn fail(&self, id: &OperationId, error: String) {
        let mut ops = self.ops.lock();
        if let Some(entry) = ops.get_mut(&id.0) {
            entry.status = OperationStatus::Failed { error };
            entry.completed_at = Some(Instant::now());
        }
    }

    pub fn poll(&self, id: u64) -> Option<OperationStatus> {
        let ops = self.ops.lock();
        ops.get(&id).map(|e| e.status.clone())
    }

    fn expire(&self, ops: &mut HashMap<u64, OperationEntry>) {
        let now = Instant::now();
        ops.retain(|_, entry| {
            entry
                .completed_at
                .map(|t| now.duration_since(t).as_secs() < EXPIRY_SECS)
                .unwrap_or(true)
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn start_returns_incrementing_ids() {
        let tracker = OperationTracker::new();
        let id1 = tracker.start();
        let id2 = tracker.start();
        assert_eq!(id1.0, 1);
        assert_eq!(id2.0, 2);
    }

    #[test]
    fn poll_returns_running_then_completed() {
        let tracker = OperationTracker::new();
        let id = tracker.start();
        assert!(matches!(tracker.poll(id.0), Some(OperationStatus::Running)));
        tracker.complete(&id);
        assert!(matches!(
            tracker.poll(id.0),
            Some(OperationStatus::Completed)
        ));
    }

    #[test]
    fn poll_returns_failed_with_message() {
        let tracker = OperationTracker::new();
        let id = tracker.start();
        tracker.fail(&id, "boom".into());
        match tracker.poll(id.0) {
            Some(OperationStatus::Failed { error }) => assert_eq!(error, "boom"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn poll_unknown_id_returns_none() {
        let tracker = OperationTracker::new();
        assert!(tracker.poll(999).is_none());
    }
}
