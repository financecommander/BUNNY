//! Quantization-aware optimization for ternary model compression.
//!
//! Tools for finding optimal quantization thresholds, measuring weight
//! distribution quality, and sparsity-targeted compression.

use bunny_triton::packing::{quantize_f32, PackedTernary, TernaryValue};

use crate::error::{CalculusError, Result};

/// Statistics about a ternary quantization result.
#[derive(Debug, Clone)]
pub struct QuantizationStats {
    /// Fraction of weights that are zero [0.0, 1.0].
    pub sparsity: f64,
    /// Fraction of weights that are +1.
    pub positive_ratio: f64,
    /// Fraction of weights that are -1.
    pub negative_ratio: f64,
    /// Mean squared error between original f32 and quantized ternary.
    pub mse: f64,
    /// Signal-to-quantization-noise ratio (dB).
    pub sqnr_db: f64,
    /// Total number of weights.
    pub total: usize,
}

/// Analyze quantization quality for a given threshold.
pub fn analyze_quantization(values: &[f32], threshold: f32) -> QuantizationStats {
    let total = values.len();
    if total == 0 {
        return QuantizationStats {
            sparsity: 1.0,
            positive_ratio: 0.0,
            negative_ratio: 0.0,
            mse: 0.0,
            sqnr_db: 0.0,
            total: 0,
        };
    }

    let quantized = quantize_f32(values, threshold);
    let mut zeros = 0usize;
    let mut pos = 0usize;
    let mut neg = 0usize;
    let mut mse_sum = 0.0f64;
    let mut signal_power = 0.0f64;

    for (orig, quant) in values.iter().zip(quantized.iter()) {
        match quant {
            TernaryValue::Zero => zeros += 1,
            TernaryValue::Pos => pos += 1,
            TernaryValue::Neg => neg += 1,
        }
        let q_f32 = (*quant as i8) as f64;
        let o_f64 = *orig as f64;
        mse_sum += (o_f64 - q_f32).powi(2);
        signal_power += o_f64.powi(2);
    }

    let mse = mse_sum / total as f64;
    let sqnr_db = if mse > 0.0 {
        10.0 * (signal_power / total as f64 / mse).log10()
    } else {
        f64::INFINITY
    };

    QuantizationStats {
        sparsity: zeros as f64 / total as f64,
        positive_ratio: pos as f64 / total as f64,
        negative_ratio: neg as f64 / total as f64,
        mse,
        sqnr_db,
        total,
    }
}

/// Search for the optimal quantization threshold that achieves a target sparsity.
///
/// Uses binary search on the threshold parameter to find the value that
/// produces the desired fraction of zero weights.
pub fn find_threshold_for_sparsity(
    values: &[f32],
    target_sparsity: f64,
    tolerance: f64,
    max_iterations: usize,
) -> Result<(f32, QuantizationStats)> {
    if values.is_empty() {
        return Err(CalculusError::EmptyInput);
    }
    if target_sparsity < 0.0 || target_sparsity > 1.0 {
        return Err(CalculusError::InvalidThreshold(format!(
            "target sparsity must be 0.0–1.0, got {}",
            target_sparsity
        )));
    }

    // Find max absolute value for search range.
    let max_abs = values
        .iter()
        .map(|v| v.abs())
        .fold(0.0f32, f32::max);

    let mut lo = 0.0f32;
    let mut hi = max_abs;

    for _ in 0..max_iterations {
        let mid = (lo + hi) / 2.0;
        let stats = analyze_quantization(values, mid);

        if (stats.sparsity - target_sparsity).abs() < tolerance {
            return Ok((mid, stats));
        }

        if stats.sparsity < target_sparsity {
            lo = mid; // need higher threshold → more zeros
        } else {
            hi = mid; // need lower threshold → fewer zeros
        }
    }

    // Return best effort after max iterations.
    let threshold = (lo + hi) / 2.0;
    let stats = analyze_quantization(values, threshold);
    Ok((threshold, stats))
}

/// Compute weight distribution entropy (bits).
///
/// Maximum entropy for ternary = log2(3) ≈ 1.585 bits.
pub fn weight_entropy(packed: &PackedTernary) -> f64 {
    let values = bunny_triton::packing::unpack(packed);
    let total = values.len() as f64;
    if total == 0.0 {
        return 0.0;
    }

    let mut counts = [0usize; 3]; // [neg, zero, pos]
    for v in &values {
        match v {
            TernaryValue::Neg => counts[0] += 1,
            TernaryValue::Zero => counts[1] += 1,
            TernaryValue::Pos => counts[2] += 1,
        }
    }

    let mut entropy = 0.0;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / total;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Check if a weight distribution is balanced (roughly equal ±1 counts).
///
/// Returns the balance ratio: |pos - neg| / (pos + neg). Lower = better.
/// 0.0 = perfectly balanced, 1.0 = completely one-sided.
pub fn weight_balance(packed: &PackedTernary) -> f64 {
    let values = bunny_triton::packing::unpack(packed);
    let mut pos = 0usize;
    let mut neg = 0usize;

    for v in &values {
        match v {
            TernaryValue::Pos => pos += 1,
            TernaryValue::Neg => neg += 1,
            _ => {}
        }
    }

    let total_nonzero = pos + neg;
    if total_nonzero == 0 {
        return 0.0;
    }

    (pos as f64 - neg as f64).abs() / total_nonzero as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_triton::packing::pack;

    #[test]
    fn analyze_basic() {
        let values = vec![0.8, -0.7, 0.1, -0.05, 0.9, -0.85, 0.0, 0.3];
        let stats = analyze_quantization(&values, 0.5);

        assert_eq!(stats.total, 8);
        assert!(stats.sparsity > 0.0); // some values below threshold
        assert!(stats.mse < 1.0); // reasonable error
    }

    #[test]
    fn threshold_search() {
        let values: Vec<f32> = (0..100).map(|i| (i as f32 - 50.0) / 50.0).collect();
        let (threshold, stats) = find_threshold_for_sparsity(&values, 0.5, 0.05, 50).unwrap();

        assert!(threshold > 0.0);
        assert!((stats.sparsity - 0.5).abs() < 0.1);
    }

    #[test]
    fn entropy_uniform() {
        // Equal distribution of all three values → max entropy
        let values: Vec<TernaryValue> = (0..30)
            .map(|i| match i % 3 {
                0 => TernaryValue::Neg,
                1 => TernaryValue::Zero,
                _ => TernaryValue::Pos,
            })
            .collect();
        let packed = pack(&values);
        let entropy = weight_entropy(&packed);
        // Max ternary entropy = log2(3) ≈ 1.585
        assert!((entropy - 1.585).abs() < 0.01);
    }

    #[test]
    fn balance_symmetric() {
        let values = vec![
            TernaryValue::Pos,
            TernaryValue::Neg,
            TernaryValue::Pos,
            TernaryValue::Neg,
        ];
        let packed = pack(&values);
        assert_eq!(weight_balance(&packed), 0.0);
    }

    #[test]
    fn balance_onesided() {
        let values = vec![TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Pos];
        let packed = pack(&values);
        assert_eq!(weight_balance(&packed), 1.0);
    }

    #[test]
    fn empty_analysis() {
        let stats = analyze_quantization(&[], 0.5);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.sparsity, 1.0);
    }
}
