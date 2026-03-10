use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Portal-level telemetry counters and gauges.
///
/// All operations are lock-free via atomics. Designed to be cloned cheaply
/// and shared across request handlers.
#[derive(Clone)]
pub struct PortalTelemetry {
    inner: Arc<TelemetryInner>,
}

struct TelemetryInner {
    // Counters (monotonically increasing)
    requests_total: AtomicU64,
    requests_success: AtomicU64,
    requests_failed: AtomicU64,
    dispatches_total: AtomicU64,
    dispatches_no_worker: AtomicU64,
    sessions_created: AtomicU64,
    sessions_expired: AtomicU64,

    // Gauges (point-in-time values)
    active_sessions: AtomicU64,
    pending_tasks: AtomicU64,
    active_workers: AtomicU64,
    loaded_models: AtomicU64,

    // Latency tracking (sum + count for average)
    inference_time_sum_us: AtomicU64,
    inference_time_count: AtomicU64,
    routing_time_sum_us: AtomicU64,
    routing_time_count: AtomicU64,

    // Startup time
    started_at: u64,
}

impl PortalTelemetry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TelemetryInner {
                requests_total: AtomicU64::new(0),
                requests_success: AtomicU64::new(0),
                requests_failed: AtomicU64::new(0),
                dispatches_total: AtomicU64::new(0),
                dispatches_no_worker: AtomicU64::new(0),
                sessions_created: AtomicU64::new(0),
                sessions_expired: AtomicU64::new(0),
                active_sessions: AtomicU64::new(0),
                pending_tasks: AtomicU64::new(0),
                active_workers: AtomicU64::new(0),
                loaded_models: AtomicU64::new(0),
                inference_time_sum_us: AtomicU64::new(0),
                inference_time_count: AtomicU64::new(0),
                routing_time_sum_us: AtomicU64::new(0),
                routing_time_count: AtomicU64::new(0),
                started_at: now_ms(),
            }),
        }
    }

    // --- Counter increments ---

    pub fn inc_requests(&self) {
        self.inner.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_success(&self) {
        self.inner.requests_success.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_failed(&self) {
        self.inner.requests_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_dispatches(&self) {
        self.inner.dispatches_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_no_worker(&self) {
        self.inner
            .dispatches_no_worker
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_sessions_created(&self) {
        self.inner.sessions_created.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_sessions_expired(&self, count: u64) {
        self.inner
            .sessions_expired
            .fetch_add(count, Ordering::Relaxed);
    }

    // --- Gauge setters ---

    pub fn set_active_sessions(&self, count: u64) {
        self.inner.active_sessions.store(count, Ordering::Relaxed);
    }

    pub fn set_pending_tasks(&self, count: u64) {
        self.inner.pending_tasks.store(count, Ordering::Relaxed);
    }

    pub fn set_active_workers(&self, count: u64) {
        self.inner.active_workers.store(count, Ordering::Relaxed);
    }

    pub fn set_loaded_models(&self, count: u64) {
        self.inner.loaded_models.store(count, Ordering::Relaxed);
    }

    // --- Latency recording ---

    pub fn record_inference_time(&self, us: u64) {
        self.inner
            .inference_time_sum_us
            .fetch_add(us, Ordering::Relaxed);
        self.inner
            .inference_time_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_routing_time(&self, us: u64) {
        self.inner
            .routing_time_sum_us
            .fetch_add(us, Ordering::Relaxed);
        self.inner
            .routing_time_count
            .fetch_add(1, Ordering::Relaxed);
    }

    // --- Snapshot ---

    /// Capture a point-in-time snapshot of all metrics.
    pub fn snapshot(&self) -> TelemetrySnapshot {
        let i = &self.inner;
        let inf_count = i.inference_time_count.load(Ordering::Relaxed);
        let route_count = i.routing_time_count.load(Ordering::Relaxed);

        TelemetrySnapshot {
            uptime_ms: now_ms() - i.started_at,
            requests_total: i.requests_total.load(Ordering::Relaxed),
            requests_success: i.requests_success.load(Ordering::Relaxed),
            requests_failed: i.requests_failed.load(Ordering::Relaxed),
            dispatches_total: i.dispatches_total.load(Ordering::Relaxed),
            dispatches_no_worker: i.dispatches_no_worker.load(Ordering::Relaxed),
            sessions_created: i.sessions_created.load(Ordering::Relaxed),
            sessions_expired: i.sessions_expired.load(Ordering::Relaxed),
            active_sessions: i.active_sessions.load(Ordering::Relaxed),
            pending_tasks: i.pending_tasks.load(Ordering::Relaxed),
            active_workers: i.active_workers.load(Ordering::Relaxed),
            loaded_models: i.loaded_models.load(Ordering::Relaxed),
            avg_inference_us: if inf_count > 0 {
                i.inference_time_sum_us.load(Ordering::Relaxed) / inf_count
            } else {
                0
            },
            avg_routing_us: if route_count > 0 {
                i.routing_time_sum_us.load(Ordering::Relaxed) / route_count
            } else {
                0
            },
        }
    }
}

impl Default for PortalTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable point-in-time metrics snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySnapshot {
    pub uptime_ms: u64,
    pub requests_total: u64,
    pub requests_success: u64,
    pub requests_failed: u64,
    pub dispatches_total: u64,
    pub dispatches_no_worker: u64,
    pub sessions_created: u64,
    pub sessions_expired: u64,
    pub active_sessions: u64,
    pub pending_tasks: u64,
    pub active_workers: u64,
    pub loaded_models: u64,
    pub avg_inference_us: u64,
    pub avg_routing_us: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters() {
        let t = PortalTelemetry::new();
        t.inc_requests();
        t.inc_requests();
        t.inc_success();
        t.inc_failed();

        let snap = t.snapshot();
        assert_eq!(snap.requests_total, 2);
        assert_eq!(snap.requests_success, 1);
        assert_eq!(snap.requests_failed, 1);
    }

    #[test]
    fn gauges() {
        let t = PortalTelemetry::new();
        t.set_active_sessions(5);
        t.set_pending_tasks(12);
        t.set_active_workers(3);
        t.set_loaded_models(2);

        let snap = t.snapshot();
        assert_eq!(snap.active_sessions, 5);
        assert_eq!(snap.pending_tasks, 12);
        assert_eq!(snap.active_workers, 3);
        assert_eq!(snap.loaded_models, 2);
    }

    #[test]
    fn latency_average() {
        let t = PortalTelemetry::new();
        t.record_inference_time(1000);
        t.record_inference_time(3000);

        let snap = t.snapshot();
        assert_eq!(snap.avg_inference_us, 2000);
    }

    #[test]
    fn uptime() {
        let t = PortalTelemetry::new();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let snap = t.snapshot();
        assert!(snap.uptime_ms >= 4);
    }

    #[test]
    fn clone_shares_state() {
        let t1 = PortalTelemetry::new();
        let t2 = t1.clone();

        t1.inc_requests();
        t2.inc_requests();

        assert_eq!(t1.snapshot().requests_total, 2);
    }
}
