//! Running statistics and anomaly detection for swarm telemetry.
//!
//! These tools power the ThreatHunter agent's anomaly detection pipeline,
//! computing z-scores, IQR outliers, and exponential moving averages on
//! streaming data without storing full history.

use crate::error::{CalculusError, Result};

/// Running mean and variance tracker (Welford's online algorithm).
///
/// Memory-constant: stores only count, mean, and M2 — no sample history.
#[derive(Debug, Clone)]
pub struct RunningStats {
    count: u64,
    mean: f64,
    m2: f64,
    min: f64,
    max: f64,
}

impl RunningStats {
    pub fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }

    /// Add a new observation.
    pub fn push(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;

        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
    }

    /// Number of observations.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Current mean.
    pub fn mean(&self) -> f64 {
        self.mean
    }

    /// Sample variance.
    pub fn variance(&self) -> f64 {
        if self.count < 2 {
            0.0
        } else {
            self.m2 / (self.count - 1) as f64
        }
    }

    /// Sample standard deviation.
    pub fn stddev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Min observed value.
    pub fn min(&self) -> f64 {
        self.min
    }

    /// Max observed value.
    pub fn max(&self) -> f64 {
        self.max
    }

    /// Z-score of a value relative to this distribution.
    ///
    /// Returns 0.0 if stddev is 0 (all values identical).
    pub fn z_score(&self, value: f64) -> f64 {
        let sd = self.stddev();
        if sd == 0.0 {
            0.0
        } else {
            (value - self.mean) / sd
        }
    }

    /// Whether a value is anomalous (|z-score| > threshold).
    ///
    /// Default threshold: 3.0 (99.7% of normal data falls within ±3σ).
    pub fn is_anomalous(&self, value: f64, threshold: f64) -> bool {
        self.count >= 10 && self.z_score(value).abs() > threshold
    }
}

impl Default for RunningStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Exponential moving average (EMA) tracker.
///
/// Weight recent observations more heavily. Used for latency smoothing,
/// throughput estimation, and adaptive thresholds.
#[derive(Debug, Clone)]
pub struct Ema {
    alpha: f64,
    value: f64,
    initialized: bool,
}

impl Ema {
    /// Create a new EMA with smoothing factor `alpha` ∈ (0, 1).
    ///
    /// Higher alpha = more responsive to recent values.
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.01, 0.99),
            value: 0.0,
            initialized: false,
        }
    }

    /// Update with a new observation.
    pub fn update(&mut self, observation: f64) {
        if !self.initialized {
            self.value = observation;
            self.initialized = true;
        } else {
            self.value = self.alpha * observation + (1.0 - self.alpha) * self.value;
        }
    }

    /// Current EMA value.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Whether the EMA has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

/// Batch statistics on a slice of f64 values.
pub fn batch_mean(values: &[f64]) -> Result<f64> {
    if values.is_empty() {
        return Err(CalculusError::EmptyInput);
    }
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}

pub fn batch_variance(values: &[f64]) -> Result<f64> {
    if values.len() < 2 {
        return Err(CalculusError::InsufficientData {
            needed: 2,
            have: values.len(),
        });
    }
    let mean = batch_mean(values)?;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Ok(var)
}

pub fn batch_stddev(values: &[f64]) -> Result<f64> {
    Ok(batch_variance(values)?.sqrt())
}

/// Compute percentile (0–100) using linear interpolation.
pub fn percentile(sorted_values: &[f64], p: f64) -> Result<f64> {
    if sorted_values.is_empty() {
        return Err(CalculusError::EmptyInput);
    }
    if p < 0.0 || p > 100.0 {
        return Err(CalculusError::InvalidThreshold(format!(
            "percentile must be 0–100, got {}",
            p
        )));
    }

    let n = sorted_values.len();
    if n == 1 {
        return Ok(sorted_values[0]);
    }

    let rank = p / 100.0 * (n - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let frac = rank - lower as f64;

    Ok(sorted_values[lower] * (1.0 - frac) + sorted_values[upper.min(n - 1)] * frac)
}

/// Interquartile range (IQR) for outlier detection.
pub fn iqr(sorted_values: &[f64]) -> Result<(f64, f64, f64)> {
    let q1 = percentile(sorted_values, 25.0)?;
    let q3 = percentile(sorted_values, 75.0)?;
    let iqr = q3 - q1;
    Ok((q1, q3, iqr))
}

/// Detect outliers using the IQR method (1.5 × IQR rule).
pub fn iqr_outliers(sorted_values: &[f64]) -> Result<Vec<f64>> {
    let (q1, q3, iqr_val) = iqr(sorted_values)?;
    let lower_fence = q1 - 1.5 * iqr_val;
    let upper_fence = q3 + 1.5 * iqr_val;

    Ok(sorted_values
        .iter()
        .filter(|v| **v < lower_fence || **v > upper_fence)
        .copied()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_stats_basic() {
        let mut stats = RunningStats::new();
        for v in &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            stats.push(*v);
        }
        assert_eq!(stats.count(), 8);
        assert_eq!(stats.mean(), 5.0);
        assert!((stats.variance() - 4.571428571428571).abs() < 1e-10);
        assert_eq!(stats.min(), 2.0);
        assert_eq!(stats.max(), 9.0);
    }

    #[test]
    fn z_score_anomaly() {
        let mut stats = RunningStats::new();
        for i in 0..100 {
            stats.push(50.0 + (i % 5) as f64); // values 50–54
        }
        // 50 is normal, 100 is anomalous
        assert!(!stats.is_anomalous(52.0, 3.0));
        assert!(stats.is_anomalous(100.0, 3.0));
    }

    #[test]
    fn ema_tracks_trend() {
        let mut ema = Ema::new(0.3);
        assert!(!ema.is_initialized());

        ema.update(10.0);
        assert!(ema.is_initialized());
        assert_eq!(ema.value(), 10.0); // first value

        ema.update(20.0);
        // 0.3*20 + 0.7*10 = 6 + 7 = 13
        assert!((ema.value() - 13.0).abs() < f64::EPSILON);
    }

    #[test]
    fn batch_stats() {
        let vals = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert_eq!(batch_mean(&vals).unwrap(), 5.0);
        assert!((batch_variance(&vals).unwrap() - 4.571428571428571).abs() < 1e-10);
    }

    #[test]
    fn percentile_computation() {
        let vals = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&vals, 0.0).unwrap(), 1.0);
        assert_eq!(percentile(&vals, 50.0).unwrap(), 3.0);
        assert_eq!(percentile(&vals, 100.0).unwrap(), 5.0);
    }

    #[test]
    fn iqr_outlier_detection() {
        let mut vals = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 100.0];
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let outliers = iqr_outliers(&vals).unwrap();
        assert!(!outliers.is_empty());
        assert!(outliers.contains(&100.0));
    }

    #[test]
    fn empty_input_errors() {
        assert!(batch_mean(&[]).is_err());
        assert!(batch_variance(&[1.0]).is_err());
        assert!(percentile(&[], 50.0).is_err());
    }
}
