use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tokio::sync::broadcast;

use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::health::{check_integrity_parallel, include_disabled_pending, IntegrityHealth};
use crate::web::sse::ServerEvent;

struct CachedIntegrity {
    result: IntegrityHealth,
    checked_at: Instant,
    generation: u64,
}

#[derive(Clone)]
pub struct IntegrityCache {
    cache: Arc<RwLock<Option<CachedIntegrity>>>,
    generation: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    ttl: Duration,
}

impl IntegrityCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicUsize::new(0)),
            total: Arc::new(AtomicUsize::new(0)),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self) -> Option<IntegrityHealth> {
        self.cache.read().as_ref().map(|c| c.result.clone())
    }

    pub fn is_stale(&self) -> bool {
        match self.cache.read().as_ref() {
            Some(c) => {
                c.generation != self.generation.load(Ordering::Relaxed)
                    || c.checked_at.elapsed() >= self.ttl
            }
            None => true,
        }
    }

    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn progress(&self) -> (usize, usize) {
        (
            self.progress.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }

    pub fn start_check(
        &self,
        db: Arc<parking_lot::Mutex<Database>>,
        dirs: QumaDirs,
        events: broadcast::Sender<ServerEvent>,
    ) {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        self.progress.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);

        let generation_at_start = self.generation.load(Ordering::Relaxed);
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            let (tracked_files, pending) = {
                let db = db.lock();
                match db
                    .get_all_enabled_mod_files()
                    .and_then(|files| db.pending_file_rehashes().map(|pending| (files, pending)))
                {
                    Ok(files) => files,
                    Err(e) => {
                        tracing::warn!(err = %e, "integrity check: failed to query tracked files");
                        cache.running.store(false, Ordering::Relaxed);
                        return;
                    }
                }
            };

            match check_integrity_parallel(&tracked_files, &dirs, &cache.progress, &cache.total) {
                Ok(mut result) => {
                    include_disabled_pending(&mut result, tracked_files.len(), &pending);
                    cache.complete_check(result, generation_at_start);
                    let _ = events.send(ServerEvent::IntegrityChanged);
                }
                Err(e) => {
                    tracing::warn!(err = %e, "integrity check failed");
                    cache.running.store(false, Ordering::Relaxed);
                }
            }
        });
    }

    fn complete_check(&self, result: IntegrityHealth, generation_at_start: u64) {
        *self.cache.write() = Some(CachedIntegrity {
            result,
            checked_at: Instant::now(),
            generation: generation_at_start,
        });
        self.running.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::health::IntegrityHealth;

    fn empty_result() -> IntegrityHealth {
        IntegrityHealth {
            tracked_files: 10,
            missing_files: vec![],
            modified_files: vec![],
            unverified_files: vec![],
            untracked_dirs: vec![],
        }
    }

    #[tokio::test]
    async fn cache_reports_disabled_pending_files_without_claiming_verification() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = QumaDirs::from_root(temp.path().into());
        let db = Database::open_in_memory().unwrap();
        db.conn().execute_batch(
            "INSERT INTO installed_mods (id,name,version,disabled) VALUES (1,'Off','1',1);
             INSERT INTO installed_addons (id,forge_addon_id,forge_version_id,parent_mod_id,name,version,disabled) VALUES (1,1,1,1,'Addon','1',0);
             INSERT INTO installed_files (id,mod_id,addon_id,file_path,file_hash) VALUES
                 (1,1,NULL,'BepInEx/plugins/off.dll','0123456789abcdef'),
                 (2,NULL,1,'BepInEx/plugins/addon.dll','0123456789abcdef');
             INSERT INTO pending_file_rehashes SELECT id FROM installed_files;"
        ).unwrap();
        let db = Arc::new(parking_lot::Mutex::new(db));
        let cache = IntegrityCache::new(600);
        let (events, _) = broadcast::channel(4);
        cache.start_check(db, dirs, events);
        tokio::time::timeout(Duration::from_secs(5), async {
            while cache.is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let result = cache.get().expect("completed real integrity check");
        let mut ids: Vec<_> = result.unverified_files.iter().map(|f| f.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(result.tracked_files, 2);
        assert!(result.unverified_files.iter().all(|f| f.reason == "legacy"));
    }

    #[test]
    fn cache_starts_empty_and_stale() {
        let cache = IntegrityCache::new(600);
        assert!(cache.get().is_none());
        assert!(cache.is_stale());
        assert!(!cache.is_running());
    }

    #[test]
    fn cache_hit_after_complete_check() {
        let cache = IntegrityCache::new(600);
        let gen = cache.generation.load(Ordering::Relaxed);
        cache.complete_check(empty_result(), gen);
        let result = cache.get();
        assert!(result.is_some());
        assert_eq!(result.unwrap().tracked_files, 10);
        assert!(!cache.is_stale());
    }

    #[test]
    fn cache_stale_after_invalidate() {
        let cache = IntegrityCache::new(600);
        let gen = cache.generation.load(Ordering::Relaxed);
        cache.complete_check(empty_result(), gen);
        assert!(!cache.is_stale());
        cache.invalidate();
        assert!(cache.is_stale());
        // But the old result is still readable
        assert!(cache.get().is_some());
    }

    #[test]
    fn cache_stale_after_ttl() {
        let cache = IntegrityCache::new(0); // 0s TTL = immediately stale
        let gen = cache.generation.load(Ordering::Relaxed);
        cache.complete_check(empty_result(), gen);
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.is_stale());
    }

    #[test]
    fn cache_stale_when_invalidated_during_check() {
        let cache = IntegrityCache::new(600);
        let gen = cache.generation.load(Ordering::Relaxed);
        cache.invalidate();
        cache.complete_check(empty_result(), gen);
        assert!(cache.is_stale());
        assert!(cache.get().is_some());
    }

    #[test]
    fn progress_tracking() {
        let cache = IntegrityCache::new(600);
        cache.total.store(100, Ordering::Relaxed);
        cache.progress.store(42, Ordering::Relaxed);
        assert_eq!(cache.progress(), (42, 100));
    }
}
