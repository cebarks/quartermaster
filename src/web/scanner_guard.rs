use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use actix_web::body::BoxBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{web, HttpResponse};
use parking_lot::Mutex;

struct IpState {
    consecutive_unhandled: u32,
    last_seen: Instant,
}

struct GuardState {
    counters: HashMap<IpAddr, IpState>,
    bans: HashMap<IpAddr, Instant>,
}

pub struct ScannerGuard {
    state: Mutex<GuardState>,
    threshold: u32,
    ban_duration: Duration,
    request_count: AtomicU64,
}

// ponytail: cleanup every 1000 requests, stale counter expiry 10 minutes
const CLEANUP_INTERVAL: u64 = 1000;
const STALE_COUNTER_SECS: u64 = 600;

impl ScannerGuard {
    pub fn new(threshold: u32, ban_duration: Duration) -> Self {
        Self {
            state: Mutex::new(GuardState {
                counters: HashMap::new(),
                bans: HashMap::new(),
            }),
            threshold,
            ban_duration,
            request_count: AtomicU64::new(0),
        }
    }

    pub fn is_banned(&self, ip: &IpAddr) -> bool {
        let state = self.state.lock();
        matches!(state.bans.get(ip), Some(expiry) if Instant::now() < *expiry)
    }

    pub fn record_response(&self, ip: IpAddr, status: u16) {
        let now = Instant::now();
        let mut state = self.state.lock();

        if status == 404 || status == 405 {
            let entry = state.counters.entry(ip).or_insert(IpState {
                consecutive_unhandled: 0,
                last_seen: now,
            });
            entry.consecutive_unhandled += 1;
            entry.last_seen = now;

            if entry.consecutive_unhandled >= self.threshold {
                let expiry = now + self.ban_duration;
                state.bans.insert(ip, expiry);
                state.counters.remove(&ip);
                tracing::warn!(
                    ip = %ip,
                    threshold = self.threshold,
                    ban_duration_secs = self.ban_duration.as_secs(),
                    "scanner guard banned IP after consecutive unhandled requests"
                );
            }
        } else {
            state.counters.remove(&ip);
        }

        // Periodic cleanup outside the hot path
        drop(state);
        let count = self.request_count.fetch_add(1, Ordering::Relaxed);
        if count > 0 && count.is_multiple_of(CLEANUP_INTERVAL) {
            self.cleanup(now);
        }
    }

    fn cleanup(&self, now: Instant) {
        let mut state = self.state.lock();
        let stale_threshold = Duration::from_secs(STALE_COUNTER_SECS);
        state
            .counters
            .retain(|_, v| now.duration_since(v.last_seen) < stale_threshold);
        state.bans.retain(|_, expiry| now < *expiry);
    }
}

pub async fn scanner_guard_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let guard = req.app_data::<web::Data<ScannerGuard>>().cloned();

    let Some(guard) = guard else {
        return next.call(req).await;
    };

    let Some(ip) = req.peer_addr().map(|a| a.ip()) else {
        return next.call(req).await;
    };

    if guard.is_banned(&ip) {
        tracing::warn!(
            ip = %ip,
            method = %req.method(),
            path = %req.path(),
            "scanner guard blocked request from banned IP"
        );
        let response = HttpResponse::Forbidden().body("Forbidden");
        return Ok(req.into_response(response).map_into_boxed_body());
    }

    let resp = next.call(req).await?;
    let status = resp.status().as_u16();
    guard.record_response(ip, status);
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_404s_trigger_ban() {
        let guard = ScannerGuard::new(5, Duration::from_secs(3600));
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        for _ in 0..4 {
            guard.record_response(ip, 404);
            assert!(!guard.is_banned(&ip));
        }

        guard.record_response(ip, 404);
        assert!(guard.is_banned(&ip));
    }

    #[test]
    fn success_resets_counter() {
        let guard = ScannerGuard::new(5, Duration::from_secs(3600));
        let ip: IpAddr = "10.0.0.2".parse().unwrap();

        for _ in 0..4 {
            guard.record_response(ip, 404);
        }
        // One success resets
        guard.record_response(ip, 200);

        // Need another full run to ban
        for _ in 0..4 {
            guard.record_response(ip, 404);
        }
        assert!(!guard.is_banned(&ip));

        guard.record_response(ip, 404);
        assert!(guard.is_banned(&ip));
    }

    #[test]
    fn below_threshold_not_banned() {
        let guard = ScannerGuard::new(10, Duration::from_secs(3600));
        let ip: IpAddr = "10.0.0.3".parse().unwrap();

        for _ in 0..9 {
            guard.record_response(ip, 404);
        }
        assert!(!guard.is_banned(&ip));
    }

    #[test]
    fn ban_expires() {
        let guard = ScannerGuard::new(2, Duration::from_secs(0));
        let ip: IpAddr = "10.0.0.4".parse().unwrap();

        guard.record_response(ip, 404);
        guard.record_response(ip, 404);
        // Ban duration is 0 seconds, so it should already be expired
        assert!(!guard.is_banned(&ip));
    }

    #[test]
    fn mixed_status_codes() {
        let guard = ScannerGuard::new(3, Duration::from_secs(3600));
        let ip: IpAddr = "10.0.0.5".parse().unwrap();

        guard.record_response(ip, 404);
        guard.record_response(ip, 405);
        // 401 is NOT unhandled -- resets counter
        guard.record_response(ip, 401);
        assert!(!guard.is_banned(&ip));

        // Need 3 more consecutive unhandled
        guard.record_response(ip, 404);
        guard.record_response(ip, 405);
        guard.record_response(ip, 404);
        assert!(guard.is_banned(&ip));
    }

    #[test]
    fn different_ips_tracked_independently() {
        let guard = ScannerGuard::new(3, Duration::from_secs(3600));
        let ip1: IpAddr = "10.0.0.6".parse().unwrap();
        let ip2: IpAddr = "10.0.0.7".parse().unwrap();

        for _ in 0..3 {
            guard.record_response(ip1, 404);
        }
        assert!(guard.is_banned(&ip1));
        assert!(!guard.is_banned(&ip2));
    }

    #[test]
    fn cleanup_removes_expired_bans() {
        let guard = ScannerGuard::new(2, Duration::from_secs(0));
        let ip: IpAddr = "10.0.0.8".parse().unwrap();

        guard.record_response(ip, 404);
        guard.record_response(ip, 404);

        // Force cleanup
        guard.cleanup(Instant::now() + Duration::from_secs(1));

        let state = guard.state.lock();
        assert!(!state.bans.contains_key(&ip));
    }
}
