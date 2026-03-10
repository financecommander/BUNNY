use serde::{Deserialize, Serialize};

use crate::activation::Activation;
use crate::error::{Result, TritonError};
use crate::packing::{accumulate_word, PackedTernary};

/// A single ternary neural network layer.
///
/// Stores weights in 2-bit packed format (16 weights per u32).
/// Matrix multiplication uses addition/subtraction only (no FP multiply).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TernaryLayer {
    /// Layer name (e.g. "layer_0", "output").
    pub name: String,
    /// Input dimension.
    pub input_dim: usize,
    /// Output dimension.
    pub output_dim: usize,
    /// Packed weight matrix, stored row-major.
    /// `weights[row]` contains the packed weights for output neuron `row`.
    pub weights: Vec<PackedTernary>,
    /// Bias vector, one per output neuron.
    pub bias: Vec<f32>,
    /// Activation function.
    pub activation: Activation,
}

impl TernaryLayer {
    /// Create a new layer with the given dimensions and weights.
    pub fn new(
        name: String,
        input_dim: usize,
        output_dim: usize,
        weights: Vec<PackedTernary>,
        bias: Vec<f32>,
        activation: Activation,
    ) -> Result<Self> {
        if weights.len() != output_dim {
            return Err(TritonError::DimensionMismatch {
                expected: output_dim,
                got: weights.len(),
            });
        }
        if bias.len() != output_dim {
            return Err(TritonError::DimensionMismatch {
                expected: output_dim,
                got: bias.len(),
            });
        }
        for (i, w) in weights.iter().enumerate() {
            if w.len != input_dim {
                return Err(TritonError::DimensionMismatch {
                    expected: input_dim,
                    got: w.len,
                });
            }
            let expected_words = (input_dim + 15) / 16;
            if w.data.len() != expected_words {
                return Err(TritonError::InvalidFormat(format!(
                    "weight row {i}: expected {expected_words} packed words, got {}",
                    w.data.len()
                )));
            }
        }
        Ok(Self {
            name,
            input_dim,
            output_dim,
            weights,
            bias,
            activation,
        })
    }

    /// Fuse batch normalization parameters into the bias.
    ///
    /// Computes: `new_bias[i] = gamma[i] * (old_bias[i] - mean[i]) / sqrt(var[i] + eps) + beta[i]`
    ///
    /// After fusion, the BN is a no-op and can be removed from the inference path.
    pub fn fuse_batch_norm(
        &mut self,
        gamma: &[f32],
        beta: &[f32],
        mean: &[f32],
        var: &[f32],
        eps: f32,
    ) -> Result<()> {
        let n = self.output_dim;
        if gamma.len() != n || beta.len() != n || mean.len() != n || var.len() != n {
            return Err(TritonError::DimensionMismatch {
                expected: n,
                got: gamma.len().min(beta.len()).min(mean.len()).min(var.len()),
            });
        }

        for i in 0..n {
            let scale = gamma[i] / (var[i] + eps).sqrt();
            self.bias[i] = scale * (self.bias[i] - mean[i]) + beta[i];
        }

        Ok(())
    }

    /// Forward pass: packed ternary matmul with zero-skipping + bias + activation.
    ///
    /// For each output neuron, iterates over packed u32 words. Words that are
    /// all zeros (16 zero-weights) are skipped entirely. For non-zero words,
    /// +1 adds the input and -1 subtracts — no floating-point multiply.
    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>> {
        if input.len() != self.input_dim {
            return Err(TritonError::DimensionMismatch {
                expected: self.input_dim,
                got: input.len(),
            });
        }

        // Pad input to multiple of 16 for safe word-level access
        let padded_len = (self.input_dim + 15) / 16 * 16;
        let padded_input = if input.len() < padded_len {
            let mut p = vec![0.0f32; padded_len];
            p[..input.len()].copy_from_slice(input);
            p
        } else {
            input.to_vec()
        };

        let mut output = vec![0.0f32; self.output_dim];

        for (row, packed_row) in self.weights.iter().enumerate() {
            let mut acc = self.bias[row];

            for (word_idx, &word) in packed_row.data.iter().enumerate() {
                // Zero-skipping: skip entire word if all 16 weights are zero
                if word == 0 {
                    continue;
                }
                acc += accumulate_word(word, &padded_input, word_idx * 16);
            }

            output[row] = acc;
        }

        self.activation.apply(&mut output);
        Ok(output)
    }

    /// Count non-zero weights (sparsity metric).
    pub fn nonzero_count(&self) -> usize {
        let mut count = 0usize;
        for packed_row in &self.weights {
            for &word in &packed_row.data {
                if word == 0 {
                    continue;
                }
                let mut w = word;
                for _ in 0..16 {
                    if w & 0b11 != 0 {
                        count += 1;
                    }
                    w >>= 2;
                }
            }
        }
        count
    }

    /// Total weight count.
    pub fn weight_count(&self) -> usize {
        self.input_dim * self.output_dim
    }

    /// Sparsity ratio (0.0 = all nonzero, 1.0 = all zero).
    pub fn sparsity(&self) -> f32 {
        1.0 - (self.nonzero_count() as f32 / self.weight_count() as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packing::{pack, TernaryValue};

    fn make_identity_layer(dim: usize) -> TernaryLayer {
        // Identity-like: weight[i][j] = +1 if i==j, else 0
        let mut weights = Vec::new();
        for i in 0..dim {
            let mut row = vec![TernaryValue::Zero; dim];
            row[i] = TernaryValue::Pos;
            weights.push(pack(&row));
        }
        TernaryLayer::new(
            "identity".into(),
            dim,
            dim,
            weights,
            vec![0.0; dim],
            Activation::None,
        )
        .unwrap()
    }

    #[test]
    fn identity_forward() {
        let layer = make_identity_layer(4);
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let output = layer.forward(&input).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn negation_forward() {
        // All weights are -1 on diagonal
        let dim = 4;
        let mut weights = Vec::new();
        for i in 0..dim {
            let mut row = vec![TernaryValue::Zero; dim];
            row[i] = TernaryValue::Neg;
            weights.push(pack(&row));
        }
        let layer = TernaryLayer::new(
            "negate".into(),
            dim,
            dim,
            weights,
            vec![0.0; dim],
            Activation::None,
        )
        .unwrap();
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let output = layer.forward(&input).unwrap();
        assert_eq!(output, vec![-1.0, -2.0, -3.0, -4.0]);
    }

    #[test]
    fn bias_added() {
        let layer = TernaryLayer::new(
            "biased".into(),
            4,
            4,
            {
                let mut w = Vec::new();
                for _ in 0..4 {
                    w.push(pack(&vec![TernaryValue::Zero; 4]));
                }
                w
            },
            vec![1.0, 2.0, 3.0, 4.0],
            Activation::None,
        )
        .unwrap();
        let output = layer.forward(&[0.0; 4]).unwrap();
        assert_eq!(output, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn relu_activation() {
        let dim = 4;
        let mut weights = Vec::new();
        for i in 0..dim {
            let mut row = vec![TernaryValue::Zero; dim];
            row[i] = TernaryValue::Neg;
            weights.push(pack(&row));
        }
        let layer = TernaryLayer::new(
            "relu_test".into(),
            dim,
            dim,
            weights,
            vec![0.0; dim],
            Activation::ReLU,
        )
        .unwrap();
        let input = vec![1.0, -2.0, 3.0, -4.0];
        let output = layer.forward(&input).unwrap();
        // Negated: [-1, 2, -3, 4] → ReLU: [0, 2, 0, 4]
        assert_eq!(output, vec![0.0, 2.0, 0.0, 4.0]);
    }

    #[test]
    fn matmul_sum_row() {
        // Single output neuron: all +1 weights → sums all inputs
        let dim = 5;
        let weights = vec![pack(&vec![TernaryValue::Pos; dim])];
        let layer = TernaryLayer::new(
            "sum".into(),
            dim,
            1,
            weights,
            vec![0.0],
            Activation::None,
        )
        .unwrap();
        let input = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let output = layer.forward(&input).unwrap();
        assert!((output[0] - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_skipping_correctness() {
        // 32 inputs: first 16 all zero weights, second 16 all +1
        let dim = 32;
        let mut row_vals = vec![TernaryValue::Zero; 16];
        row_vals.extend(vec![TernaryValue::Pos; 16]);
        let weights = vec![pack(&row_vals)];
        let layer = TernaryLayer::new(
            "skip".into(),
            dim,
            1,
            weights,
            vec![0.0],
            Activation::None,
        )
        .unwrap();

        let mut input = vec![100.0; 16]; // first 16 should be skipped
        input.extend(vec![1.0; 16]); // second 16 summed
        let output = layer.forward(&input).unwrap();
        assert!((output[0] - 16.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sparsity_measurement() {
        let dim = 16;
        // All zeros
        let all_zero = TernaryLayer::new(
            "sparse".into(),
            dim,
            1,
            vec![pack(&vec![TernaryValue::Zero; dim])],
            vec![0.0],
            Activation::None,
        )
        .unwrap();
        assert!((all_zero.sparsity() - 1.0).abs() < f32::EPSILON);

        // All nonzero
        let all_pos = TernaryLayer::new(
            "dense".into(),
            dim,
            1,
            vec![pack(&vec![TernaryValue::Pos; dim])],
            vec![0.0],
            Activation::None,
        )
        .unwrap();
        assert!((all_pos.sparsity() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fuse_batch_norm() {
        let dim = 2;
        let mut layer = TernaryLayer::new(
            "bn".into(),
            dim,
            dim,
            vec![
                pack(&vec![TernaryValue::Pos, TernaryValue::Zero]),
                pack(&vec![TernaryValue::Zero, TernaryValue::Pos]),
            ],
            vec![1.0, 2.0], // original bias
            Activation::None,
        )
        .unwrap();

        let gamma = vec![1.0, 1.0];
        let beta = vec![0.0, 0.0];
        let mean = vec![1.0, 2.0]; // same as bias → (bias - mean) = 0
        let var = vec![1.0, 1.0];
        layer.fuse_batch_norm(&gamma, &beta, &mean, &var, 0.0).unwrap();

        // new_bias = gamma * (old_bias - mean) / sqrt(var + eps) + beta
        // = 1.0 * (1.0 - 1.0) / 1.0 + 0.0 = 0.0
        assert!((layer.bias[0] - 0.0).abs() < 1e-6);
        assert!((layer.bias[1] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn dimension_mismatch_errors() {
        let layer = make_identity_layer(4);
        assert!(layer.forward(&[1.0, 2.0, 3.0]).is_err());
        assert!(layer.forward(&[1.0, 2.0, 3.0, 4.0, 5.0]).is_err());
    }
}
