//! Vector similarity search for ternary embeddings.
//!
//! Provides cosine similarity, Jaccard index, and nearest-neighbor search
//! on packed ternary vectors. Used for model routing (find similar model
//! embeddings) and RAG (retrieval-augmented generation with ternary stores).

use bunny_triton::packing::{unpack, PackedTernary, TernaryValue};

use crate::error::{CalculusError, Result};
use crate::tensor_ops::{ternary_dot, ternary_l1_norm};

/// Cosine similarity between two ternary vectors.
///
/// Returns a value in [-1.0, 1.0]. For ternary vectors, the "magnitudes"
/// are simply the L1 norms (count of non-zero elements) since |±1|² = 1.
pub fn cosine_similarity(a: &PackedTernary, b: &PackedTernary) -> Result<f64> {
    let dot = ternary_dot(a, b)? as f64;
    let norm_a = ternary_l1_norm(a) as f64;
    let norm_b = ternary_l1_norm(b) as f64;

    if norm_a == 0.0 || norm_b == 0.0 {
        return Ok(0.0);
    }

    Ok(dot / (norm_a.sqrt() * norm_b.sqrt()))
}

/// Jaccard index between two ternary vectors.
///
/// Measures overlap: |A ∩ B| / |A ∪ B| where the "set" is positions
/// where both vectors agree on a non-zero value.
pub fn jaccard_index(a: &PackedTernary, b: &PackedTernary) -> Result<f64> {
    if a.len != b.len {
        return Err(CalculusError::DimensionMismatch {
            expected: a.len,
            got: b.len,
        });
    }
    if a.len == 0 {
        return Ok(1.0); // two empty vectors are identical
    }

    let vals_a = unpack(a);
    let vals_b = unpack(b);

    let mut intersection = 0usize;
    let mut union = 0usize;

    for (va, vb) in vals_a.iter().zip(vals_b.iter()) {
        let a_nz = *va != TernaryValue::Zero;
        let b_nz = *vb != TernaryValue::Zero;

        if a_nz || b_nz {
            union += 1;
            if *va == *vb {
                intersection += 1;
            }
        }
    }

    if union == 0 {
        Ok(1.0) // both all-zero
    } else {
        Ok(intersection as f64 / union as f64)
    }
}

/// Find the k nearest neighbors by cosine similarity.
///
/// Returns indices sorted by decreasing similarity.
pub fn knn_cosine(
    query: &PackedTernary,
    candidates: &[PackedTernary],
    k: usize,
) -> Result<Vec<(usize, f64)>> {
    if candidates.is_empty() {
        return Ok(vec![]);
    }

    let mut scores: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let sim = cosine_similarity(query, c).unwrap_or(0.0);
            (i, sim)
        })
        .collect();

    // Sort descending by similarity.
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(k);

    Ok(scores)
}

/// Ternary sign agreement ratio.
///
/// Counts positions where both vectors have the same sign (ignoring zeros).
/// Useful for measuring weight alignment in federated learning.
pub fn sign_agreement(a: &PackedTernary, b: &PackedTernary) -> Result<f64> {
    if a.len != b.len {
        return Err(CalculusError::DimensionMismatch {
            expected: a.len,
            got: b.len,
        });
    }

    let vals_a = unpack(a);
    let vals_b = unpack(b);

    let mut agree = 0usize;
    let mut total = 0usize;

    for (va, vb) in vals_a.iter().zip(vals_b.iter()) {
        if *va != TernaryValue::Zero && *vb != TernaryValue::Zero {
            total += 1;
            if *va == *vb {
                agree += 1;
            }
        }
    }

    if total == 0 {
        Ok(1.0)
    } else {
        Ok(agree as f64 / total as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_triton::packing::pack;

    #[test]
    fn cosine_identical() {
        let v = pack(&[TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Pos]);
        let sim = cosine_similarity(&v, &v).unwrap();
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn cosine_opposite() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Pos]);
        let b = pack(&[TernaryValue::Neg, TernaryValue::Neg]);
        let sim = cosine_similarity(&a, &b).unwrap();
        assert!((sim - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn cosine_orthogonal() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Zero]);
        let b = pack(&[TernaryValue::Zero, TernaryValue::Pos]);
        let sim = cosine_similarity(&a, &b).unwrap();
        assert!((sim - 0.0).abs() < 1e-10);
    }

    #[test]
    fn jaccard_identical() {
        let v = pack(&[TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Pos]);
        assert!((jaccard_index(&v, &v).unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn jaccard_disjoint() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Zero]);
        let b = pack(&[TernaryValue::Zero, TernaryValue::Neg]);
        let jaccard = jaccard_index(&a, &b).unwrap();
        assert_eq!(jaccard, 0.0);
    }

    #[test]
    fn knn_search() {
        let query = pack(&[TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Pos]);
        let candidates = vec![
            pack(&[TernaryValue::Neg, TernaryValue::Neg, TernaryValue::Neg]), // opposite
            pack(&[TernaryValue::Pos, TernaryValue::Pos, TernaryValue::Pos]), // identical
            pack(&[TernaryValue::Pos, TernaryValue::Zero, TernaryValue::Pos]), // partial
        ];

        let result = knn_cosine(&query, &candidates, 2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, 1); // identical should be first
    }

    #[test]
    fn sign_agreement_full() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Neg]);
        let b = pack(&[TernaryValue::Pos, TernaryValue::Neg]);
        assert_eq!(sign_agreement(&a, &b).unwrap(), 1.0);
    }

    #[test]
    fn sign_agreement_none() {
        let a = pack(&[TernaryValue::Pos, TernaryValue::Pos]);
        let b = pack(&[TernaryValue::Neg, TernaryValue::Neg]);
        assert_eq!(sign_agreement(&a, &b).unwrap(), 0.0);
    }
}
