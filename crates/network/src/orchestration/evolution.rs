//! Worker fitness evolution tracker.
//!
//! Ports the exponential moving average (EMA) fitness tracking from `swarm-core`'s
//! EvolutionTracker into native Rust for BUNNY's worker pool. Tracks per-worker
//! fitness scores based on success rate, latency, and task throughput, enabling
//! adaptive worker selection that favors consistently high-performing nodes.

use std::collections::HashMap;

use bunny_crypto::types::AgentId;

/// Fitness record for a single worker.
#[derive(Debug, Clone)]
pub struct WorkerFitness {
    /// Exponential moving average of success rate [0.0, 1.0].
    pub success_ema: f64,
    /// Exponential moving average of normalized latency [0.0, 1.0] (lower = faster).
    pub latency_ema: f64,
    /// Total observations recorded.
    pub observations: u64,
    /// Total successful inferences.
    pub successes: u64,
    /// Total inference time in microseconds.
    pub total_time_us: u64,
}

impl WorkerFitness {
    fn new() -> Self {
        Self {
            success_ema: 0.5, // neutral prior
            latency_ema: 0.5,
            observations: 0,
            successes: 0,
            total_time_us: 0,
        }
    }

    /// Composite fitness score [0.0, 1.0]. Higher = better worker.
    ///
    /// Weighted: 40% success rate + 40% speed (inverted latency) + 20% experience.
    pub fn score(&self) -> f64 {
        let speed_score = 1.0 - self.latency_ema;
        let experience = (self.observations as f64 / 100.0).min(1.0);
        0.4 * self.success_ema + 0.4 * speed_score + 0.2 * experience
    }
}

/// Tracks per-worker fitness using EMA, enabling adaptive worker selection.
///
/// Workers with higher fitness scores are preferred for task dispatch. The tracker
/// uses exponential moving averages to weight recent performance more heavily than
/// historical data, allowing quick adaptation to changing worker conditions.
pub struct WorkerEvolution {
    alpha: f64,
    fitness: HashMap<AgentId, WorkerFitness>,
    /// Reference latency for normalization (microseconds).
    /// Updated as a running max.
    max_latency_us: u64,
}

impl WorkerEvolution {
    /// Create with EMA smoothing factor.
    ///
    /// `alpha` in (0, 1) — higher values weight recent observations more heavily.
    /// Typical: 0.15 for gradual adaptation, 0.3 for fast adaptation.
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.01, 0.99),
            fitness: HashMap::new(),
            max_latency_us: 1_000_000, // 1 second default ceiling
        }
    }

    /// Record an inference result for a worker.
    pub fn record(
        &mut self,
        worker_id: &AgentId,
        success: bool,
        inference_time_us: u64,
    ) {
        // Update max latency reference
        if inference_time_us > self.max_latency_us {
            self.max_latency_us = inference_time_us;
        }

        let entry = self
            .fitness
            .entry(worker_id.clone())
            .or_insert_with(WorkerFitness::new);

        let success_val = if success { 1.0 } else { 0.0 };
        let latency_norm = (inference_time_us as f64) / (self.max_latency_us as f64);

        entry.success_ema = self.alpha * success_val + (1.0 - self.alpha) * entry.success_ema;
        entry.latency_ema = self.alpha * latency_norm + (1.0 - self.alpha) * entry.latency_ema;
        entry.observations += 1;
        if success {
            entry.successes += 1;
        }
        entry.total_time_us += inference_time_us;
    }

    /// Get the fitness score for a worker. Returns 0.5 (neutral) if unknown.
    pub fn score(&self, worker_id: &AgentId) -> f64 {
        self.fitness
            .get(worker_id)
            .map(|f| f.score())
            .unwrap_or(0.5)
    }

    /// Get scores for multiple workers, sorted descending by fitness.
    pub fn ranked_scores(&self, worker_ids: &[AgentId]) -> Vec<(AgentId, f64)> {
        let mut scores: Vec<(AgentId, f64)> = worker_ids
            .iter()
            .map(|id| (id.clone(), self.score(id)))
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
    }

    /// Total tracked workers.
    pub fn tracked_count(&self) -> usize {
        self.fitness.len()
    }

    /// Total observations across all workers.
    pub fn total_observations(&self) -> u64 {
        self.fitness.values().map(|f| f.observations).sum()
    }

    /// Get fitness details for a worker.
    pub fn get_fitness(&self, worker_id: &AgentId) -> Option<&WorkerFitness> {
        self.fitness.get(worker_id)
    }
}

impl Default for WorkerEvolution {
    fn default() -> Self {
        Self::new(0.15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent() -> AgentId {
        AgentId::new()
    }

    #[test]
    fn new_worker_has_neutral_score() {
        let evo = WorkerEvolution::new(0.15);
        let id = test_agent();
        assert!((evo.score(&id) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn successful_worker_score_increases() {
        let mut evo = WorkerEvolution::new(0.3);
        let id = test_agent();

        for _ in 0..20 {
            evo.record(&id, true, 1_000); // fast + successful
        }

        let score = evo.score(&id);
        assert!(score > 0.7, "score should be high: {score}");
    }

    #[test]
    fn failing_worker_score_decreases() {
        let mut evo = WorkerEvolution::new(0.3);
        let id = test_agent();

        for _ in 0..20 {
            evo.record(&id, false, 500_000); // slow + failing
        }

        let score = evo.score(&id);
        assert!(score < 0.3, "score should be low: {score}");
    }

    #[test]
    fn ranked_scores_ordering() {
        let mut evo = WorkerEvolution::new(0.3);
        let good = test_agent();
        let bad = test_agent();

        for _ in 0..10 {
            evo.record(&good, true, 1_000);
            evo.record(&bad, false, 900_000);
        }

        let ranked = evo.ranked_scores(&[good.clone(), bad.clone()]);
        assert_eq!(ranked[0].0, good);
        assert!(ranked[0].1 > ranked[1].1);
    }

    #[test]
    fn total_observations() {
        let mut evo = WorkerEvolution::new(0.15);
        let id1 = test_agent();
        let id2 = test_agent();

        evo.record(&id1, true, 100);
        evo.record(&id1, true, 200);
        evo.record(&id2, false, 300);

        assert_eq!(evo.total_observations(), 3);
        assert_eq!(evo.tracked_count(), 2);
    }
}
