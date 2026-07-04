use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::db::Database;
use crate::logging::LogEntry;

pub type LogLevelCounts = Arc<parking_lot::RwLock<HashMap<String, i64>>>;

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
    counts: LogLevelCounts,
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
            let counts = Arc::clone(&counts);
            let _ = tokio::task::spawn_blocking(move || {
                let db = db.lock();
                if let Err(e) = db.prune_logs_batch(retention_days, max_entries, 10_000) {
                    tracing::warn!(err = %e, "failed to prune old log entries at startup");
                }
                // Recalculate counts after prune
                match db.log_counts_by_level() {
                    Ok(fresh) => *counts.write() = fresh,
                    Err(e) => tracing::warn!(err = %e, "failed to load log counts after prune"),
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
                        flush_batch(&db, &mut batch, &counts).await;
                    }
                }
                _ = flush_timer.tick() => {
                    if !batch.is_empty() {
                        flush_batch(&db, &mut batch, &counts).await;
                    }
                }
                _ = prune_interval.tick() => {
                    let db = Arc::clone(&db);
                    let counts = Arc::clone(&counts);
                    let _ = tokio::task::spawn_blocking(move || {
                        let db = db.lock();
                        if let Err(e) = db.prune_logs_batch(retention_days, max_entries, 10_000) {
                            tracing::warn!(err = %e, "failed to prune old log entries");
                        }
                        // Recalculate counts after prune
                        match db.log_counts_by_level() {
                            Ok(fresh) => *counts.write() = fresh,
                            Err(e) => tracing::warn!(err = %e, "failed to reload log counts after prune"),
                        }
                    }).await;
                }
                _ = shutdown_rx.recv() => {
                    // Drain remaining entries
                    while let Ok(entry) = rx.try_recv() {
                        batch.push(entry);
                    }
                    if !batch.is_empty() {
                        flush_batch(&db, &mut batch, &counts).await;
                    }
                    break;
                }
            }
        }
    });

    (handle, LogWriterHandle { shutdown_tx })
}

async fn flush_batch(
    db: &Arc<Mutex<Database>>,
    batch: &mut Vec<LogEntry>,
    counts: &LogLevelCounts,
) {
    let db = Arc::clone(db);
    let entries = std::mem::take(batch);
    // Accumulate deltas before spawning blocking task
    let mut deltas: HashMap<String, i64> = HashMap::new();
    for entry in &entries {
        *deltas.entry(entry.level.clone()).or_default() += 1;
    }
    let counts = Arc::clone(counts);
    let _ = tokio::task::spawn_blocking(move || {
        let db = db.lock();
        if let Err(e) = db.insert_log_batch(&entries) {
            eprintln!("failed to write log batch to DB: {e}");
            return;
        }
        // Update cached counts
        let mut c = counts.write();
        for (level, delta) in deltas {
            *c.entry(level).or_default() += delta;
        }
    })
    .await;
}
