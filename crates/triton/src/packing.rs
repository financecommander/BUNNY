use serde::{Deserialize, Serialize};

use crate::error::{Result, TritonError};

/// A ternary value: -1, 0, or +1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i8)]
pub enum TernaryValue {
    Neg = -1,
    Zero = 0,
    Pos = 1,
}

impl TernaryValue {
    pub fn from_i8(v: i8) -> Result<Self> {
        match v {
            -1 => Ok(Self::Neg),
            0 => Ok(Self::Zero),
            1 => Ok(Self::Pos),
            _ => Err(TritonError::InvalidValue(v)),
        }
    }
}

/// 2-bit encoding per weight, 16 weights per u32.
///
/// Encoding:
/// - `0b00` = 0 (zero)
/// - `0b01` = +1 (positive)
/// - `0b11` = -1 (negative)
///
/// This encoding means a u32 of all zeros is 16 zero-weights,
/// enabling fast zero-skipping during matmul.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedTernary {
    /// Packed weight data, 16 weights per u32.
    pub data: Vec<u32>,
    /// Total number of ternary values (may not be a multiple of 16).
    pub len: usize,
}

impl PackedTernary {
    /// Number of packed u32 words.
    pub fn word_count(&self) -> usize {
        self.data.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Encode a single ternary value into its 2-bit representation.
#[inline]
fn encode(v: TernaryValue) -> u32 {
    match v {
        TernaryValue::Zero => 0b00,
        TernaryValue::Pos => 0b01,
        TernaryValue::Neg => 0b11,
    }
}

/// Decode a 2-bit value back to a ternary value.
#[inline]
fn decode(bits: u32) -> TernaryValue {
    match bits & 0b11 {
        0b00 => TernaryValue::Zero,
        0b01 => TernaryValue::Pos,
        0b11 => TernaryValue::Neg,
        // 0b10 is unused; treat as zero for robustness
        _ => TernaryValue::Zero,
    }
}

/// Pack a slice of ternary values into a `PackedTernary`.
///
/// 16 values are packed per u32 word, with the first value in the
/// least significant 2 bits.
pub fn pack(values: &[TernaryValue]) -> PackedTernary {
    let num_words = (values.len() + 15) / 16;
    let mut data = vec![0u32; num_words];

    for (i, &v) in values.iter().enumerate() {
        let word_idx = i / 16;
        let bit_offset = (i % 16) * 2;
        data[word_idx] |= encode(v) << bit_offset;
    }

    PackedTernary {
        data,
        len: values.len(),
    }
}

/// Unpack a `PackedTernary` back into a vector of ternary values.
pub fn unpack(packed: &PackedTernary) -> Vec<TernaryValue> {
    let mut values = Vec::with_capacity(packed.len);

    for i in 0..packed.len {
        let word_idx = i / 16;
        let bit_offset = (i % 16) * 2;
        let bits = (packed.data[word_idx] >> bit_offset) & 0b11;
        values.push(decode(bits));
    }

    values
}

/// Quantize f32 values to ternary using a threshold.
///
/// - `|x| > threshold` → sign(x)
/// - `|x| <= threshold` → 0
pub fn quantize_f32(values: &[f32], threshold: f32) -> Vec<TernaryValue> {
    values
        .iter()
        .map(|&x| {
            if x > threshold {
                TernaryValue::Pos
            } else if x < -threshold {
                TernaryValue::Neg
            } else {
                TernaryValue::Zero
            }
        })
        .collect()
}

/// Quantize f32 values directly into packed ternary format.
pub fn quantize_f32_packed(values: &[f32], threshold: f32) -> PackedTernary {
    let ternary = quantize_f32(values, threshold);
    pack(&ternary)
}

/// Accumulate a packed u32 word against an input slice.
///
/// For each of the 16 values in the word:
/// - +1 → add input[offset + i]
/// - -1 → subtract input[offset + i]
/// - 0  → skip
///
/// Returns the accumulated sum (no floating-point multiply needed).
#[inline]
pub fn accumulate_word(word: u32, input: &[f32], offset: usize) -> f32 {
    let mut acc = 0.0f32;
    let mut w = word;

    for i in 0..16 {
        let bits = w & 0b11;
        match bits {
            0b01 => acc += input[offset + i],
            0b11 => acc -= input[offset + i],
            _ => {} // 0b00 (zero) or 0b10 (unused) — skip
        }
        w >>= 2;
    }

    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let values = vec![
            TernaryValue::Pos,
            TernaryValue::Neg,
            TernaryValue::Zero,
            TernaryValue::Pos,
            TernaryValue::Zero,
        ];
        let packed = pack(&values);
        let unpacked = unpack(&packed);
        assert_eq!(unpacked, values);
    }

    #[test]
    fn pack_16_values_single_word() {
        let values = vec![TernaryValue::Pos; 16];
        let packed = pack(&values);
        assert_eq!(packed.word_count(), 1);
        assert_eq!(packed.len, 16);
        let unpacked = unpack(&packed);
        assert_eq!(unpacked, values);
    }

    #[test]
    fn pack_17_values_two_words() {
        let values = vec![TernaryValue::Neg; 17];
        let packed = pack(&values);
        assert_eq!(packed.word_count(), 2);
        assert_eq!(packed.len, 17);
        let unpacked = unpack(&packed);
        assert_eq!(unpacked, values);
    }

    #[test]
    fn all_zeros_pack_to_zero_word() {
        let values = vec![TernaryValue::Zero; 16];
        let packed = pack(&values);
        assert_eq!(packed.data[0], 0x00000000);
    }

    #[test]
    fn quantize_threshold() {
        let values = vec![0.5, -0.5, 0.1, -0.1, 0.3, -0.3];
        let result = quantize_f32(&values, 0.3);
        assert_eq!(
            result,
            vec![
                TernaryValue::Pos,
                TernaryValue::Neg,
                TernaryValue::Zero,
                TernaryValue::Zero,
                TernaryValue::Zero, // 0.3 is not > 0.3
                TernaryValue::Zero, // -0.3 is not < -0.3
            ]
        );
    }

    #[test]
    fn quantize_packed_roundtrip() {
        let values = vec![1.0, -1.0, 0.0, 0.5, -0.5];
        let packed = quantize_f32_packed(&values, 0.3);
        let unpacked = unpack(&packed);
        assert_eq!(
            unpacked,
            vec![
                TernaryValue::Pos,
                TernaryValue::Neg,
                TernaryValue::Zero,
                TernaryValue::Pos,
                TernaryValue::Neg,
            ]
        );
    }

    #[test]
    fn accumulate_word_correctness() {
        // Pack: [+1, -1, 0, +1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        let values = vec![
            TernaryValue::Pos,
            TernaryValue::Neg,
            TernaryValue::Zero,
            TernaryValue::Pos,
        ];
        let mut padded = values;
        padded.resize(16, TernaryValue::Zero);
        let packed = pack(&padded);

        let input = vec![
            1.0, 2.0, 3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ];
        // Expected: +1*1.0 + (-1)*2.0 + 0*3.0 + (+1)*4.0 = 1 - 2 + 0 + 4 = 3.0
        let result = accumulate_word(packed.data[0], &input, 0);
        assert!((result - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_word_skip() {
        // All zeros → word is 0x00000000
        let packed = pack(&vec![TernaryValue::Zero; 16]);
        assert_eq!(packed.data[0], 0);
        // Accumulate on zero word should give 0
        let input = vec![1.0; 16];
        let result = accumulate_word(packed.data[0], &input, 0);
        assert!((result - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn empty_pack() {
        let packed = pack(&[]);
        assert!(packed.is_empty());
        assert_eq!(packed.word_count(), 0);
        let unpacked = unpack(&packed);
        assert!(unpacked.is_empty());
    }

    #[test]
    fn encoding_values() {
        // Verify the bit patterns directly
        let packed = pack(&[TernaryValue::Pos]); // 0b01
        assert_eq!(packed.data[0] & 0b11, 0b01);

        let packed = pack(&[TernaryValue::Neg]); // 0b11
        assert_eq!(packed.data[0] & 0b11, 0b11);

        let packed = pack(&[TernaryValue::Zero]); // 0b00
        assert_eq!(packed.data[0] & 0b11, 0b00);
    }
}
