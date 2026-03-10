use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{info, warn};

use bunny_crypto::types::AgentId;

use super::worker_pool::WorkerPool;

/// Heartbeat monitoring and dead-worker detection.
///
/// The `HealthMonitor` periodically checks worker heartbeat timestamps
/// and marks workers as dead if they exceed the timeout. Dead workers
/// can then be evicted from the pool.
pub struct HealthMonitor {
    pool: WorkerPool,
    heartbeat_timeout_ms: u64,
    check_interval_ms: u64,
    last_check: u64,
}

impl HealthMonitor {
    /// Create a new health monitor.
    ///
    /// - `heartbeat_timeout_ms`: time since last heartbeat before marking dead (default 30s)
    /// - `check_interval_ms`: minimum time between health checks (default 10s)
    pub fn new(pool: WorkerPool, heartbeat_timeout_ms: u64, check_interval_ms: u64) -> Self {
        Self {
            pool,
            heartbeat_timeout_ms,
            check_interval_ms,
            last_check: 0,
        }
    }

    /// Run a health check if enough time has elapsed since the last one.
    ///
    /// Returns (newly_dead, evicted) worker IDs.
    pub fn check(&mut self) -> (Vec<AgentId>, Vec<AgentId>) {
        let now = now_ms();
        if now.saturating_sub(self.last_check) < self.check_interval_ms {
            return (vec![], vec![]);
        }
        self.last_check = now;

        let newly_dead = self.pool.mark_dead(self.heartbeat_timeout_ms);
        if !newly_dead.is_empty() {
            warn!(
                count = newly_dead.len(),
                "workers missed heartbeat, marked dead"
            );
        }

        let evicted = self.pool.evict_dead();
        if !evicted.is_empty() {
            info!(count = evicted.len(), "evicted dead workers");
        }

        (newly_dead, evicted)
    }

    /// Force a health check regardless of interval.
    pub fn force_check(&mut self) -> (Vec<AgentId>, Vec<AgentId>) {
        self.last_check = 0;
        self.check()
    }

    /// Record a heartbeat from a worker.
    pub fn heartbeat(&self, agent_id: &AgentId) {
        self.pool.heartbeat(agent_id);
    }

    /// Current pool stats: (total, idle, busy, dead).
    pub fn stats(&self) -> HealthStats {
        let snapshot = self.pool.snapshot();
        let total = snapshot.len();
        let idle = snapshot
            .iter()
            .filter(|w| w.status == super::worker_pool::WorkerStatus::Idle)
            .count();
        let busy = snapshot
            .iter()
            .filter(|w| w.status == super::worker_pool::WorkerStatus::Busy)
            .count();
        let dead = snapshot
            .iter()
            .filter(|w| w.status == super::worker_pool::WorkerStatus::Dead)
            .count();

        HealthStats {
            total,
            idle,
            busy,
            dead,
        }
    }
}

/// Summary statistics for the worker pool health.
#[derive(Debug, Clone, Copy)]
pub struct HealthStats {
    pub total: usize,
    pub idle: usize,
    pub busy: usize,
    pub dead: usize,
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
    use super::super::worker_pool::WorkerInfo;

    #[test]
    fn health_check_interval() {
        let pool = WorkerPool::new();
        // Register a worker with ancient heartbeat.
        let id = AgentId::new();
        let mut info = WorkerInfo::new(id.clone(), vec![]);
        info.last_heartbeat = 0;
        pool.register(info);

        let mut monitor = HealthMonitor::new(pool, 1000, 5000);

        // First check: should detect dead worker and evict.
        let (dead, evicted) = monitor.force_check();
        assert_eq!(dead.len(), 1);
        assert_eq!(evicted.len(), 1);

        // Second immediate check: interval not elapsed, should return empty.
        let (dead, evicted) = monitor.check();
        assert!(dead.is_empty());
        assert!(evicted.is_empty());
    }

    #[test]
    fn stats() {
        let pool = WorkerPool::new();
        pool.register(WorkerInfo::new(AgentId::new(), vec![]));
        pool.register(WorkerInfo::new(AgentId::new(), vec![]));

        let id = AgentId::new();
        pool.register(WorkerInfo::new(id.clone(), vec![]));
        pool.assign_task(&id, uuid::Uuid::new_v4());

        let monitor = HealthMonitor::new(pool, 30_000, 10_000);
        let stats = monitor.stats();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.idle, 2);
        assert_eq!(stats.busy, 1);
        assert_eq!(stats.dead, 0);
    }
}
