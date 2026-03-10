use ed25519_dalek::{Signer as Ed25519Signer, Verifier as Ed25519Verifier};
use ml_dsa::{KeyGen, MlDsa65};
use serde::{Deserialize, Serialize};

use crate::error::{CryptoError, Result};

/// Which signature schemes are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureScheme {
    /// Classical Ed25519 only (backwards compatible).
    Ed25519Only = 0x01,
    /// Hybrid: Ed25519 + ML-DSA-65 (post-quantum resistant).
    Hybrid = 0x02,
}

/// A hybrid signature containing Ed25519 + optional ML-DSA-65.
///
/// Wire format:
/// ```text
/// ┌────────┬──────────────┬──────────────────┐
/// │ scheme │ ed25519_sig  │ ml_dsa_sig       │
/// │ 1 byte │ 64 bytes     │ 0 or N bytes     │
/// └────────┴──────────────┴──────────────────┘
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSignature {
    pub scheme: SignatureScheme,
    pub ed25519_sig: Vec<u8>,
    pub ml_dsa_sig: Option<Vec<u8>>,
}

impl HybridSignature {
    pub fn to_bytes(&self) -> Vec<u8> {
        let ml_dsa_len = self.ml_dsa_sig.as_ref().map_or(0, |s| s.len());
        let mut buf = Vec::with_capacity(1 + 64 + ml_dsa_len);
        buf.push(self.scheme as u8);
        buf.extend_from_slice(&self.ed25519_sig);
        if let Some(ref pq_sig) = self.ml_dsa_sig {
            buf.extend_from_slice(pq_sig);
        }
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 65 {
            return Err(CryptoError::ValidationFailed(
                "hybrid signature too short".into(),
            ));
        }

        let scheme = match data[0] {
            0x01 => SignatureScheme::Ed25519Only,
            0x02 => SignatureScheme::Hybrid,
            other => {
                return Err(CryptoError::ValidationFailed(format!(
                    "unknown signature scheme: {other:#x}"
                )))
            }
        };

        let ed25519_sig = data[1..65].to_vec();

        let ml_dsa_sig = if data.len() > 65 {
            Some(data[65..].to_vec())
        } else {
            None
        };

        Ok(Self {
            scheme,
            ed25519_sig,
            ml_dsa_sig,
        })
    }
}

/// Public verifying keys for hybrid verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridVerifyingKey {
    /// Ed25519 verifying key (32 bytes).
    pub ed25519_key: [u8; 32],
    /// ML-DSA-65 verifying key bytes, None if Ed25519-only.
    pub ml_dsa_key: Option<Vec<u8>>,
}

/// Private signing keys for hybrid signing.
pub struct HybridSigningKey {
    ed25519: ed25519_dalek::SigningKey,
    ml_dsa: Option<ml_dsa::KeyPair<MlDsa65>>,
}

impl HybridSigningKey {
    /// Generate a new Ed25519-only signing key.
    pub fn generate_ed25519() -> (Self, HybridVerifyingKey) {
        let ed25519 = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let ed25519_vk = ed25519.verifying_key();

        let signing = Self {
            ed25519,
            ml_dsa: None,
        };
        let verifying = HybridVerifyingKey {
            ed25519_key: ed25519_vk.to_bytes(),
            ml_dsa_key: None,
        };

        (signing, verifying)
    }

    /// Generate a new hybrid (Ed25519 + ML-DSA-65) signing key.
    pub fn generate_hybrid() -> (Self, HybridVerifyingKey) {
        let ed25519 = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let ed25519_vk = ed25519.verifying_key();

        // ML-DSA uses rand 0.10 / rand_core 0.10 ecosystem
        let ml_dsa_kp = MlDsa65::key_gen(&mut rand_pq::rng());

        // Encode verifying key to bytes for transmission
        let ml_dsa_vk_bytes = ml_dsa_kp.verifying_key().encode().as_slice().to_vec();

        let signing = Self {
            ed25519,
            ml_dsa: Some(ml_dsa_kp),
        };
        let verifying = HybridVerifyingKey {
            ed25519_key: ed25519_vk.to_bytes(),
            ml_dsa_key: Some(ml_dsa_vk_bytes),
        };

        (signing, verifying)
    }

    /// Sign data with all available schemes.
    pub fn sign(&self, data: &[u8]) -> HybridSignature {
        let ed25519_sig = Ed25519Signer::sign(&self.ed25519, data)
            .to_bytes()
            .to_vec();

        let (scheme, ml_dsa_sig) = if let Some(ref kp) = self.ml_dsa {
            // Use ML-DSA randomized signing (trait from signature v3 via ml-dsa)
            use ml_dsa::signature::RandomizedSigner;
            let sig = kp.signing_key().sign_with_rng(&mut rand_pq::rng(), data);
            let sig_bytes = sig.encode().as_slice().to_vec();
            (SignatureScheme::Hybrid, Some(sig_bytes))
        } else {
            (SignatureScheme::Ed25519Only, None)
        };

        HybridSignature {
            scheme,
            ed25519_sig,
            ml_dsa_sig,
        }
    }

    /// Get the Ed25519 signing key (for backwards-compatible operations).
    pub fn ed25519_key(&self) -> &ed25519_dalek::SigningKey {
        &self.ed25519
    }

    /// Check if this key supports hybrid signing.
    pub fn is_hybrid(&self) -> bool {
        self.ml_dsa.is_some()
    }

    /// Get the signature scheme this key supports.
    pub fn scheme(&self) -> SignatureScheme {
        if self.ml_dsa.is_some() {
            SignatureScheme::Hybrid
        } else {
            SignatureScheme::Ed25519Only
        }
    }
}

// Security note: With the "zeroize" feature enabled on ed25519-dalek,
// `SigningKey` implements `ZeroizeOnDrop` — the secret scalar is automatically
// scrubbed when this struct is dropped. No manual Drop impl needed for Ed25519.
//
// ML-DSA keypair does not yet implement ZeroizeOnDrop; the memory will be
// zeroed when the OS reclaims the page. This is a known limitation until
// the ml-dsa crate adds zeroize support.

impl HybridVerifyingKey {
    /// Verify a hybrid signature.
    ///
    /// - Ed25519 signature is always verified.
    /// - ML-DSA is verified only if this key has ML-DSA support AND the signature
    ///   includes ML-DSA data.
    /// - If this key requires hybrid but the signature is Ed25519-only,
    ///   verification fails (downgrade attack prevention).
    pub fn verify(&self, data: &[u8], sig: &HybridSignature) -> Result<()> {
        // 1. Always verify Ed25519
        let ed25519_vk = ed25519_dalek::VerifyingKey::from_bytes(&self.ed25519_key)
            .map_err(|e| CryptoError::ValidationFailed(format!("invalid Ed25519 key: {e}")))?;
        let ed25519_sig_bytes: [u8; 64] = sig
            .ed25519_sig
            .as_slice()
            .try_into()
            .map_err(|_| {
                CryptoError::ValidationFailed("invalid Ed25519 signature length".into())
            })?;
        let ed25519_sig = ed25519_dalek::Signature::from_bytes(&ed25519_sig_bytes);
        Ed25519Verifier::verify(&ed25519_vk, data, &ed25519_sig).map_err(|_| {
            CryptoError::ValidationFailed("Ed25519 signature verification failed".into())
        })?;

        // 2. If we have ML-DSA key, require ML-DSA signature (prevent downgrade)
        if let Some(ref ml_dsa_vk_bytes) = self.ml_dsa_key {
            let pq_sig_bytes = sig.ml_dsa_sig.as_ref().ok_or_else(|| {
                CryptoError::ValidationFailed(
                    "hybrid key requires ML-DSA signature (downgrade rejected)".into(),
                )
            })?;

            // Reconstruct verifying key from bytes
            let vk_array =
                hybrid_array_pq::Array::try_from(ml_dsa_vk_bytes.as_slice()).map_err(|_| {
                    CryptoError::ValidationFailed("invalid ML-DSA key length".into())
                })?;
            let ml_dsa_vk = ml_dsa::VerifyingKey::<MlDsa65>::decode(&vk_array);

            // Reconstruct signature from bytes
            let sig_array =
                hybrid_array_pq::Array::try_from(pq_sig_bytes.as_slice()).map_err(|_| {
                    CryptoError::ValidationFailed("invalid ML-DSA signature length".into())
                })?;
            let ml_dsa_sig =
                ml_dsa::Signature::<MlDsa65>::decode(&sig_array).ok_or_else(|| {
                    CryptoError::ValidationFailed("invalid ML-DSA signature".into())
                })?;

            // Use the Verifier trait from signature v3 (via ml-dsa)
            {
                use ml_dsa::signature::Verifier as PqVerifier;
                PqVerifier::verify(&ml_dsa_vk, data, &ml_dsa_sig).map_err(|_| {
                    CryptoError::ValidationFailed(
                        "ML-DSA signature verification failed".into(),
                    )
                })?;
            }
        }

        Ok(())
    }

    /// Get the signature scheme this key supports.
    pub fn scheme(&self) -> SignatureScheme {
        if self.ml_dsa_key.is_some() {
            SignatureScheme::Hybrid
        } else {
            SignatureScheme::Ed25519Only
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ed25519_only_sign_verify() {
        let (signing, verifying) = HybridSigningKey::generate_ed25519();
        let data = b"test message";

        let sig = signing.sign(data);
        assert_eq!(sig.scheme, SignatureScheme::Ed25519Only);
        assert!(sig.ml_dsa_sig.is_none());

        verifying.verify(data, &sig).unwrap();
    }

    #[test]
    fn hybrid_sign_verify() {
        let (signing, verifying) = HybridSigningKey::generate_hybrid();
        let data = b"hybrid test message";

        let sig = signing.sign(data);
        assert_eq!(sig.scheme, SignatureScheme::Hybrid);
        assert!(sig.ml_dsa_sig.is_some());

        verifying.verify(data, &sig).unwrap();
    }

    #[test]
    fn tampered_data_fails_ed25519() {
        let (signing, verifying) = HybridSigningKey::generate_ed25519();
        let sig = signing.sign(b"original");

        let result = verifying.verify(b"tampered", &sig);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_data_fails_hybrid() {
        let (signing, verifying) = HybridSigningKey::generate_hybrid();
        let sig = signing.sign(b"original");

        let result = verifying.verify(b"tampered", &sig);
        assert!(result.is_err());
    }

    #[test]
    fn downgrade_attack_rejected() {
        let (signing_hybrid, verifying_hybrid) = HybridSigningKey::generate_hybrid();

        let full_sig = signing_hybrid.sign(b"data");
        let downgraded = HybridSignature {
            scheme: SignatureScheme::Ed25519Only,
            ed25519_sig: full_sig.ed25519_sig.clone(),
            ml_dsa_sig: None,
        };

        let result = verifying_hybrid.verify(b"data", &downgraded);
        assert!(result.is_err());
    }

    #[test]
    fn signature_serialization_roundtrip_ed25519() {
        let (signing, _) = HybridSigningKey::generate_ed25519();
        let sig = signing.sign(b"data");

        let bytes = sig.to_bytes();
        let restored = HybridSignature::from_bytes(&bytes).unwrap();

        assert_eq!(restored.scheme, SignatureScheme::Ed25519Only);
        assert_eq!(restored.ed25519_sig, sig.ed25519_sig);
        assert!(restored.ml_dsa_sig.is_none());
    }

    #[test]
    fn signature_serialization_roundtrip_hybrid() {
        let (signing, verifying) = HybridSigningKey::generate_hybrid();
        let sig = signing.sign(b"roundtrip");

        let bytes = sig.to_bytes();
        let restored = HybridSignature::from_bytes(&bytes).unwrap();

        assert_eq!(restored.scheme, SignatureScheme::Hybrid);
        assert_eq!(restored.ed25519_sig, sig.ed25519_sig);
        assert!(restored.ml_dsa_sig.is_some());

        verifying.verify(b"roundtrip", &restored).unwrap();
    }

    #[test]
    fn ed25519_key_compatible_with_standalone() {
        let (signing, _) = HybridSigningKey::generate_ed25519();

        let raw_sig = Ed25519Signer::sign(signing.ed25519_key(), b"direct");
        let vk = signing.ed25519_key().verifying_key();
        Ed25519Verifier::verify(&vk, b"direct", &raw_sig).unwrap();
    }

    #[test]
    fn scheme_detection() {
        let (ed_key, ed_vk) = HybridSigningKey::generate_ed25519();
        assert_eq!(ed_key.scheme(), SignatureScheme::Ed25519Only);
        assert_eq!(ed_vk.scheme(), SignatureScheme::Ed25519Only);
        assert!(!ed_key.is_hybrid());

        let (hybrid_key, hybrid_vk) = HybridSigningKey::generate_hybrid();
        assert_eq!(hybrid_key.scheme(), SignatureScheme::Hybrid);
        assert_eq!(hybrid_vk.scheme(), SignatureScheme::Hybrid);
        assert!(hybrid_key.is_hybrid());
    }
}
