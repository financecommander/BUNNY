use serde::{Deserialize, Serialize};

/// Activation function applied after each layer's matmul + bias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activation {
    None,
    ReLU,
    Sigmoid,
    Softmax,
}

impl Activation {
    /// Apply the activation function in-place.
    pub fn apply(&self, values: &mut [f32]) {
        match self {
            Activation::None => {}
            Activation::ReLU => {
                for v in values.iter_mut() {
                    *v = v.max(0.0);
                }
            }
            Activation::Sigmoid => {
                for v in values.iter_mut() {
                    *v = 1.0 / (1.0 + (-*v).exp());
                }
            }
            Activation::Softmax => {
                let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let mut sum = 0.0f32;
                for v in values.iter_mut() {
                    *v = (*v - max).exp();
                    sum += *v;
                }
                if sum > 0.0 {
                    for v in values.iter_mut() {
                        *v /= sum;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relu() {
        let mut vals = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        Activation::ReLU.apply(&mut vals);
        assert_eq!(vals, vec![0.0, 0.0, 0.0, 1.0, 2.0]);
    }

    #[test]
    fn sigmoid() {
        let mut vals = vec![0.0];
        Activation::Sigmoid.apply(&mut vals);
        assert!((vals[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn softmax_sums_to_one() {
        let mut vals = vec![1.0, 2.0, 3.0];
        Activation::Softmax.apply(&mut vals);
        let sum: f32 = vals.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
        // Largest input should have largest probability
        assert!(vals[2] > vals[1]);
        assert!(vals[1] > vals[0]);
    }

    #[test]
    fn none_passthrough() {
        let mut vals = vec![-1.0, 0.0, 1.0];
        Activation::None.apply(&mut vals);
        assert_eq!(vals, vec![-1.0, 0.0, 1.0]);
    }
}
