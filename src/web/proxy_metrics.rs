use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

#[allow(dead_code)] // Fields used by Tasks 5-7 (proxy handlers)
pub struct ProxyMetrics {
    pub total_requests: AtomicU64,
    pub error_count: AtomicU64,
    pub launcher_requests: AtomicU64,
    pub client_requests: AtomicU64,
    pub fika_requests: AtomicU64,
    pub other_requests: AtomicU64,
    pub active_ws_connections: AtomicU64,
    latencies: Mutex<LatencyBuffer>,
}

#[allow(dead_code)] // Used internally by ProxyMetrics
struct LatencyBuffer {
    buffer: Vec<u64>,
    cursor: usize,
    filled: bool,
}

const LATENCY_BUFFER_SIZE: usize = 256;

#[allow(dead_code)] // Used internally by ProxyMetrics
impl LatencyBuffer {
    fn new() -> Self {
        Self {
            buffer: vec![0; LATENCY_BUFFER_SIZE],
            cursor: 0,
            filled: false,
        }
    }

    fn push(&mut self, latency_ms: u64) {
        self.buffer[self.cursor] = latency_ms;
        self.cursor = (self.cursor + 1) % LATENCY_BUFFER_SIZE;
        if self.cursor == 0 {
            self.filled = true;
        }
    }

    fn average(&self) -> Option<u64> {
        let count = if self.filled {
            LATENCY_BUFFER_SIZE
        } else {
            self.cursor
        };
        if count == 0 {
            return None;
        }
        let sum: u64 = self.buffer[..count].iter().sum();
        Some(sum / count as u64)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by Task 7 (status page)
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub error_count: u64,
    pub launcher_requests: u64,
    pub client_requests: u64,
    pub fika_requests: u64,
    pub other_requests: u64,
    pub active_ws_connections: u64,
    pub avg_latency_ms: Option<u64>,
}

#[allow(dead_code)] // Used by Tasks 5-7 (proxy handlers, status page)
impl ProxyMetrics {
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            launcher_requests: AtomicU64::new(0),
            client_requests: AtomicU64::new(0),
            fika_requests: AtomicU64::new(0),
            other_requests: AtomicU64::new(0),
            active_ws_connections: AtomicU64::new(0),
            latencies: Mutex::new(LatencyBuffer::new()),
        }
    }

    pub fn record_request(&self, path: &str, latency_ms: u64, is_error: bool) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        if is_error {
            self.error_count.fetch_add(1, Ordering::Relaxed);
        }

        if path.starts_with("/launcher") {
            self.launcher_requests.fetch_add(1, Ordering::Relaxed);
        } else if path.starts_with("/client") {
            self.client_requests.fetch_add(1, Ordering::Relaxed);
        } else if path.starts_with("/fika") {
            self.fika_requests.fetch_add(1, Ordering::Relaxed);
        } else {
            self.other_requests.fetch_add(1, Ordering::Relaxed);
        }

        self.latencies.lock().push(latency_ms);
    }

    pub fn increment_ws_connections(&self) {
        self.active_ws_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_ws_connections(&self) {
        self.active_ws_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
            launcher_requests: self.launcher_requests.load(Ordering::Relaxed),
            client_requests: self.client_requests.load(Ordering::Relaxed),
            fika_requests: self.fika_requests.load(Ordering::Relaxed),
            other_requests: self.other_requests.load(Ordering::Relaxed),
            active_ws_connections: self.active_ws_connections.load(Ordering::Relaxed),
            avg_latency_ms: self.latencies.lock().average(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_request_by_prefix() {
        let m = ProxyMetrics::new();
        m.record_request("/launcher/ping", 10, false);
        m.record_request("/client/game/start", 50, false);
        m.record_request("/fika/headless/get", 20, true);
        m.record_request("/unknown/path", 30, false);

        let snap = m.snapshot();
        assert_eq!(snap.total_requests, 4);
        assert_eq!(snap.error_count, 1);
        assert_eq!(snap.launcher_requests, 1);
        assert_eq!(snap.client_requests, 1);
        assert_eq!(snap.fika_requests, 1);
        assert_eq!(snap.other_requests, 1);
    }

    #[test]
    fn average_latency() {
        let m = ProxyMetrics::new();
        m.record_request("/a", 10, false);
        m.record_request("/b", 30, false);
        assert_eq!(m.snapshot().avg_latency_ms, Some(20));
    }

    #[test]
    fn empty_latency() {
        let m = ProxyMetrics::new();
        assert_eq!(m.snapshot().avg_latency_ms, None);
    }

    #[test]
    fn ws_connection_tracking() {
        let m = ProxyMetrics::new();
        m.increment_ws_connections();
        m.increment_ws_connections();
        assert_eq!(m.snapshot().active_ws_connections, 2);
        m.decrement_ws_connections();
        assert_eq!(m.snapshot().active_ws_connections, 1);
    }
}
