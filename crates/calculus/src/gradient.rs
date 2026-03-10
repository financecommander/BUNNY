//! Ternary-aware gradient estimation using the Straight-Through Estimator (STE).
//!
//! In ternary networks, the quantization function Q(x) = sign(x) has zero gradient
//! almost everywhere. The STE approximates ∂Q/∂x ≈ 1 when |x| ≤ 1, enabling
//! gradient-based updates for federated learning and model evolution.

use bunny_triton::packing::{pack, unpack, PackedTernary, TernaryValue};

use crate::error::{CalculusError, Result};

/// Straight-Through Estimator (STE) for ternary quantization.
///
/// Given a ternary weight and its upstream gradient (f32), returns the
/// STE gradient: passes the gradient through unchanged if the pre-quantized
/// value was within the linear region [-1, +1].
///
/// For ternary weights we don't have the original f32 value, so we use the
/// ternary value itself as the surrogate: STE = grad * indicator(|w| <= 1).
/// Since all ternary values are in {-1, 0, +1}, the indicator is always 1.
pub fn ste_gradient(weights: &PackedTernary, upstream_grad: &[f32]) -> Result<Vec<f32>> {
    if weights.len != upstream_grad.len() {
        return Err(CalculusError::DimensionMismatch {
            expected: weights.len,
            got: upstream_grad.len(),
        });
    }

    // For ternary values in {-1, 0, +1}, all are within [-1, +1],
    // so the STE gradient equals the upstream gradient directly.
    Ok(upstream_grad.to_vec())
}

/// Apply gradient to ternary weights using threshold-based requantization.
///
/// Accumulates f32 gradients onto ternary weights by:
/// 1. Converting ternary to f32 surrogate values
/// 2. Applying gradient step: w_new = w - lr * grad
/// 3. Requantizing to ternary using the threshold
///
/// Returns the updated ternary weights.
pub fn apply_gradient(
    weights: &PackedTernary,
    gradients: &[f32],
    learning_rate: f32,
    threshold: f32,
) -> Result<PackedTernary> {
    if weights.len != gradients.len() {
        return Err(CalculusError::DimensionMismatch {
            expected: weights.len,
            got: gradients.len(),
        });
    }

    let values = unpack(weights);
    let updated: Vec<TernaryValue> = values
        .iter()
        .zip(gradients.iter())
        .map(|(w, g)| {
            let w_f32 = (*w as i8) as f32;
            let new_val = w_f32 - learning_rate * g;
            if new_val > threshold {
                TernaryValue::Pos
            } else if new_val < -threshold {
                TernaryValue::Neg
            } else {
                TernaryValue::Zero
            }
        })
        .collect();

    Ok(pack(&updated))
}

/// Compute gradient magnitude (L2 norm of the gradient vector).
pub fn gradient_norm(gradients: &[f32]) -> f32 {
    gradients.iter().map(|g| g * g).sum::<f32>().sqrt()
}

/// Clip gradient by maximum norm (gradient clipping for stability).
pub fn clip_gradient(gradients: &mut [f32], max_norm: f32) {
    let norm = gradient_norm(gradients);
    if norm > max_norm {
        let scale = max_norm / norm;
        for g in gradients.iter_mut() {
            *g *= scale;
        }
    }
}

/// Compute the gradient direction alignment between two gradient vectors.
///
/// Returns cosine similarity of the gradients, useful for detecting
/// conflicting updates in federated learning.
pub fn gradient_alignment(a: &[f32], b: &[f32]) -> Result<f32> {
    if a.len() != b.len() {
        return Err(CalculusError::DimensionMismatch {
            expected: a.len(),
            got: b.len(),
        });
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = gradient_norm(a);
    let norm_b = gradient_norm(b);

    if norm_a == 0.0 || norm_b == 0.0 {
        return Ok(0.0);
    }

    Ok(dot / (norm_a * norm_b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ste_passes_through() {
        let w = pack(&[TernaryValue::Pos, TernaryValue::Zero, TernaryValue::Neg]);
        let grad = vec![0.5, -0.3, 0.8];
        let result = ste_gradient(&w, &grad).unwrap();
        assert_eq!(result, grad);
    }

    #[test]
    fn apply_gradient_basic() {
        let w = pack(&[TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Neg]);
        // Large positive gradient pushes weights negative
        let grad = vec![5.0, 5.0, -5.0];
        let updated = apply_gradient(&w, &grad, 1.0, 0.5).unwrap();
        let vals = unpack(&updated);
        // w=1 - 1.0*5.0 = -4.0 → Neg
        assert_eq!(vals[0], TernaryValue::Neg);
        // w=1 - 1.0*5.0 = -4.0 → Neg
        assert_eq!(vals[1], TernaryValue::Neg);
        // w=-1 - 1.0*(-5.0) = 4.0 → Pos
        assert_eq!(vals[2], TernaryValue::Pos);
    }

    #[test]
    fn gradient_clipping() {
        let mut grad = vec![3.0, 4.0]; // norm = 5.0
        clip_gradient(&mut grad, 2.5);
        let norm = gradient_norm(&grad);
        assert!((norm - 2.5).abs() < 1e-5);
    }

    #[test]
    fn gradient_alignment_parallel() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let align = gradient_alignment(&a, &b).unwrap();
        assert!((align - 1.0).abs() < 1e-5);
    }

    #[test]
    fn gradient_alignment_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let align = gradient_alignment(&a, &b).unwrap();
        assert!((align - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn dimension_mismatch() {
        let w = pack(&[TernaryValue::Pos]);
        let grad = vec![1.0, 2.0];
        assert!(ste_gradient(&w, &grad).is_err());
        assert!(apply_gradient(&w, &grad, 0.1, 0.5).is_err());
    }
}
