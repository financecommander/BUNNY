use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, TritonError};
use crate::model::{ModelMetadata, TernaryModel};
use crate::layer::TernaryLayer;
use crate::packing::PackedTernary;

/// Magic bytes for the .bunny format.
const MAGIC: &[u8; 4] = b"BNNY";

/// Header stored as JSON inside the .bunny file.
#[derive(Debug, Serialize, Deserialize)]
struct BunnyHeader {
    metadata: ModelMetadata,
    layers: Vec<LayerDescriptor>,
    /// SHA-256 hash of the packed weight data.
    weight_hash: String,
}

/// Describes a layer's shape for deserialization.
#[derive(Debug, Serialize, Deserialize)]
struct LayerDescriptor {
    name: String,
    input_dim: usize,
    output_dim: usize,
    activation: crate::activation::Activation,
    /// Number of u32 words in packed weights for this layer.
    packed_words: usize,
    /// Number of f32 bias values.
    bias_count: usize,
}

/// Serialize a `TernaryModel` into .bunny binary format.
///
/// Format: `[BNNY magic: 4B] [header_len: 4B BE] [JSON header] [packed weight data]`
///
/// Weight data layout per layer: `[packed u32 words (LE)] [bias f32 values (LE)]`
pub fn serialize(model: &TernaryModel) -> Result<Vec<u8>> {
    // Collect all weight data
    let mut weight_data = Vec::new();
    let mut descriptors = Vec::new();

    for layer in &model.layers {
        let words_per_row = (layer.input_dim + 15) / 16;
        let total_words = words_per_row * layer.output_dim;

        // Write packed words row-by-row
        for row in &layer.weights {
            for &word in &row.data {
                weight_data.extend_from_slice(&word.to_le_bytes());
            }
        }
        // Write bias values
        for &b in &layer.bias {
            weight_data.extend_from_slice(&b.to_le_bytes());
        }

        descriptors.push(LayerDescriptor {
            name: layer.name.clone(),
            input_dim: layer.input_dim,
            output_dim: layer.output_dim,
            activation: layer.activation,
            packed_words: total_words,
            bias_count: layer.bias.len(),
        });
    }

    // Hash weight data
    let mut hasher = Sha256::new();
    hasher.update(&weight_data);
    let hash = format!("{:x}", hasher.finalize());

    let header = BunnyHeader {
        metadata: model.metadata.clone(),
        layers: descriptors,
        weight_hash: hash,
    };

    let header_json = serde_json::to_vec(&header)
        .map_err(|e| TritonError::SerializationError(e.to_string()))?;

    // Build final binary
    let mut output = Vec::new();
    output.write_all(MAGIC).unwrap();
    output.write_all(&(header_json.len() as u32).to_be_bytes()).unwrap();
    output.write_all(&header_json).unwrap();
    output.write_all(&weight_data).unwrap();

    Ok(output)
}

/// Deserialize a .bunny binary into a `TernaryModel`.
pub fn deserialize(data: &[u8]) -> Result<TernaryModel> {
    let mut cursor = std::io::Cursor::new(data);

    // Read magic
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).map_err(|_| TritonError::InvalidMagic)?;
    if &magic != MAGIC {
        return Err(TritonError::InvalidMagic);
    }

    // Read header length
    let mut header_len_bytes = [0u8; 4];
    cursor.read_exact(&mut header_len_bytes)
        .map_err(|e| TritonError::InvalidFormat(e.to_string()))?;
    let header_len = u32::from_be_bytes(header_len_bytes) as usize;

    // Read header JSON
    let mut header_json = vec![0u8; header_len];
    cursor.read_exact(&mut header_json)
        .map_err(|e| TritonError::InvalidFormat(e.to_string()))?;
    let header: BunnyHeader = serde_json::from_slice(&header_json)
        .map_err(|e| TritonError::SerializationError(e.to_string()))?;

    // Read weight data
    let mut weight_data = Vec::new();
    cursor.read_to_end(&mut weight_data)
        .map_err(|e| TritonError::InvalidFormat(e.to_string()))?;

    // Verify hash
    let mut hasher = Sha256::new();
    hasher.update(&weight_data);
    let hash = format!("{:x}", hasher.finalize());
    if hash != header.weight_hash {
        return Err(TritonError::InvalidFormat(
            "weight data hash mismatch — file may be corrupted".into(),
        ));
    }

    // Reconstruct layers
    let mut offset = 0usize;
    let mut layers = Vec::new();

    for desc in &header.layers {
        let words_per_row = (desc.input_dim + 15) / 16;

        // Read packed weights
        let mut weights = Vec::with_capacity(desc.output_dim);
        for _ in 0..desc.output_dim {
            let mut row_data = Vec::with_capacity(words_per_row);
            for _ in 0..words_per_row {
                let bytes: [u8; 4] = weight_data[offset..offset + 4]
                    .try_into()
                    .map_err(|_| TritonError::InvalidFormat("truncated weight data".into()))?;
                row_data.push(u32::from_le_bytes(bytes));
                offset += 4;
            }
            weights.push(PackedTernary {
                data: row_data,
                len: desc.input_dim,
            });
        }

        // Read bias
        let mut bias = Vec::with_capacity(desc.bias_count);
        for _ in 0..desc.bias_count {
            let bytes: [u8; 4] = weight_data[offset..offset + 4]
                .try_into()
                .map_err(|_| TritonError::InvalidFormat("truncated bias data".into()))?;
            bias.push(f32::from_le_bytes(bytes));
            offset += 4;
        }

        layers.push(TernaryLayer::new(
            desc.name.clone(),
            desc.input_dim,
            desc.output_dim,
            weights,
            bias,
            desc.activation,
        )?);
    }

    Ok(TernaryModel {
        metadata: header.metadata,
        layers,
    })
}

/// Save a model to a .bunny file.
pub fn save_to_file(model: &TernaryModel, path: &std::path::Path) -> Result<()> {
    let data = serialize(model)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Load a model from a .bunny file.
pub fn load_from_file(path: &std::path::Path) -> Result<TernaryModel> {
    let data = std::fs::read(path)?;
    deserialize(&data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation::Activation;
    use crate::model::TernaryModelBuilder;
    use crate::packing::TernaryValue;

    fn test_model() -> TernaryModel {
        TernaryModelBuilder::new("test_bunny")
            .version(1)
            .description("test model for .bunny format")
            .add_dense_layer(
                "hidden",
                4,
                3,
                &[
                    vec![TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Zero, TernaryValue::Pos],
                    vec![TernaryValue::Zero; 4],
                    vec![TernaryValue::Neg; 4],
                ],
                vec![0.5, -0.5, 1.0],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "output",
                3,
                2,
                &[
                    vec![TernaryValue::Pos; 3],
                    vec![TernaryValue::Neg, TernaryValue::Pos, TernaryValue::Zero],
                ],
                vec![0.0, 1.0],
                Activation::Softmax,
            )
            .unwrap()
            .build()
            .unwrap()
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        let model = test_model();
        let data = serialize(&model).unwrap();

        // Verify magic bytes
        assert_eq!(&data[0..4], b"BNNY");

        let restored = deserialize(&data).unwrap();
        assert_eq!(restored.metadata.name, "test_bunny");
        assert_eq!(restored.num_layers(), 2);
        assert_eq!(restored.input_dim(), 4);
        assert_eq!(restored.output_dim(), 2);

        // Verify forward pass gives same result
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let engine_orig = crate::engine::TernaryEngine::new(model);
        let engine_restored = crate::engine::TernaryEngine::new(restored);
        let out_orig = engine_orig.forward(&input).unwrap();
        let out_restored = engine_restored.forward(&input).unwrap();
        for (a, b) in out_orig.iter().zip(out_restored.iter()) {
            assert!((a - b).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn invalid_magic_rejected() {
        let mut data = serialize(&test_model()).unwrap();
        data[0] = b'X';
        assert!(deserialize(&data).is_err());
    }

    #[test]
    fn corrupted_weights_detected() {
        let mut data = serialize(&test_model()).unwrap();
        // Flip a byte in the weight data (after header)
        let last = data.len() - 1;
        data[last] ^= 0xFF;
        assert!(deserialize(&data).is_err());
    }
}
