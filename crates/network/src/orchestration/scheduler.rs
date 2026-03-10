use std::collections::BinaryHeap;
use std::cmp::Ordering;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use bunny_crypto::swarm::MessagePriority;
use bunny_crypto::types::AgentId;

/// A task queued for dispatch to a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub task_id: uuid::Uuid,
    pub model_name: String,
    pub input_data: Vec<u8>,
    pub priority: MessagePriority,
    pub submitted_at: u64,
    /// Which worker was assigned (set after dispatch).
    pub assigned_to: Option<AgentId>,
}

impl ScheduledTask {
    pub fn new(
        model_name: impl Into<String>,
        input_data: Vec<u8>,
        priority: MessagePriority,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        Self {
            task_id: uuid::Uuid::new_v4(),
            model_name: model_name.into(),
            input_data,
            priority,
            submitted_at: now,
            assigned_to: None,
        }
    }
}

/// Wrapper for priority queue ordering.
///
/// Higher priority first, then FIFO by submission time.
#[derive(Debug)]
struct PrioritizedTask {
    task: ScheduledTask,
}

impl PartialEq for PrioritizedTask {
    fn eq(&self, other: &Self) -> bool {
        self.task.task_id == other.task.task_id
    }
}

impl Eq for PrioritizedTask {}

impl PartialOrd for PrioritizedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first.
        let prio = (self.task.priority as u8).cmp(&(other.task.priority as u8));
        if prio != Ordering::Equal {
            return prio;
        }
        // Earlier submission first (reverse ordering for min-heap behavior).
        other.task.submitted_at.cmp(&self.task.submitted_at)
    }
}

/// Priority-ordered task queue.
///
/// Thread-safe via `Mutex<BinaryHeap>`. Tasks are dequeued in priority order,
/// with FIFO ordering within the same priority level.
pub struct TaskQueue {
    queue: Mutex<BinaryHeap<PrioritizedTask>>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Submit a new task. Returns the task ID.
    pub fn submit(&self, task: ScheduledTask) -> uuid::Uuid {
        let id = task.task_id;
        let mut q = self.queue.lock().unwrap();
        q.push(PrioritizedTask { task });
        id
    }

    /// Take the highest-priority task from the queue.
    pub fn pop(&self) -> Option<ScheduledTask> {
        let mut q = self.queue.lock().unwrap();
        q.pop().map(|pt| pt.task)
    }

    /// Peek at the highest-priority task without removing it.
    pub fn peek_priority(&self) -> Option<MessagePriority> {
        let q = self.queue.lock().unwrap();
        q.peek().map(|pt| pt.task.priority)
    }

    /// Number of tasks in the queue.
    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }

    /// Drain all tasks from the queue.
    pub fn drain(&self) -> Vec<ScheduledTask> {
        let mut q = self.queue.lock().unwrap();
        let mut tasks = Vec::with_capacity(q.len());
        while let Some(pt) = q.pop() {
            tasks.push(pt.task);
        }
        tasks
    }
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_within_same_priority() {
        let queue = TaskQueue::new();
        let t1 = ScheduledTask::new("model_a", vec![], MessagePriority::Normal);
        let t2 = ScheduledTask::new("model_b", vec![], MessagePriority::Normal);
        let id1 = t1.task_id;
        let id2 = t2.task_id;

        queue.submit(t1);
        queue.submit(t2);

        assert_eq!(queue.len(), 2);
        // First submitted should come out first (FIFO within same priority).
        let first = queue.pop().unwrap();
        assert_eq!(first.task_id, id1);
        let second = queue.pop().unwrap();
        assert_eq!(second.task_id, id2);
    }

    #[test]
    fn priority_ordering() {
        let queue = TaskQueue::new();
        let low = ScheduledTask::new("low", vec![], MessagePriority::Low);
        let high = ScheduledTask::new("high", vec![], MessagePriority::High);
        let critical = ScheduledTask::new("critical", vec![], MessagePriority::Critical);

        queue.submit(low);
        queue.submit(high);
        queue.submit(critical);

        assert_eq!(queue.pop().unwrap().model_name, "critical");
        assert_eq!(queue.pop().unwrap().model_name, "high");
        assert_eq!(queue.pop().unwrap().model_name, "low");
    }

    #[test]
    fn empty_queue() {
        let queue = TaskQueue::new();
        assert!(queue.is_empty());
        assert!(queue.pop().is_none());
        assert!(queue.peek_priority().is_none());
    }

    #[test]
    fn drain() {
        let queue = TaskQueue::new();
        queue.submit(ScheduledTask::new("a", vec![], MessagePriority::Normal));
        queue.submit(ScheduledTask::new("b", vec![], MessagePriority::High));
        queue.submit(ScheduledTask::new("c", vec![], MessagePriority::Low));

        let tasks = queue.drain();
        assert_eq!(tasks.len(), 3);
        // Drain returns in priority order.
        assert_eq!(tasks[0].model_name, "b"); // High
        assert_eq!(tasks[1].model_name, "a"); // Normal
        assert_eq!(tasks[2].model_name, "c"); // Low
        assert!(queue.is_empty());
    }
}
