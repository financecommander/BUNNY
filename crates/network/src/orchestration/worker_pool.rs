use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use bunny_crypto::types::AgentId;

/// What a worker can do.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkerCapability {
    /// Can run ternary inference for a specific model.
    TernaryInference(String),
    /// Can accept model shard delivery.
    ShardStorage,
    /// Can forward traffic (relay role).
    Relay,
}

/// Current status of a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerStatus {
    /// Ready to accept tasks.
    Idle,
    /// Currently processing a task.
    Busy,
    /// Worker is draining (finishing current work, accepting no new tasks).
    Draining,
    /// Worker is unreachable (missed heartbeats).
    Dead,
}

/// Information about a registered worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub agent_id: AgentId,
    pub capabilities: Vec<WorkerCapability>,
    pub status: WorkerStatus,
    pub current_task: Option<uuid::Uuid>,
    pub tasks_completed: u64,
    pub total_inference_time_us: u64,
    pub last_heartbeat: u64,
    pub registered_at: u64,
}

impl WorkerInfo {
    pub fn new(agent_id: AgentId, capabilities: Vec<WorkerCapability>) -> Self {
        let now = now_ms();
        Self {
            agent_id,
            capabilities,
            status: WorkerStatus::Idle,
            current_task: None,
            tasks_completed: 0,
            total_inference_time_us: 0,
            last_heartbeat: now,
            registered_at: now,
        }
    }

    /// Average inference time in microseconds (0 if no tasks completed).
    pub fn avg_inference_time_us(&self) -> u64 {
        if self.tasks_completed == 0 {
            0
        } else {
            self.total_inference_time_us / self.tasks_completed
        }
    }

    /// Whether this worker supports a given capability.
    pub fn has_capability(&self, cap: &WorkerCapability) -> bool {
        self.capabilities.contains(cap)
    }
}

/// Thread-safe worker pool using DashMap for concurrent access.
///
/// Tracks available workers, their capabilities, and status. Used by the
/// `SwarmCoordinator` to select the best worker for each inference task.
#[derive(Clone)]
pub struct WorkerPool {
    workers: Arc<DashMap<AgentId, WorkerInfo>>,
}

impl WorkerPool {
    pub fn new() -> Self {
        Self {
            workers: Arc::new(DashMap::new()),
        }
    }

    /// Register a new worker.
    pub fn register(&self, info: WorkerInfo) {
        self.workers.insert(info.agent_id.clone(), info);
    }

    /// Remove a worker.
    pub fn deregister(&self, agent_id: &AgentId) -> Option<WorkerInfo> {
        self.workers.remove(agent_id).map(|(_, w)| w)
    }

    /// Update a worker's heartbeat timestamp.
    pub fn heartbeat(&self, agent_id: &AgentId) {
        if let Some(mut w) = self.workers.get_mut(agent_id) {
            w.last_heartbeat = now_ms();
            if w.status == WorkerStatus::Dead {
                w.status = WorkerStatus::Idle;
            }
        }
    }

    /// Mark a worker as busy with a specific task.
    pub fn assign_task(&self, agent_id: &AgentId, task_id: uuid::Uuid) -> bool {
        if let Some(mut w) = self.workers.get_mut(agent_id) {
            if w.status == WorkerStatus::Idle {
                w.status = WorkerStatus::Busy;
                w.current_task = Some(task_id);
                return true;
            }
        }
        false
    }

    /// Mark a worker's current task as complete.
    pub fn complete_task(&self, agent_id: &AgentId, inference_time_us: u64) {
        if let Some(mut w) = self.workers.get_mut(agent_id) {
            w.status = WorkerStatus::Idle;
            w.current_task = None;
            w.tasks_completed += 1;
            w.total_inference_time_us += inference_time_us;
        }
    }

    /// Find idle workers that support a given capability, sorted by avg latency.
    pub fn idle_workers_with(&self, capability: &WorkerCapability) -> Vec<AgentId> {
        let mut candidates: Vec<(AgentId, u64)> = self
            .workers
            .iter()
            .filter(|r| r.status == WorkerStatus::Idle && r.has_capability(capability))
            .map(|r| (r.agent_id.clone(), r.avg_inference_time_us()))
            .collect();

        // Sort by avg latency (fastest first).
        candidates.sort_by_key(|(_, latency)| *latency);
        candidates.into_iter().map(|(id, _)| id).collect()
    }

    /// Total number of registered workers.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Number of idle workers.
    pub fn idle_count(&self) -> usize {
        self.workers
            .iter()
            .filter(|r| r.status == WorkerStatus::Idle)
            .count()
    }

    /// Number of busy workers.
    pub fn busy_count(&self) -> usize {
        self.workers
            .iter()
            .filter(|r| r.status == WorkerStatus::Busy)
            .count()
    }

    /// Mark workers with stale heartbeats as dead. Returns dead worker IDs.
    pub fn mark_dead(&self, heartbeat_timeout_ms: u64) -> Vec<AgentId> {
        let now = now_ms();
        let mut dead = Vec::new();
        for mut entry in self.workers.iter_mut() {
            if entry.status != WorkerStatus::Dead
                && now.saturating_sub(entry.last_heartbeat) > heartbeat_timeout_ms
            {
                entry.status = WorkerStatus::Dead;
                entry.current_task = None;
                dead.push(entry.agent_id.clone());
            }
        }
        dead
    }

    /// Remove all dead workers. Returns removed worker IDs.
    pub fn evict_dead(&self) -> Vec<AgentId> {
        let mut evicted = Vec::new();
        self.workers.retain(|_, w| {
            if w.status == WorkerStatus::Dead {
                evicted.push(w.agent_id.clone());
                false
            } else {
                true
            }
        });
        evicted
    }

    /// Get a snapshot of all worker info.
    pub fn snapshot(&self) -> Vec<WorkerInfo> {
        self.workers.iter().map(|r| r.value().clone()).collect()
    }
}

impl Default for WorkerPool {
    fn default() -> Self {
        Self::new()
    }
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
    fn register_and_query() {
        let pool = WorkerPool::new();
        let id = AgentId::new();
        let info = WorkerInfo::new(
            id.clone(),
            vec![WorkerCapability::TernaryInference("model_a".into())],
        );
        pool.register(info);

        assert_eq!(pool.worker_count(), 1);
        assert_eq!(pool.idle_count(), 1);

        let cap = WorkerCapability::TernaryInference("model_a".into());
        let idle = pool.idle_workers_with(&cap);
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0], id);
    }

    #[test]
    fn assign_and_complete() {
        let pool = WorkerPool::new();
        let id = AgentId::new();
        pool.register(WorkerInfo::new(id.clone(), vec![]));

        let task_id = uuid::Uuid::new_v4();
        assert!(pool.assign_task(&id, task_id));
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.busy_count(), 1);

        // Can't double-assign.
        assert!(!pool.assign_task(&id, uuid::Uuid::new_v4()));

        pool.complete_task(&id, 500);
        assert_eq!(pool.idle_count(), 1);
        assert_eq!(pool.busy_count(), 0);
    }

    #[test]
    fn dead_detection() {
        let pool = WorkerPool::new();
        let id = AgentId::new();
        let mut info = WorkerInfo::new(id.clone(), vec![]);
        // Set heartbeat to 10 seconds ago.
        info.last_heartbeat = now_ms().saturating_sub(10_000);
        pool.register(info);

        // 5s timeout — worker should be marked dead.
        let dead = pool.mark_dead(5_000);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0], id);

        // Heartbeat revives.
        pool.heartbeat(&id);
        let dead = pool.mark_dead(5_000);
        assert!(dead.is_empty());
    }

    #[test]
    fn evict_dead() {
        let pool = WorkerPool::new();
        let id = AgentId::new();
        let mut info = WorkerInfo::new(id.clone(), vec![]);
        info.last_heartbeat = 0; // ancient
        pool.register(info);

        pool.mark_dead(1);
        let evicted = pool.evict_dead();
        assert_eq!(evicted.len(), 1);
        assert_eq!(pool.worker_count(), 0);
    }

    #[test]
    fn capability_filter() {
        let pool = WorkerPool::new();

        let id_a = AgentId::new();
        pool.register(WorkerInfo::new(
            id_a.clone(),
            vec![WorkerCapability::TernaryInference("model_a".into())],
        ));

        let id_b = AgentId::new();
        pool.register(WorkerInfo::new(
            id_b.clone(),
            vec![WorkerCapability::TernaryInference("model_b".into())],
        ));

        let cap_a = WorkerCapability::TernaryInference("model_a".into());
        let cap_b = WorkerCapability::TernaryInference("model_b".into());
        let cap_c = WorkerCapability::TernaryInference("model_c".into());

        assert_eq!(pool.idle_workers_with(&cap_a).len(), 1);
        assert_eq!(pool.idle_workers_with(&cap_b).len(), 1);
        assert_eq!(pool.idle_workers_with(&cap_c).len(), 0);
    }
}
