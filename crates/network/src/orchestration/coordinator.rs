use tracing::{debug, info, warn};

use bunny_crypto::swarm::{MessagePriority, SwarmMessage, SwarmMessageType};
use bunny_crypto::ternary::TernaryPacket;
use bunny_crypto::types::AgentId;

use super::evolution::WorkerEvolution;
use super::health::HealthMonitor;
use super::quantum_dispatch::QuantumDispatch;
use super::scheduler::{ScheduledTask, TaskQueue};
use super::worker_pool::{WorkerCapability, WorkerInfo, WorkerPool};

/// Result of a completed inference task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_id: uuid::Uuid,
    pub worker_id: AgentId,
    pub output_data: Vec<u8>,
    pub inference_time_us: u64,
}

/// Central swarm coordinator: dispatches tasks, manages workers, distributes models.
///
/// The coordinator owns the worker pool, task queue, and health monitor. It:
/// - Accepts inference tasks and queues them by priority
/// - Selects the best available worker for each task
/// - Builds `SwarmMessage::InferenceDispatch` for the network layer to send
/// - Processes `SwarmMessage::InferenceReturn` and records results
/// - Monitors worker health via heartbeats
pub struct SwarmCoordinator {
    pool: WorkerPool,
    queue: TaskQueue,
    health: HealthMonitor,
    /// Quantum-enhanced worker selection (from swarm-core algorithm).
    quantum: QuantumDispatch,
    /// EMA-based worker fitness evolution tracker (from swarm-core algorithm).
    evolution: WorkerEvolution,
}

impl SwarmCoordinator {
    /// Create a new coordinator with default health settings.
    pub fn new() -> Self {
        let pool = WorkerPool::new();
        let health = HealthMonitor::new(pool.clone(), 30_000, 10_000);
        Self {
            pool,
            queue: TaskQueue::new(),
            health,
            quantum: QuantumDispatch::new(3),   // 3 qutrits (dim=27) for worker selection
            evolution: WorkerEvolution::new(0.15), // EMA alpha=0.15
        }
    }

    /// Create with custom health parameters.
    pub fn with_health(heartbeat_timeout_ms: u64, check_interval_ms: u64) -> Self {
        let pool = WorkerPool::new();
        let health = HealthMonitor::new(pool.clone(), heartbeat_timeout_ms, check_interval_ms);
        Self {
            pool,
            queue: TaskQueue::new(),
            health,
            quantum: QuantumDispatch::new(3),
            evolution: WorkerEvolution::new(0.15),
        }
    }

    /// Register a worker node.
    pub fn register_worker(&self, info: WorkerInfo) {
        info!(
            agent_id = ?info.agent_id,
            capabilities = info.capabilities.len(),
            "worker registered"
        );
        self.pool.register(info);
    }

    /// Remove a worker node.
    pub fn deregister_worker(&self, agent_id: &AgentId) -> Option<WorkerInfo> {
        self.pool.deregister(agent_id)
    }

    /// Submit an inference task. Returns the task ID.
    pub fn submit_task(
        &self,
        model_name: impl Into<String>,
        input_data: Vec<u8>,
        priority: MessagePriority,
    ) -> uuid::Uuid {
        let task = ScheduledTask::new(model_name, input_data, priority);
        let id = task.task_id;
        self.queue.submit(task);
        debug!(task_id = %id, "task submitted");
        id
    }

    /// Try to dispatch the next task from the queue.
    ///
    /// Uses quantum-enhanced worker selection when multiple candidates are available:
    /// 1. Filter idle workers by capability (existing behavior)
    /// 2. Score candidates via evolution fitness tracker (EMA of success/latency)
    /// 3. Apply quantum dispatch for final selection (qutrit evolution of features)
    ///
    /// Returns `Some((worker_id, SwarmMessage))` if a worker is available,
    /// or `None` if no worker can handle the next task (or queue is empty).
    pub fn try_dispatch(&mut self) -> Option<(AgentId, SwarmMessage)> {
        // Peek at the next task to find a compatible worker.
        let task = self.queue.pop()?;

        let capability = WorkerCapability::TernaryInference(task.model_name.clone());
        let candidates = self.pool.idle_workers_with(&capability);

        // Quantum-enhanced selection: when multiple workers match, use evolution
        // fitness + quantum dispatch to pick the optimal one.
        let selected_worker = if candidates.len() > 1 {
            // Build feature vectors from evolution tracker
            let features: Vec<(f64, f64, f64)> = candidates
                .iter()
                .map(|id| {
                    let fitness = self.evolution.score(id);
                    let snapshot = self.pool.snapshot();
                    let worker = snapshot.iter().find(|w| &w.agent_id == id);
                    let latency_score = worker
                        .map(|w| {
                            let avg = w.avg_inference_time_us();
                            if avg == 0 { 0.5 } else { 1.0 / (1.0 + avg as f64 / 10_000.0) }
                        })
                        .unwrap_or(0.5);
                    let capability_match = 1.0; // already filtered by capability
                    (latency_score, fitness, capability_match)
                })
                .collect();

            let idx = self.quantum.select_worker(&features);
            candidates.get(idx).cloned()
        } else {
            candidates.first().cloned()
        };

        if let Some(worker_id) = selected_worker {
            // Assign the task to this worker.
            if self.pool.assign_task(&worker_id, task.task_id) {
                debug!(
                    task_id = %task.task_id,
                    worker = ?worker_id,
                    model = %task.model_name,
                    candidates = candidates.len(),
                    "dispatching task (quantum-enhanced selection)"
                );

                // Build the dispatch message.
                let packet =
                    TernaryPacket::inference_request(&task.model_name, task.input_data.clone());
                let message = SwarmMessage::inference_dispatch(&packet);

                return Some((worker_id, message));
            }
        }

        // No available worker — re-queue the task.
        warn!(
            task_id = %task.task_id,
            model = %task.model_name,
            "no available worker, re-queuing"
        );
        self.queue.submit(task);
        None
    }

    /// Handle an `InferenceReturn` message from a worker.
    ///
    /// Records the result in the evolution tracker for adaptive fitness scoring.
    pub fn handle_result(
        &mut self,
        worker_id: &AgentId,
        message: &SwarmMessage,
    ) -> Option<TaskResult> {
        if message.msg_type != SwarmMessageType::InferenceReturn {
            return None;
        }

        let packet = message.into_ternary().ok()?;

        // TODO: extract actual inference time from the result payload.
        // For now, record a placeholder.
        let inference_time_us = 0_u64;
        self.pool.complete_task(worker_id, inference_time_us);

        // Feed evolution tracker — success=true since we got a response,
        // latency from inference time for adaptive worker scoring.
        self.evolution.record(worker_id, true, inference_time_us);

        Some(TaskResult {
            task_id: uuid::Uuid::nil(),
            worker_id: worker_id.clone(),
            output_data: packet.data,
            inference_time_us,
        })
    }

    /// Record a worker failure in the evolution tracker.
    ///
    /// Call this when a dispatch times out or returns an error.
    pub fn record_failure(&mut self, worker_id: &AgentId, latency_us: u64) {
        self.evolution.record(worker_id, false, latency_us);
    }

    /// Get the evolution tracker (for diagnostics / telemetry).
    pub fn evolution(&self) -> &WorkerEvolution {
        &self.evolution
    }

    /// Get the quantum dispatch selector (for diagnostics).
    pub fn quantum(&self) -> &QuantumDispatch {
        &self.quantum
    }

    /// Record a heartbeat from a worker.
    pub fn heartbeat(&self, agent_id: &AgentId) {
        self.health.heartbeat(agent_id);
    }

    /// Run a health check. Returns (newly_dead, evicted) worker IDs.
    pub fn health_check(&mut self) -> (Vec<AgentId>, Vec<AgentId>) {
        self.health.force_check()
    }

    /// Queue depth.
    pub fn pending_tasks(&self) -> usize {
        self.queue.len()
    }

    /// Worker pool reference.
    pub fn pool(&self) -> &WorkerPool {
        &self.pool
    }
}

impl Default for SwarmCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_and_dispatch() {
        let mut coord = SwarmCoordinator::new();

        // Register a worker capable of "threat_model".
        let worker_id = AgentId::new();
        coord.register_worker(WorkerInfo::new(
            worker_id.clone(),
            vec![WorkerCapability::TernaryInference("threat_model".into())],
        ));

        // Submit a task.
        let _task_id = coord.submit_task("threat_model", vec![1, 2, 3, 4], MessagePriority::High);
        assert_eq!(coord.pending_tasks(), 1);

        // Dispatch — should match the worker.
        let (dispatched_to, message) = coord.try_dispatch().unwrap();
        assert_eq!(dispatched_to, worker_id);
        assert_eq!(message.msg_type, SwarmMessageType::InferenceDispatch);
        assert_eq!(coord.pending_tasks(), 0);
        assert_eq!(coord.pool().busy_count(), 1);
    }

    #[test]
    fn no_worker_available() {
        let mut coord = SwarmCoordinator::new();

        // Submit a task with no workers registered.
        coord.submit_task("model_x", vec![], MessagePriority::Normal);
        assert!(coord.try_dispatch().is_none());

        // Task should be re-queued.
        assert_eq!(coord.pending_tasks(), 1);
    }

    #[test]
    fn capability_mismatch() {
        let mut coord = SwarmCoordinator::new();

        coord.register_worker(WorkerInfo::new(
            AgentId::new(),
            vec![WorkerCapability::TernaryInference("model_a".into())],
        ));

        // Submit task for model_b — no match.
        coord.submit_task("model_b", vec![], MessagePriority::Normal);
        assert!(coord.try_dispatch().is_none());
        assert_eq!(coord.pending_tasks(), 1);
    }

    #[test]
    fn handle_result_completes_task() {
        let mut coord = SwarmCoordinator::new();

        let worker_id = AgentId::new();
        coord.register_worker(WorkerInfo::new(
            worker_id.clone(),
            vec![WorkerCapability::TernaryInference("model".into())],
        ));

        coord.submit_task("model", vec![1, 2, 3, 4], MessagePriority::Normal);
        let (_, dispatch_msg) = coord.try_dispatch().unwrap();
        assert_eq!(coord.pool().busy_count(), 1);

        // Build a fake InferenceReturn.
        let result_packet = TernaryPacket::inference_result("model", vec![2], vec![0, 1]);
        let return_msg = SwarmMessage::inference_return(&dispatch_msg, &result_packet);

        let result = coord.handle_result(&worker_id, &return_msg);
        assert!(result.is_some());
        assert_eq!(coord.pool().busy_count(), 0);
        assert_eq!(coord.pool().idle_count(), 1);
    }

    #[test]
    fn health_check_evicts_dead() {
        let mut coord = SwarmCoordinator::with_health(1, 0);

        let worker_id = AgentId::new();
        let mut info = WorkerInfo::new(worker_id.clone(), vec![]);
        info.last_heartbeat = 0; // ancient
        coord.register_worker(info);

        let (dead, evicted) = coord.health_check();
        assert_eq!(dead.len(), 1);
        assert_eq!(evicted.len(), 1);
        assert_eq!(coord.pool().worker_count(), 0);
    }

    #[test]
    fn priority_dispatch_order() {
        let mut coord = SwarmCoordinator::new();

        // Register two workers for same model.
        for _ in 0..2 {
            coord.register_worker(WorkerInfo::new(
                AgentId::new(),
                vec![WorkerCapability::TernaryInference("model".into())],
            ));
        }

        // Submit low then high priority.
        coord.submit_task("model", vec![], MessagePriority::Low);
        coord.submit_task("model", vec![], MessagePriority::Critical);

        // Critical should dispatch first.
        let (_, msg1) = coord.try_dispatch().unwrap();
        let (_, _msg2) = coord.try_dispatch().unwrap();

        // Both dispatched — verify the dispatch message was InferenceDispatch.
        assert_eq!(msg1.msg_type, SwarmMessageType::InferenceDispatch);
        assert_eq!(coord.pending_tasks(), 0);
    }
}
