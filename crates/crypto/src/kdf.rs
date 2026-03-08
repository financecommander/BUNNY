use hkdf::Hkdf;
use sha2::Sha256;

use crate::error::{CryptoError, Result};
use crate::types::SymmetricKey;

const SALT: &[u8] = b"bunny-swarm-v1";

/// Derive a 256-bit symmetric key from combined key material.
///
/// `ikm`: Input key material (e.g., X25519 shared secret || ML-KEM shared secret)
/// `info`: Context binding (e.g., "session-key" or "agent-id:session-id")
pub fn derive_session_key(ikm: &[u8], info: &[u8]) -> Result<SymmetricKey> {
    let hk = Hkdf::<Sha256>::new(Some(SALT), ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm)
        .map_err(|_| CryptoError::KdfFailed)?;
    Ok(SymmetricKey::from_bytes(okm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_produces_32_byte_key() {
        let ikm = b"some-shared-secret-material";
        let key = derive_session_key(ikm, b"session-key").unwrap();
        assert_eq!(key.as_bytes().len(), 32);
    }

    #[test]
    fn different_info_produces_different_keys() {
        let ikm = b"same-ikm";
        let key1 = derive_session_key(ikm, b"info-a").unwrap();
        let key2 = derive_session_key(ikm, b"info-b").unwrap();
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn deterministic_output() {
        let ikm = b"deterministic-test";
        let key1 = derive_session_key(ikm, b"ctx").unwrap();
        let key2 = derive_session_key(ikm, b"ctx").unwrap();
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }
}
