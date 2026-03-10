use tracing::debug;

use crate::error::{Result, TritonError};
use crate::model::TernaryModel;

/// Ternary inference engine — runs forward passes through a `TernaryModel`.
pub struct TernaryEngine {
    model: TernaryModel,
}

impl TernaryEngine {
    /// Create a new engine from a model.
    pub fn new(model: TernaryModel) -> Self {
        Self { model }
    }

    /// Run a forward pass through all layers.
    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>> {
        if input.len() != self.model.input_dim() {
            return Err(TritonError::DimensionMismatch {
                expected: self.model.input_dim(),
                got: input.len(),
            });
        }

        let mut current = input.to_vec();

        for (i, layer) in self.model.layers.iter().enumerate() {
            debug!(
                layer = i,
                name = %layer.name,
                input_dim = layer.input_dim,
                output_dim = layer.output_dim,
                "running layer"
            );
            current = layer.forward(&current)?;
        }

        Ok(current)
    }

    /// Run inference and return the index of the highest output.
    pub fn predict(&self, input: &[f32]) -> Result<usize> {
        let output = self.forward(input)?;
        output
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .ok_or(TritonError::EmptyModel)
    }

    /// Get a reference to the underlying model.
    pub fn model(&self) -> &TernaryModel {
        &self.model
    }

    /// Consume the engine and return the model.
    pub fn into_model(self) -> TernaryModel {
        self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation::Activation;
    use crate::model::TernaryModelBuilder;
    use crate::packing::TernaryValue;

    fn make_test_engine() -> TernaryEngine {
        // 4 → 3 (ReLU) → 2 (None)
        let model = TernaryModelBuilder::new("test_engine")
            .add_dense_layer(
                "hidden",
                4,
                3,
                &[
                    vec![TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Zero, TernaryValue::Zero],
                    vec![TernaryValue::Zero, TernaryValue::Zero, TernaryValue::Pos, TernaryValue::Pos],
                    vec![TernaryValue::Neg, TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Pos],
                ],
                vec![0.0; 3],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "output",
                3,
                2,
                &[
                    vec![TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Zero],
                    vec![TernaryValue::Neg, TernaryValue::Pos, TernaryValue::Pos],
                ],
                vec![0.0; 2],
                Activation::None,
            )
            .unwrap()
            .build()
            .unwrap();

        TernaryEngine::new(model)
    }

    #[test]
    fn forward_pass() {
        let engine = make_test_engine();
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let output = engine.forward(&input).unwrap();
        assert_eq!(output.len(), 2);

        // Hidden layer:
        // neuron 0: +1*1 + +1*2 + 0*3 + 0*4 = 3.0 → ReLU → 3.0
        // neuron 1: 0*1 + 0*2 + +1*3 + +1*4 = 7.0 → ReLU → 7.0
        // neuron 2: -1*1 + +1*2 + -1*3 + +1*4 = -1+2-3+4 = 2.0 → ReLU → 2.0
        //
        // Output layer (input [3.0, 7.0, 2.0]):
        // neuron 0: +1*3 + -1*7 + 0*2 = 3 - 7 = -4.0
        // neuron 1: -1*3 + +1*7 + +1*2 = -3 + 7 + 2 = 6.0
        assert!((output[0] - (-4.0)).abs() < f32::EPSILON);
        assert!((output[1] - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn predict_returns_argmax() {
        let engine = make_test_engine();
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let prediction = engine.predict(&input).unwrap();
        // output[1] = 6.0 > output[0] = -4.0
        assert_eq!(prediction, 1);
    }

    #[test]
    fn wrong_input_dim() {
        let engine = make_test_engine();
        assert!(engine.forward(&[1.0, 2.0]).is_err());
    }

    #[test]
    fn model_access() {
        let engine = make_test_engine();
        assert_eq!(engine.model().num_layers(), 2);
        assert_eq!(engine.model().input_dim(), 4);
        assert_eq!(engine.model().output_dim(), 2);
    }
}
