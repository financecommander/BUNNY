//! Ternary tensor arithmetic operating on PackedTernary data.
//!
//! These ops work directly on 2-bit packed u32 words, avoiding the need to
//! unpack to f32 for many common operations. Zero-words (all zeros) are
//! skipped automatically for O(sparsity) performance.

use bunny_triton::packing::{pack, unpack, PackedTernary, TernaryValue};

use crate::error::{CalculusError, Result};

/// Dot product of two ternary vectors.
///
/// Since values are {-1, 0, +1}, the dot product is computed purely via
/// addition/subtraction with zero-skipping — no multiplies needed.
pub fn ternary_dot(a: &PackedTernary, b: &PackedTernary) -> Result<i64> {
    if a.len != b.len {
        return Err(CalculusError::DimensionMismatch {
            expected: a.len,
            got: b.len,
        });
    }
    if a.len == 0 {
        return Ok(0);
    }

    let mut sum: i64 = 0;

    for (wa, wb) in a.data.iter().zip(b.data.iter()) {
        // Skip if either word is all zeros.
        if *wa == 0 || *wb == 0 {
            continue;
        }

        // Process 16 pairs per word.
        for i in 0..16 {
            let shift = i * 2;
            let va = (*wa >> shift) & 0x03;
            let vb = (*wb >> shift) & 0x03;

            // Encoding: 0b00=0, 0b01=+1, 0b11=-1
            let sa: i64 = match va {
                0b01 => 1,
                0b11 => -1,
                _ => 0,
            };
            let sb: i64 = match vb {
                0b01 => 1,
                0b11 => -1,
                _ => 0,
            };

            sum += sa * sb;
        }
    }

    Ok(sum)
}

/// Hamming distance between two ternary vectors.
///
/// Counts positions where values differ. Useful for nearest-neighbor search
/// on packed ternary embeddings.
pub fn ternary_hamming(a: &PackedTernary, b: &PackedTernary) -> Result<usize> {
    if a.len != b.len {
        return Err(CalculusError::DimensionMismatch {
            expected: a.len,
            got: b.len,
        });
    }

    let mut distance: usize = 0;

    for (wa, wb) in a.data.iter().zip(b.data.iter()) {
        // XOR reveals differing bits, then count differing 2-bit slots.
        let diff = wa ^ wb;
        for i in 0..16 {
            let slot = (diff >> (i * 2)) & 0x03;
            if slot != 0 {
                distance += 1;
            }
        }
    }

    // Clamp to actual length (last word may have padding slots).
    let capacity = a.data.len() * 16;
    let _padding = capacity - a.len;
    Ok(distance.min(a.len))
}

/// L1 norm of a ternary vector (count of non-zero elements).
pub fn ternary_l1_norm(v: &PackedTernary) -> usize {
    let mut count = 0;
    for word in &v.data {
        if *word == 0 {
            continue;
        }
        for i in 0..16 {
            let slot = (*word >> (i * 2)) & 0x03;
            if slot != 0 {
                count += 1;
            }
        }
    }
    // Clamp to actual length (last word may have padding).
    count.min(v.len)
}

/// Sparsity ratio [0.0, 1.0] — fraction of zeros.
pub fn ternary_sparsity(v: &PackedTernary) -> f64 {
    if v.len == 0 {
        return 1.0;
    }
    let nonzero = ternary_l1_norm(v);
    1.0 - (nonzero as f64 / v.len as f64)
}

/// Element-wise ternary addition (saturating to {-1, 0, +1}).
///
/// Useful for gradient accumulation in federated learning.
pub fn ternary_add_saturate(a: &PackedTernary, b: &PackedTernary) -> Result<PackedTernary> {
    if a.len != b.len {
        return Err(CalculusError::DimensionMismatch {
            expected: a.len,
            got: b.len,
        });
    }

    let vals_a = unpack(a);
    let vals_b = unpack(b);

    let result: Vec<TernaryValue> = vals_a
        .iter()
        .zip(vals_b.iter())
        .map(|(va, vb)| {
            let sum = (*va as i8) as i16 + (*vb as i8) as i16;
            if sum > 0 {
                TernaryValue::Pos
            } else if sum < 0 {
                TernaryValue::Neg
            } else {
                TernaryValue::Zero
            }
        })
        .collect();

    Ok(pack(&result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_product_orthogonal() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Zero, TernaryValue::Zero]);
        let b = pack(&[TernaryValue::Zero, TernaryValue::Pos, TernaryValue::Zero]);
        assert_eq!(ternary_dot(&a, &b).unwrap(), 0);
    }

    #[test]
    fn dot_product_parallel() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Neg]);
        let b = pack(&[TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Neg]);
        // 1*1 + 1*1 + (-1)*(-1) = 3
        assert_eq!(ternary_dot(&a, &b).unwrap(), 3);
    }

    #[test]
    fn dot_product_antiparallel() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Pos]);
        let b = pack(&[TernaryValue::Neg, TernaryValue::Neg]);
        assert_eq!(ternary_dot(&a, &b).unwrap(), -2);
    }

    #[test]
    fn dot_dimension_mismatch() {
        let a = pack(&[TernaryValue::Pos]);
        let b = pack(&[TernaryValue::Pos, TernaryValue::Neg]);
        assert!(ternary_dot(&a, &b).is_err());
    }

    #[test]
    fn l1_norm() {
        let v = pack(&[
            TernaryValue::Pos,
            TernaryValue::Zero,
            TernaryValue::Neg,
            TernaryValue::Zero,
        ]);
        assert_eq!(ternary_l1_norm(&v), 2);
    }

    #[test]
    fn sparsity() {
        let v = pack(&[
            TernaryValue::Pos,
            TernaryValue::Zero,
            TernaryValue::Zero,
            TernaryValue::Zero,
        ]);
        assert!((ternary_sparsity(&v) - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn add_saturate() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Pos]);
        let b = pack(&[TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Neg]);
        let result = ternary_add_saturate(&a, &b).unwrap();
        let values = unpack(&result);
        // 1+1=1(sat), -1+1=0, 1+(-1)=0
        assert_eq!(values[0], TernaryValue::Pos);
        assert_eq!(values[1], TernaryValue::Zero);
        assert_eq!(values[2], TernaryValue::Zero);
    }

    #[test]
    fn empty_vectors() {
        let a = pack(&[]);
        assert_eq!(ternary_dot(&a, &a).unwrap(), 0);
        assert_eq!(ternary_l1_norm(&a), 0);
        assert!((ternary_sparsity(&a) - 1.0).abs() < f64::EPSILON);
    }
}
