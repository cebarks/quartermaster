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
    pub mod_name: String,
    pub action: String,
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

    pub fn has_running_for_mod(&self, forge_mod_id: i64) -> bool {
        let inner = self.inner.lock();
        inner
            .tasks
            .values()
            .any(|t| t.forge_mod_id == forge_mod_id && t.status.is_running())
    }

    pub fn start(&self, action: &str, mod_name: &str, forge_mod_id: i64) -> u64 {
        let mut inner = self.inner.lock();
        let id = inner.next_id;
        inner.next_id += 1;

        let info = TaskInfo {
            status: TaskStatus::Running {
                message: format!("{} {}...", action, mod_name),
            },
            mod_name: mod_name.to_string(),
            action: action.to_string(),
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
