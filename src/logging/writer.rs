use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::db::Database;
use crate::logging::LogEntry;

pub struct LogWriterHandle {
    shutdown_tx: mpsc::Sender<()>,
}

impl LogWriterHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(()).await;
    }
}

pub fn spawn(
    db: Arc<Mutex<Database>>,
    mut rx: mpsc::UnboundedReceiver<LogEntry>,
    retention_days: u64,
    max_entries: u64,
) -> (tokio::task::JoinHandle<()>, LogWriterHandle) {
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    let handle = tokio::spawn(async move {
        let mut batch: Vec<LogEntry> = Vec::with_capacity(100);
        let flush_interval = Duration::from_millis(500);
        let mut flush_timer = tokio::time::interval(flush_interval);
        flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Prune at startup
        {
            let db = Arc::clone(&db);
            let _ = tokio::task::spawn_blocking(move || {
                let db = db.lock();
                if let Err(e) = db.prune_logs_batch(retention_days, max_entries, 10_000) {
                    tracing::warn!(err = %e, "failed to prune old log entries at startup");
                }
            })
            .await;
        }

        // Set up daily prune timer
        let mut prune_interval = tokio::time::interval(Duration::from_secs(86_400));
        prune_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let _ = prune_interval.tick().await; // skip immediate tick

        loop {
            tokio::select! {
                Some(entry) = rx.recv() => {
                    batch.push(entry);
                    if batch.len() >= 100 {
                        flush_batch(&db, &mut batch).await;
                    }
                }
                _ = flush_timer.tick() => {
                    if !batch.is_empty() {
                        flush_batch(&db, &mut batch).await;
                    }
                }
                _ = prune_interval.tick() => {
                    let db = Arc::clone(&db);
                    let _ = tokio::task::spawn_blocking(move || {
                        let db = db.lock();
                        if let Err(e) = db.prune_logs_batch(retention_days, max_entries, 10_000) {
                            tracing::warn!(err = %e, "failed to prune old log entries");
                        }
                    }).await;
                }
                _ = shutdown_rx.recv() => {
                    // Drain remaining entries
                    while let Ok(entry) = rx.try_recv() {
                        batch.push(entry);
                    }
                    if !batch.is_empty() {
                        flush_batch(&db, &mut batch).await;
                    }
                    break;
                }
            }
        }
    });

    (handle, LogWriterHandle { shutdown_tx })
}

/// Flush accumulated log entries to the DB via spawn_blocking.
/// MUST use spawn_blocking — the DB is behind a parking_lot::Mutex and
/// acquiring it on the async runtime blocks the tokio worker thread.
async fn flush_batch(db: &Arc<Mutex<Database>>, batch: &mut Vec<LogEntry>) {
    let db = Arc::clone(db);
    let entries = std::mem::take(batch);
    let _ = tokio::task::spawn_blocking(move || {
        let db = db.lock();
        if let Err(e) = db.insert_log_batch(&entries) {
            eprintln!("failed to write log batch to DB: {e}");
        }
    })
    .await;
}
