use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::forge::models::UpdatesResponseData;

#[derive(Clone)]
pub struct UpdateCache {
    inner: Arc<Mutex<Option<(Instant, UpdatesResponseData)>>>,
    ttl: Duration,
}

impl UpdateCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self) -> Option<UpdatesResponseData> {
        let guard = self.inner.lock();
        guard.as_ref().and_then(|(ts, data)| {
            if ts.elapsed() < self.ttl {
                Some(data.clone())
            } else {
                None
            }
        })
    }

    pub fn set(&self, data: UpdatesResponseData) {
        let mut guard = self.inner.lock();
        *guard = Some((Instant::now(), data));
    }

    pub fn invalidate(&self) {
        let mut guard = self.inner.lock();
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::models::UpdatesResponseData;

    fn empty_response() -> UpdatesResponseData {
        UpdatesResponseData {
            spt_version: "4.0.0".to_string(),
            updates: vec![],
            blocked_updates: vec![],
            up_to_date: vec![],
            incompatible_with_spt: vec![],
        }
    }

    #[test]
    fn cache_miss_when_empty() {
        let cache = UpdateCache::new(300);
        assert!(cache.get().is_none());
    }

    #[test]
    fn cache_hit_after_set() {
        let cache = UpdateCache::new(300);
        cache.set(empty_response());
        let result = cache.get();
        assert!(result.is_some());
        assert_eq!(result.unwrap().spt_version, "4.0.0");
    }

    #[test]
    fn cache_miss_after_invalidate() {
        let cache = UpdateCache::new(300);
        cache.set(empty_response());
        cache.invalidate();
        assert!(cache.get().is_none());
    }
}
