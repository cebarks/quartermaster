use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    min_interval: Duration,
    last_request: Arc<Mutex<Option<Instant>>>,
}

impl RateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_request: Arc::new(Mutex::new(None)),
        }
    }

    /// Wait until enough time has passed since the last request.
    pub async fn acquire(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_does_not_block_first_call() {
        let rl = RateLimiter::new(Duration::from_millis(100));
        let start = Instant::now();
        rl.acquire().await;
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn acquire_spaces_sequential_calls() {
        let rl = RateLimiter::new(Duration::from_millis(100));
        rl.acquire().await;
        let start = Instant::now();
        rl.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(90));
    }

    #[tokio::test]
    async fn acquire_does_not_wait_if_interval_elapsed() {
        let rl = RateLimiter::new(Duration::from_millis(50));
        rl.acquire().await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let start = Instant::now();
        rl.acquire().await;
        assert!(start.elapsed() < Duration::from_millis(30));
    }
}
