//! Quantum-enhanced worker selection for swarm task dispatch.
//!
//! Ports the core qutrit evolution algorithm from `swarm-core`'s QuantumEmulator
//! into native Rust for BUNNY's edge orchestration. Uses a lightweight 3-qutrit
//! state vector (dim=27) to break ties between equally-capable workers by evolving
//! task features through quantum gates and measuring the resulting probability
//! distribution.
//!
//! This is the same algorithm used by the Swarm Mainframe's Rust routing core,
//! adapted for BUNNY's worker-pool dispatch model.

use std::f64::consts::PI;

/// Lightweight qutrit-based quantum selector for worker dispatch.
///
/// Maps worker features (latency, task count, capability match) to qutrit
/// weights {-1, 0, +1}, evolves through Hadamard and phase gates, then
/// measures to select the optimal worker index.
pub struct QuantumDispatch {
    num_qutrits: usize,
    dim: usize,
    state: Vec<f64>, // real amplitudes (dim = 3^num_qutrits)
}

impl QuantumDispatch {
    /// Create a new quantum dispatch selector.
    ///
    /// `num_qutrits` controls the state space size (dim = 3^n).
    /// For worker selection, 3 qutrits (dim=27) is sufficient.
    pub fn new(num_qutrits: usize) -> Self {
        let dim = 3usize.pow(num_qutrits as u32);
        let mut state = vec![0.0; dim];
        state[0] = 1.0; // |000...0> ground state
        Self {
            num_qutrits,
            dim,
            state,
        }
    }

    /// Reset to ground state.
    pub fn reset(&mut self) {
        self.state.fill(0.0);
        self.state[0] = 1.0;
    }

    /// Apply qutrit Hadamard gate to qubit `target`.
    ///
    /// Creates equal superposition across all 3 basis states of the target qutrit,
    /// enabling quantum parallelism in worker evaluation.
    pub fn apply_hadamard(&mut self, target: usize) {
        let stride = 3usize.pow(target as u32);
        let block = stride * 3;
        let inv_sqrt3 = 1.0 / 3.0_f64.sqrt();
        let omega = 2.0 * PI / 3.0;

        let mut new_state = vec![0.0; self.dim];

        for block_start in (0..self.dim).step_by(block) {
            for offset in 0..stride {
                let i0 = block_start + offset;
                let i1 = i0 + stride;
                let i2 = i1 + stride;

                let a0 = self.state[i0];
                let a1 = self.state[i1];
                let a2 = self.state[i2];

                // 3x3 DFT matrix (real part for real amplitudes)
                new_state[i0] = inv_sqrt3 * (a0 + a1 + a2);
                new_state[i1] =
                    inv_sqrt3 * (a0 + a1 * omega.cos() + a2 * (2.0 * omega).cos());
                new_state[i2] =
                    inv_sqrt3 * (a0 + a1 * (2.0 * omega).cos() + a2 * omega.cos());
            }
        }

        self.state = new_state;
    }

    /// Apply phase gate to target qutrit, rotating |1> and |2> amplitudes.
    pub fn apply_phase(&mut self, target: usize, phase1: f64, phase2: f64) {
        let stride = 3usize.pow(target as u32);
        let block = stride * 3;

        for block_start in (0..self.dim).step_by(block) {
            for offset in 0..stride {
                let i1 = block_start + offset + stride;
                let i2 = i1 + stride;
                self.state[i1] *= phase1.cos();
                self.state[i2] *= phase2.cos();
            }
        }
    }

    /// Evolve worker feature weights through quantum gates.
    ///
    /// `weights` maps each qutrit to a ternary value {-1, 0, +1} derived from
    /// worker features (e.g., latency percentile, success rate, capability score).
    /// Depth controls the number of evolution layers.
    pub fn evolve_weights(&mut self, weights: &[i8], depth: usize) {
        self.reset();

        let n = weights.len().min(self.num_qutrits);

        for _layer in 0..depth {
            for i in 0..n {
                self.apply_hadamard(i);

                // Phase rotation proportional to weight
                let w = weights[i] as f64;
                self.apply_phase(i, w * PI / 3.0, w * 2.0 * PI / 3.0);
            }
        }
    }

    /// Measure the quantum state, returning probabilities for each basis state.
    ///
    /// The index with highest probability indicates the optimal worker selection
    /// from the quantum evolution of features.
    pub fn measure_probabilities(&self) -> Vec<f64> {
        let probs: Vec<f64> = self.state.iter().map(|a| a * a).collect();
        let total: f64 = probs.iter().sum();
        if total > 0.0 {
            probs.iter().map(|p| p / total).collect()
        } else {
            vec![1.0 / self.dim as f64; self.dim]
        }
    }

    /// Select the optimal worker index from a candidate list.
    ///
    /// Uses a composite fitness score with quantum tie-breaking:
    /// 1. Compute weighted composite score per candidate
    /// 2. If top candidates are within 5% of each other, use quantum evolution
    ///    to break the tie via qutrit probability measurement
    /// 3. Otherwise, select the highest-scoring candidate directly
    pub fn select_worker(&mut self, candidate_features: &[(f64, f64, f64)]) -> usize {
        if candidate_features.is_empty() {
            return 0;
        }
        if candidate_features.len() == 1 {
            return 0;
        }

        // Compute composite scores: 40% latency + 40% success + 20% capability
        let mut scored: Vec<(usize, f64)> = candidate_features
            .iter()
            .enumerate()
            .map(|(idx, (latency, success, capability))| {
                let score = 0.4 * latency + 0.4 * success + 0.2 * capability;
                (idx, score)
            })
            .collect();

        // Sort descending by score
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let best_score = scored[0].1;
        let threshold = best_score * 0.95; // 5% tie zone

        // Find candidates within the tie zone
        let tied: Vec<usize> = scored
            .iter()
            .filter(|(_, s)| *s >= threshold)
            .map(|(idx, _)| *idx)
            .collect();

        if tied.len() <= 1 {
            return scored[0].0;
        }

        // Quantum tie-breaking: evolve features, measure, select by probability
        let mut best_idx = tied[0];
        let mut best_quantum_score = f64::NEG_INFINITY;

        for &idx in &tied {
            let (lat, suc, cap) = candidate_features[idx];
            let weights = [
                feature_to_ternary(lat),
                feature_to_ternary(suc),
                feature_to_ternary(cap),
            ];
            self.evolve_weights(&weights, 2);
            let probs = self.measure_probabilities();

            // Quantum score: weighted sum using feature-aligned basis indices
            let quantum_score: f64 = probs
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let basis_weight = (i % 3) as f64; // 0, 1, 2
                    p * basis_weight
                })
                .sum::<f64>()
                + scored.iter().find(|(i, _)| *i == idx).unwrap().1; // add classical score

            if quantum_score > best_quantum_score {
                best_quantum_score = quantum_score;
                best_idx = idx;
            }
        }

        best_idx
    }
}

/// Map a [0.0, 1.0] feature score to ternary {-1, 0, +1}.
fn feature_to_ternary(score: f64) -> i8 {
    if score > 0.66 {
        1
    } else if score < 0.33 {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_state() {
        let qd = QuantumDispatch::new(3);
        assert_eq!(qd.dim, 27);
        assert!((qd.state[0] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn hadamard_creates_superposition() {
        let mut qd = QuantumDispatch::new(1);
        qd.apply_hadamard(0);
        // After Hadamard, all 3 amplitudes should be non-zero
        for i in 0..3 {
            assert!(qd.state[i].abs() > 0.1);
        }
    }

    #[test]
    fn select_best_worker() {
        let mut qd = QuantumDispatch::new(3);
        let features = vec![
            (0.2, 0.3, 0.5), // poor
            (0.9, 0.95, 1.0), // excellent
            (0.5, 0.5, 0.5), // average
        ];
        let selected = qd.select_worker(&features);
        assert_eq!(selected, 1); // should pick the best
    }

    #[test]
    fn single_candidate() {
        let mut qd = QuantumDispatch::new(3);
        assert_eq!(qd.select_worker(&[(0.5, 0.5, 0.5)]), 0);
    }

    #[test]
    fn empty_candidates() {
        let mut qd = QuantumDispatch::new(3);
        assert_eq!(qd.select_worker(&[]), 0);
    }

    #[test]
    fn evolve_and_measure() {
        let mut qd = QuantumDispatch::new(3);
        qd.evolve_weights(&[1, -1, 0], 3);
        let probs = qd.measure_probabilities();
        let total: f64 = probs.iter().sum();
        assert!((total - 1.0).abs() < 1e-10);
    }
}
