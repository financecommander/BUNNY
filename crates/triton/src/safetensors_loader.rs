use safetensors::SafeTensors;

use crate::activation::Activation;
use crate::error::{Result, TritonError};
use crate::layer::TernaryLayer;
use crate::model::{TernaryModel, TernaryModelBuilder};
use crate::packing::{quantize_f32_packed, PackedTernary};

/// Configuration for loading and quantizing a safetensors model.
pub struct SafetensorsConfig {
    /// Model name.
    pub name: String,
    /// Quantization threshold: `|x| > threshold → sign(x)`, else 0.
    pub threshold: f32,
    /// Layer specifications in order. Each entry describes how to find
    /// weight and bias tensors and what activation to apply.
    pub layers: Vec<LayerSpec>,
}

/// Specification for a single layer to extract from safetensors.
pub struct LayerSpec {
    /// Layer name.
    pub name: String,
    /// Tensor name for the weight matrix.
    pub weight_tensor: String,
    /// Tensor name for the bias vector (optional).
    pub bias_tensor: Option<String>,
    /// Activation function.
    pub activation: Activation,
}

/// Load f32 values from a safetensors tensor.
fn read_f32_tensor(tensors: &SafeTensors, name: &str) -> Result<Vec<f32>> {
    let view = tensors
        .tensor(name)
        .map_err(|e| TritonError::SafetensorsError(format!("tensor '{name}': {e}")))?;

    let data = view.data();

    match view.dtype() {
        safetensors::Dtype::F32 => {
            if data.len() % 4 != 0 {
                return Err(TritonError::SafetensorsError(format!(
                    "tensor '{name}': data length {} not aligned to f32",
                    data.len()
                )));
            }
            Ok(data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect())
        }
        safetensors::Dtype::F16 => {
            if data.len() % 2 != 0 {
                return Err(TritonError::SafetensorsError(format!(
                    "tensor '{name}': data length {} not aligned to f16",
                    data.len()
                )));
            }
            Ok(data
                .chunks_exact(2)
                .map(|chunk| {
                    let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
                    f16_to_f32(bits)
                })
                .collect())
        }
        safetensors::Dtype::BF16 => {
            if data.len() % 2 != 0 {
                return Err(TritonError::SafetensorsError(format!(
                    "tensor '{name}': data length {} not aligned to bf16",
                    data.len()
                )));
            }
            Ok(data
                .chunks_exact(2)
                .map(|chunk| {
                    let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
                    bf16_to_f32(bits)
                })
                .collect())
        }
        other => Err(TritonError::SafetensorsError(format!(
            "tensor '{name}': unsupported dtype {other:?}, expected F32/F16/BF16"
        ))),
    }
}

/// Convert IEEE 754 half-precision (f16) to f32.
fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1F) as u32;
    let mantissa = (bits & 0x3FF) as u32;

    if exp == 0 {
        // Subnormal or zero
        if mantissa == 0 {
            return f32::from_bits(sign << 31);
        }
        // Subnormal: normalize
        let mut m = mantissa;
        let mut e = 0i32;
        while m & 0x400 == 0 {
            m <<= 1;
            e -= 1;
        }
        m &= 0x3FF;
        let f32_exp = ((127 - 15 + 1 + e) as u32) & 0xFF;
        f32::from_bits((sign << 31) | (f32_exp << 23) | (m << 13))
    } else if exp == 31 {
        // Inf or NaN
        f32::from_bits((sign << 31) | (0xFF << 23) | (mantissa << 13))
    } else {
        // Normal
        let f32_exp = (exp + 127 - 15) & 0xFF;
        f32::from_bits((sign << 31) | (f32_exp << 23) | (mantissa << 13))
    }
}

/// Convert bfloat16 to f32 (simple left-shift of 16 bits).
fn bf16_to_f32(bits: u16) -> f32 {
    f32::from_bits((bits as u32) << 16)
}

/// Load a safetensors file and quantize it into a `TernaryModel`.
pub fn load_from_bytes(data: &[u8], config: &SafetensorsConfig) -> Result<TernaryModel> {
    let tensors = SafeTensors::deserialize(data)
        .map_err(|e| TritonError::SafetensorsError(e.to_string()))?;

    let mut builder = TernaryModelBuilder::new(&config.name);

    for spec in &config.layers {
        let weight_values = read_f32_tensor(&tensors, &spec.weight_tensor)?;

        let weight_view = tensors
            .tensor(&spec.weight_tensor)
            .map_err(|e| TritonError::SafetensorsError(e.to_string()))?;
        let shape = weight_view.shape();

        if shape.len() != 2 {
            return Err(TritonError::SafetensorsError(format!(
                "weight tensor '{}': expected 2D, got {}D",
                spec.weight_tensor,
                shape.len()
            )));
        }

        let output_dim = shape[0];
        let input_dim = shape[1];

        // Quantize and pack each row
        let mut packed_weights: Vec<PackedTernary> = Vec::with_capacity(output_dim);
        for row in 0..output_dim {
            let row_start = row * input_dim;
            let row_end = row_start + input_dim;
            let row_data = &weight_values[row_start..row_end];
            packed_weights.push(quantize_f32_packed(row_data, config.threshold));
        }

        // Load or zero-fill bias
        let bias = if let Some(ref bias_name) = spec.bias_tensor {
            read_f32_tensor(&tensors, bias_name)?
        } else {
            vec![0.0; output_dim]
        };

        if bias.len() != output_dim {
            return Err(TritonError::DimensionMismatch {
                expected: output_dim,
                got: bias.len(),
            });
        }

        let layer = TernaryLayer::new(
            spec.name.clone(),
            input_dim,
            output_dim,
            packed_weights,
            bias,
            spec.activation,
        )?;

        builder = builder.add_layer(layer);
    }

    builder.build()
}

/// Load a safetensors file from disk and quantize it.
pub fn load_from_file(path: &std::path::Path, config: &SafetensorsConfig) -> Result<TernaryModel> {
    let data = std::fs::read(path)?;
    load_from_bytes(&data, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_conversion() {
        // f16 for 1.0: sign=0, exp=15, mantissa=0 → 0b0_01111_0000000000 = 0x3C00
        let one = f16_to_f32(0x3C00);
        assert!((one - 1.0).abs() < 1e-6);

        // f16 for -1.0: 0xBC00
        let neg_one = f16_to_f32(0xBC00);
        assert!((neg_one - (-1.0)).abs() < 1e-6);

        // f16 for 0.0: 0x0000
        let zero = f16_to_f32(0x0000);
        assert!((zero - 0.0).abs() < 1e-6);
    }

    #[test]
    fn bf16_conversion() {
        // bf16 for 1.0: upper 16 bits of f32 1.0 = 0x3F80
        let one = bf16_to_f32(0x3F80);
        assert!((one - 1.0).abs() < 1e-6);

        // bf16 for -2.0: 0xC000
        let neg_two = bf16_to_f32(0xC000);
        assert!((neg_two - (-2.0)).abs() < 1e-6);
    }

    #[test]
    fn load_from_safetensors_bytes() {
        // Build a minimal safetensors file in memory
        use std::collections::HashMap;

        // Create weight tensor: 2x3 matrix of f32
        let weight_data: Vec<f32> = vec![
            1.0, -1.0, 0.1, // row 0
            -0.5, 0.8, 0.0, // row 1
        ];
        let weight_bytes: Vec<u8> = weight_data.iter().flat_map(|f| f.to_le_bytes()).collect();

        // Create bias tensor: 2 values
        let bias_data: Vec<f32> = vec![0.5, -0.5];
        let bias_bytes: Vec<u8> = bias_data.iter().flat_map(|f| f.to_le_bytes()).collect();

        let mut tensors = HashMap::new();
        tensors.insert(
            "layer.weight".to_string(),
            safetensors::tensor::TensorView::new(
                safetensors::Dtype::F32,
                vec![2, 3],
                &weight_bytes,
            )
            .unwrap(),
        );
        tensors.insert(
            "layer.bias".to_string(),
            safetensors::tensor::TensorView::new(
                safetensors::Dtype::F32,
                vec![2],
                &bias_bytes,
            )
            .unwrap(),
        );

        let serialized = safetensors::tensor::serialize(&tensors, &None).unwrap();

        let config = SafetensorsConfig {
            name: "test_st".into(),
            threshold: 0.3,
            layers: vec![LayerSpec {
                name: "layer0".into(),
                weight_tensor: "layer.weight".into(),
                bias_tensor: Some("layer.bias".into()),
                activation: Activation::ReLU,
            }],
        };

        let model = load_from_bytes(&serialized, &config).unwrap();
        assert_eq!(model.num_layers(), 1);
        assert_eq!(model.input_dim(), 3);
        assert_eq!(model.output_dim(), 2);

        // Verify quantization: threshold 0.3
        // Row 0: [1.0→+1, -1.0→-1, 0.1→0]
        // Row 1: [-0.5→-1, 0.8→+1, 0.0→0]
        let engine = crate::engine::TernaryEngine::new(model);
        let output = engine.forward(&[1.0, 1.0, 1.0]).unwrap();
        // Row 0: +1*1 + -1*1 + 0*1 + bias=0.5 = 0.5 → ReLU → 0.5
        // Row 1: -1*1 + +1*1 + 0*1 + bias=-0.5 = -0.5 → ReLU → 0.0
        assert!((output[0] - 0.5).abs() < f32::EPSILON);
        assert!((output[1] - 0.0).abs() < f32::EPSILON);
    }
}
