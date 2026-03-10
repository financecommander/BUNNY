use serde::{Deserialize, Serialize};

use crate::error::{Result, TritonError};
use crate::model::{ModelMetadata, TernaryModel};
use crate::layer::TernaryLayer;

/// A shard of a model — a contiguous subset of layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelShard {
    /// Which shard this is (0-indexed).
    pub shard_index: usize,
    /// Total number of shards.
    pub total_shards: usize,
    /// Original model metadata.
    pub metadata: ModelMetadata,
    /// Layers in this shard.
    pub layers: Vec<TernaryLayer>,
    /// Index of the first layer in the original model.
    pub start_layer: usize,
}

impl ModelShard {
    /// Number of layers in this shard.
    pub fn num_layers(&self) -> usize {
        self.layers.len()
    }
}

/// Split a model into `num_shards` shards by distributing layers evenly.
///
/// Layers are distributed round-robin-style to keep shard sizes balanced.
/// For pipeline parallelism, each shard processes its contiguous layer block.
pub fn split(model: TernaryModel, num_shards: usize) -> Result<Vec<ModelShard>> {
    if num_shards == 0 {
        return Err(TritonError::ShardError("num_shards must be > 0".into()));
    }
    if num_shards > model.num_layers() {
        return Err(TritonError::ShardError(format!(
            "num_shards ({num_shards}) exceeds layer count ({})",
            model.num_layers()
        )));
    }

    let total_layers = model.num_layers();
    let base_size = total_layers / num_shards;
    let remainder = total_layers % num_shards;

    let mut shards = Vec::with_capacity(num_shards);
    let mut layer_iter = model.layers.into_iter();
    let mut start_layer = 0;

    for i in 0..num_shards {
        let count = base_size + if i < remainder { 1 } else { 0 };
        let layers: Vec<TernaryLayer> = layer_iter.by_ref().take(count).collect();

        shards.push(ModelShard {
            shard_index: i,
            total_shards: num_shards,
            metadata: model.metadata.clone(),
            layers,
            start_layer,
        });

        start_layer += count;
    }

    Ok(shards)
}

/// Merge shards back into a complete model.
///
/// Shards must be provided in order (sorted by `shard_index`).
pub fn merge(mut shards: Vec<ModelShard>) -> Result<TernaryModel> {
    if shards.is_empty() {
        return Err(TritonError::ShardError("no shards to merge".into()));
    }

    // Sort by shard index
    shards.sort_by_key(|s| s.shard_index);

    // Validate completeness
    let total = shards[0].total_shards;
    if shards.len() != total {
        return Err(TritonError::ShardError(format!(
            "expected {total} shards, got {}",
            shards.len()
        )));
    }
    for (i, shard) in shards.iter().enumerate() {
        if shard.shard_index != i {
            return Err(TritonError::ShardError(format!(
                "missing shard {i}, got shard {}",
                shard.shard_index
            )));
        }
    }

    let metadata = shards[0].metadata.clone();
    let layers: Vec<TernaryLayer> = shards.into_iter().flat_map(|s| s.layers).collect();

    Ok(TernaryModel { metadata, layers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation::Activation;
    use crate::engine::TernaryEngine;
    use crate::model::TernaryModelBuilder;
    use crate::packing::TernaryValue;

    fn test_model() -> TernaryModel {
        TernaryModelBuilder::new("shard_test")
            .add_dense_layer(
                "layer0",
                4,
                4,
                &vec![vec![TernaryValue::Pos; 4]; 4],
                vec![0.0; 4],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "layer1",
                4,
                4,
                &[
                    vec![TernaryValue::Pos, TernaryValue::Zero, TernaryValue::Zero, TernaryValue::Zero],
                    vec![TernaryValue::Zero, TernaryValue::Pos, TernaryValue::Zero, TernaryValue::Zero],
                    vec![TernaryValue::Zero, TernaryValue::Zero, TernaryValue::Pos, TernaryValue::Zero],
                    vec![TernaryValue::Zero, TernaryValue::Zero, TernaryValue::Zero, TernaryValue::Pos],
                ],
                vec![1.0; 4],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "layer2",
                4,
                2,
                &[
                    vec![TernaryValue::Pos; 4],
                    vec![TernaryValue::Neg; 4],
                ],
                vec![0.0; 2],
                Activation::None,
            )
            .unwrap()
            .build()
            .unwrap()
    }

    #[test]
    fn split_and_merge_roundtrip() {
        let model = test_model();
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let expected = TernaryEngine::new(model.clone()).forward(&input).unwrap();

        let shards = split(model, 2).unwrap();
        assert_eq!(shards.len(), 2);
        assert_eq!(shards[0].shard_index, 0);
        assert_eq!(shards[1].shard_index, 1);
        // 3 layers / 2 shards = [2, 1]
        assert_eq!(shards[0].num_layers(), 2);
        assert_eq!(shards[1].num_layers(), 1);

        let restored = merge(shards).unwrap();
        let actual = TernaryEngine::new(restored).forward(&input).unwrap();
        for (a, b) in expected.iter().zip(actual.iter()) {
            assert!((a - b).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn split_single_shard() {
        let model = test_model();
        let shards = split(model, 1).unwrap();
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].num_layers(), 3);
    }

    #[test]
    fn split_max_shards() {
        let model = test_model();
        let shards = split(model, 3).unwrap();
        assert_eq!(shards.len(), 3);
        for shard in &shards {
            assert_eq!(shard.num_layers(), 1);
        }
    }

    #[test]
    fn split_too_many_shards() {
        let model = test_model();
        assert!(split(model, 4).is_err());
    }

    #[test]
    fn split_zero_shards() {
        let model = test_model();
        assert!(split(model, 0).is_err());
    }

    #[test]
    fn merge_validates_completeness() {
        let model = test_model();
        let mut shards = split(model, 2).unwrap();
        shards.pop(); // remove last shard
        assert!(merge(shards).is_err());
    }
}
