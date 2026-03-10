use serde::{Deserialize, Serialize};

use crate::activation::Activation;
use crate::error::{Result, TritonError};
use crate::layer::TernaryLayer;
use crate::packing::{pack, PackedTernary, TernaryValue};

/// Metadata about a ternary model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub name: String,
    pub version: u32,
    pub description: String,
    pub total_params: usize,
}

/// A complete ternary neural network model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TernaryModel {
    pub metadata: ModelMetadata,
    pub layers: Vec<TernaryLayer>,
}

impl TernaryModel {
    /// Input dimension (first layer's input).
    pub fn input_dim(&self) -> usize {
        self.layers.first().map_or(0, |l| l.input_dim)
    }

    /// Output dimension (last layer's output).
    pub fn output_dim(&self) -> usize {
        self.layers.last().map_or(0, |l| l.output_dim)
    }

    /// Total number of layers.
    pub fn num_layers(&self) -> usize {
        self.layers.len()
    }

    /// Total parameter count across all layers.
    pub fn total_params(&self) -> usize {
        self.layers.iter().map(|l| l.weight_count() + l.output_dim).sum()
    }

    /// Average sparsity across all layers.
    pub fn average_sparsity(&self) -> f32 {
        if self.layers.is_empty() {
            return 0.0;
        }
        let total: f32 = self.layers.iter().map(|l| l.sparsity()).sum();
        total / self.layers.len() as f32
    }

    /// Get a layer by index.
    pub fn layer(&self, index: usize) -> Result<&TernaryLayer> {
        self.layers.get(index).ok_or(TritonError::LayerOutOfBounds {
            index,
            total: self.layers.len(),
        })
    }
}

/// Builder for constructing ternary models layer by layer.
pub struct TernaryModelBuilder {
    name: String,
    version: u32,
    description: String,
    layers: Vec<TernaryLayer>,
}

impl TernaryModelBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: 1,
            description: String::new(),
            layers: Vec::new(),
        }
    }

    pub fn version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Add a pre-built layer.
    pub fn add_layer(mut self, layer: TernaryLayer) -> Self {
        self.layers.push(layer);
        self
    }

    /// Add a layer from raw ternary values.
    ///
    /// `weights` is row-major: `weights[output_neuron][input_neuron]`.
    pub fn add_dense_layer(
        mut self,
        name: impl Into<String>,
        input_dim: usize,
        output_dim: usize,
        weights: &[Vec<TernaryValue>],
        bias: Vec<f32>,
        activation: Activation,
    ) -> Result<Self> {
        let packed: Vec<PackedTernary> = weights.iter().map(|row| pack(row)).collect();
        let layer = TernaryLayer::new(name.into(), input_dim, output_dim, packed, bias, activation)?;
        self.layers.push(layer);
        Ok(self)
    }

    /// Build the model, validating layer dimension chain.
    pub fn build(self) -> Result<TernaryModel> {
        if self.layers.is_empty() {
            return Err(TritonError::EmptyModel);
        }

        // Validate dimension chain: layer[i].output_dim == layer[i+1].input_dim
        for i in 0..self.layers.len() - 1 {
            let out = self.layers[i].output_dim;
            let next_in = self.layers[i + 1].input_dim;
            if out != next_in {
                return Err(TritonError::DimensionMismatch {
                    expected: out,
                    got: next_in,
                });
            }
        }

        let total_params: usize = self.layers.iter().map(|l| l.weight_count() + l.output_dim).sum();

        Ok(TernaryModel {
            metadata: ModelMetadata {
                name: self.name,
                version: self.version,
                description: self.description,
                total_params,
            },
            layers: self.layers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_single_layer() {
        let model = TernaryModelBuilder::new("test")
            .description("test model")
            .add_dense_layer(
                "layer0",
                4,
                2,
                &[
                    vec![TernaryValue::Pos; 4],
                    vec![TernaryValue::Neg; 4],
                ],
                vec![0.0, 0.0],
                Activation::None,
            )
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(model.input_dim(), 4);
        assert_eq!(model.output_dim(), 2);
        assert_eq!(model.num_layers(), 1);
    }

    #[test]
    fn builder_chain_validated() {
        let result = TernaryModelBuilder::new("bad")
            .add_dense_layer(
                "layer0",
                4,
                3,
                &vec![vec![TernaryValue::Pos; 4]; 3],
                vec![0.0; 3],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "layer1",
                5, // mismatch: prev output is 3
                2,
                &vec![vec![TernaryValue::Pos; 5]; 2],
                vec![0.0; 2],
                Activation::None,
            )
            .unwrap()
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn builder_multi_layer() {
        let model = TernaryModelBuilder::new("mlp")
            .version(2)
            .add_dense_layer(
                "hidden",
                4,
                3,
                &vec![vec![TernaryValue::Pos; 4]; 3],
                vec![0.0; 3],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "output",
                3,
                2,
                &vec![vec![TernaryValue::Neg; 3]; 2],
                vec![1.0; 2],
                Activation::Softmax,
            )
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(model.input_dim(), 4);
        assert_eq!(model.output_dim(), 2);
        assert_eq!(model.num_layers(), 2);
        assert_eq!(model.metadata.version, 2);
    }

    #[test]
    fn empty_model_rejected() {
        let result = TernaryModelBuilder::new("empty").build();
        assert!(result.is_err());
    }

    #[test]
    fn total_params() {
        let model = TernaryModelBuilder::new("test")
            .add_dense_layer(
                "layer0",
                4,
                2,
                &vec![vec![TernaryValue::Pos; 4]; 2],
                vec![0.0; 2],
                Activation::None,
            )
            .unwrap()
            .build()
            .unwrap();

        // 4*2 weights + 2 biases = 10
        assert_eq!(model.total_params(), 10);
    }
}
