use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::health::IntegrityHealth;

struct CachedIntegrity {
    result: IntegrityHealth,
    checked_at: Instant,
}

#[derive(Clone)]
pub struct IntegrityCache {
    cache: Arc<RwLock<Option<CachedIntegrity>>>,
    dirty: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    ttl: Duration,
}

impl IntegrityCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
            dirty: Arc::new(AtomicBool::new(true)),
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
        if self.dirty.load(Ordering::Relaxed) {
            return true;
        }
        match self.cache.read().as_ref() {
            Some(c) => c.checked_at.elapsed() >= self.ttl,
            None => true,
        }
    }

    pub fn invalidate(&self) {
        self.dirty.store(true, Ordering::Relaxed);
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

    fn set(&self, result: IntegrityHealth) {
        *self.cache.write() = Some(CachedIntegrity {
            result,
            checked_at: Instant::now(),
        });
        self.dirty.store(false, Ordering::Relaxed);
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
            untracked_dirs: vec![],
        }
    }

    #[test]
    fn cache_starts_empty_and_stale() {
        let cache = IntegrityCache::new(600);
        assert!(cache.get().is_none());
        assert!(cache.is_stale());
        assert!(!cache.is_running());
    }

    #[test]
    fn cache_hit_after_set() {
        let cache = IntegrityCache::new(600);
        cache.set(empty_result());
        let result = cache.get();
        assert!(result.is_some());
        assert_eq!(result.unwrap().tracked_files, 10);
        assert!(!cache.is_stale());
    }

    #[test]
    fn cache_stale_after_invalidate() {
        let cache = IntegrityCache::new(600);
        cache.set(empty_result());
        assert!(!cache.is_stale());
        cache.invalidate();
        assert!(cache.is_stale());
        // But the old result is still readable
        assert!(cache.get().is_some());
    }

    #[test]
    fn cache_stale_after_ttl() {
        let cache = IntegrityCache::new(0); // 0s TTL = immediately stale
        cache.set(empty_result());
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.is_stale());
    }

    #[test]
    fn progress_tracking() {
        let cache = IntegrityCache::new(600);
        cache.total.store(100, Ordering::Relaxed);
        cache.progress.store(42, Ordering::Relaxed);
        assert_eq!(cache.progress(), (42, 100));
    }
}
