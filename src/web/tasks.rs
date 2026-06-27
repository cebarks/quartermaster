use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::web::sse::ServerEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Running { message: String },
    Completed { message: String },
    Failed { message: String },
}

impl TaskStatus {
    pub fn css_class(&self) -> &'static str {
        match self {
            TaskStatus::Running { .. } => "alert-info loading-pulse",
            TaskStatus::Completed { .. } => "alert-success",
            TaskStatus::Failed { .. } => "alert-error",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            TaskStatus::Running { message }
            | TaskStatus::Completed { message }
            | TaskStatus::Failed { message } => message,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, TaskStatus::Running { .. })
    }
}

#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub status: TaskStatus,
    pub forge_mod_id: i64,
}

/// View-model for templates — pre-computed CSS class and message to avoid
/// askama match blocks with full crate paths.
pub struct TaskView {
    pub id: u64,
    pub css_class: String,
    pub message: String,
    pub is_running: bool,
}

struct TrackerInner {
    tasks: HashMap<u64, TaskInfo>,
    next_id: u64,
    events_tx: broadcast::Sender<ServerEvent>,
}

#[derive(Clone)]
pub struct TaskTracker {
    inner: Arc<Mutex<TrackerInner>>,
}

impl TaskTracker {
    pub fn new(events_tx: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(TrackerInner {
                tasks: HashMap::new(),
                next_id: 1,
                events_tx,
            })),
        }
    }

    fn send_event(inner: &TrackerInner, event: ServerEvent) {
        let _ = inner.events_tx.send(event);
    }

    #[allow(dead_code)] // Useful read-only query; handlers now use start_if_not_running
    pub fn has_running_for_mod(&self, forge_mod_id: i64) -> bool {
        let inner = self.inner.lock();
        inner
            .tasks
            .values()
            .any(|t| t.forge_mod_id == forge_mod_id && t.status.is_running())
    }

    #[allow(dead_code)] // Non-atomic version; handlers now use start_if_not_running
    pub fn start(&self, action: &str, mod_name: &str, forge_mod_id: i64) -> u64 {
        let mut inner = self.inner.lock();
        let id = inner.next_id;
        inner.next_id += 1;

        let info = TaskInfo {
            status: TaskStatus::Running {
                message: format!("{} {}...", action, mod_name),
            },
            forge_mod_id,
        };
        inner.tasks.insert(id, info);
        Self::send_event(&inner, ServerEvent::TaskChanged);
        id
    }

    pub fn complete(&self, id: u64, message: String) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(&id) {
            task.status = TaskStatus::Completed { message };
        }
        Self::send_event(&inner, ServerEvent::TaskChanged);
        Self::send_event(&inner, ServerEvent::ModsChanged);
        Self::prune_old(&mut inner);
    }

    pub fn fail(&self, id: u64, message: String) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(&id) {
            task.status = TaskStatus::Failed { message };
        }
        Self::send_event(&inner, ServerEvent::TaskChanged);
        Self::prune_old(&mut inner);
    }

    pub fn update_message(&self, id: u64, message: String) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(&id) {
            if task.status.is_running() {
                task.status = TaskStatus::Running { message };
                Self::send_event(&inner, ServerEvent::TaskChanged);
            }
        }
    }

    pub fn task_views(&self) -> Vec<TaskView> {
        let inner = self.inner.lock();
        inner
            .tasks
            .iter()
            .map(|(id, info)| TaskView {
                id: *id,
                css_class: info.status.css_class().to_string(),
                message: info.status.message().to_string(),
                is_running: info.status.is_running(),
            })
            .collect()
    }

    pub fn dismiss(&self, id: u64) {
        self.inner.lock().tasks.remove(&id);
    }

    /// Atomically check whether any task is running for `forge_mod_id` and, if
    /// not, start a new one.  Returns `Some(task_id)` on success, `None` if a
    /// task for that mod is already in progress.
    pub fn start_if_not_running(
        &self,
        action: &str,
        mod_name: &str,
        forge_mod_id: i64,
    ) -> Option<u64> {
        let mut inner = self.inner.lock();
        let already_running = inner
            .tasks
            .values()
            .any(|t| t.forge_mod_id == forge_mod_id && t.status.is_running());
        if already_running {
            return None;
        }

        let id = inner.next_id;
        inner.next_id += 1;
        let info = TaskInfo {
            status: TaskStatus::Running {
                message: format!("{} {}...", action, mod_name),
            },
            forge_mod_id,
        };
        inner.tasks.insert(id, info);
        Self::send_event(&inner, ServerEvent::TaskChanged);
        Some(id)
    }

    /// Atomically check whether *any* task is currently running and, if not,
    /// start a new one.  Returns `Some(task_id)` on success, `None` if any
    /// task is active.  Intended for bulk operations (e.g. "update all",
    /// "apply queue") that must not overlap with other work.
    pub fn start_if_no_active(
        &self,
        action: &str,
        mod_name: &str,
        forge_mod_id: i64,
    ) -> Option<u64> {
        let mut inner = self.inner.lock();
        let any_running = inner.tasks.values().any(|t| t.status.is_running());
        if any_running {
            return None;
        }

        let id = inner.next_id;
        inner.next_id += 1;
        let info = TaskInfo {
            status: TaskStatus::Running {
                message: format!("{} {}...", action, mod_name),
            },
            forge_mod_id,
        };
        inner.tasks.insert(id, info);
        Self::send_event(&inner, ServerEvent::TaskChanged);
        Some(id)
    }

    #[allow(dead_code)] // Useful read-only query; handlers now use start_if_no_active
    pub fn has_active(&self) -> bool {
        self.inner
            .lock()
            .tasks
            .values()
            .any(|t| t.status.is_running())
    }

    /// Remove completed/failed tasks that exceed a cap (keep at most 20 finished tasks).
    fn prune_old(inner: &mut TrackerInner) {
        let finished: Vec<u64> = inner
            .tasks
            .iter()
            .filter(|(_, t)| !t.status.is_running())
            .map(|(id, _)| *id)
            .collect();
        if finished.len() > 20 {
            let mut to_remove: Vec<u64> = finished;
            to_remove.sort();
            for id in &to_remove[..to_remove.len() - 20] {
                inner.tasks.remove(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn make_tracker() -> TaskTracker {
        let (tx, _rx) = broadcast::channel(16);
        TaskTracker::new(tx)
    }

    #[test]
    fn start_if_not_running_returns_some_then_none_for_same_mod() {
        let tracker = make_tracker();
        let first = tracker.start_if_not_running("Installing", "TestMod", 42);
        assert!(first.is_some(), "first call should return Some(task_id)");

        let second = tracker.start_if_not_running("Installing", "TestMod", 42);
        assert!(
            second.is_none(),
            "second call for same mod should return None"
        );
    }

    #[test]
    fn start_if_not_running_allows_different_mods() {
        let tracker = make_tracker();
        let first = tracker.start_if_not_running("Installing", "ModA", 1);
        assert!(first.is_some());

        let second = tracker.start_if_not_running("Installing", "ModB", 2);
        assert!(second.is_some(), "different mod_id should be allowed");
    }

    #[test]
    fn start_if_no_active_returns_none_when_task_running() {
        let tracker = make_tracker();
        // Start a task for some mod so there is an active task.
        let _id = tracker.start("Installing", "SomeMod", 99);

        let result = tracker.start_if_no_active("Update All", "bulk", 0);
        assert!(
            result.is_none(),
            "should return None when any task is active"
        );
    }

    #[test]
    fn start_if_no_active_returns_some_when_idle() {
        let tracker = make_tracker();
        let result = tracker.start_if_no_active("Update All", "bulk", 0);
        assert!(
            result.is_some(),
            "should return Some when no tasks are active"
        );
    }

    #[test]
    fn start_if_not_running_allows_restart_after_complete() {
        let tracker = make_tracker();
        let task_id = tracker
            .start_if_not_running("Installing", "TestMod", 42)
            .expect("first start should succeed");

        tracker.complete(task_id, "done".to_string());

        let second = tracker.start_if_not_running("Installing", "TestMod", 42);
        assert!(second.is_some(), "should allow restarting after completion");
    }

    #[test]
    fn start_if_not_running_allows_restart_after_failure() {
        let tracker = make_tracker();
        let task_id = tracker
            .start_if_not_running("Installing", "TestMod", 42)
            .expect("first start should succeed");

        tracker.fail(task_id, "error".to_string());

        let second = tracker.start_if_not_running("Installing", "TestMod", 42);
        assert!(second.is_some(), "should allow restarting after failure");
    }
}
