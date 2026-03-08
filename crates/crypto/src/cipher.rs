use aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce as AesNonce};
use chacha20poly1305::{ChaCha20Poly1305, Nonce as ChachaNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{CryptoError, Result};
use crate::types::{Nonce, SymmetricKey};

/// Which AEAD cipher is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum CipherSuite {
    Aes256Gcm = 0x01,
    ChaCha20Poly1305 = 0x02,
}

impl CipherSuite {
    /// Detect the best cipher for this CPU at runtime.
    pub fn auto_detect() -> Self {
        if Self::has_aes_hardware() {
            CipherSuite::Aes256Gcm
        } else {
            CipherSuite::ChaCha20Poly1305
        }
    }

    /// Check for hardware AES support.
    fn has_aes_hardware() -> bool {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            cpufeatures::new!(cpuid_aes, "aes");
            return cpuid_aes::init().get();
        }

        #[cfg(target_arch = "aarch64")]
        {
            cpufeatures::new!(cpuid_aes, "aes");
            return cpuid_aes::init().get();
        }

        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        {
            return false;
        }
    }

    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0x01 => Ok(CipherSuite::Aes256Gcm),
            0x02 => Ok(CipherSuite::ChaCha20Poly1305),
            other => Err(CryptoError::UnsupportedCipherSuite(other)),
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

enum CipherInner {
    Aes(Aes256Gcm),
    ChaCha(ChaCha20Poly1305),
}

/// Unified AEAD cipher that wraps either AES-256-GCM or ChaCha20-Poly1305.
pub struct SwarmCipher {
    inner: CipherInner,
    suite: CipherSuite,
    key_copy: [u8; 32],
}

impl SwarmCipher {
    /// Create a cipher from a 32-byte key, auto-detecting the best algorithm.
    pub fn new(key: &SymmetricKey) -> Self {
        Self::with_suite(key, CipherSuite::auto_detect())
    }

    /// Create a cipher with a specific suite.
    pub fn with_suite(key: &SymmetricKey, suite: CipherSuite) -> Self {
        let inner = match suite {
            CipherSuite::Aes256Gcm => {
                CipherInner::Aes(Aes256Gcm::new(key.as_bytes().into()))
            }
            CipherSuite::ChaCha20Poly1305 => {
                CipherInner::ChaCha(ChaCha20Poly1305::new(key.as_bytes().into()))
            }
        };
        let mut key_copy = [0u8; 32];
        key_copy.copy_from_slice(key.as_bytes());
        Self {
            inner,
            suite,
            key_copy,
        }
    }

    /// Which cipher suite is active.
    pub fn suite(&self) -> CipherSuite {
        self.suite
    }

    /// Encrypt plaintext with associated data. Returns ciphertext with appended tag.
    pub fn encrypt(&self, nonce: &Nonce, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let payload = aead::Payload { msg: plaintext, aad };
        match &self.inner {
            CipherInner::Aes(cipher) => cipher
                .encrypt(AesNonce::from_slice(nonce), payload)
                .map_err(|_| CryptoError::EncryptionFailed),
            CipherInner::ChaCha(cipher) => cipher
                .encrypt(ChachaNonce::from_slice(nonce), payload)
                .map_err(|_| CryptoError::EncryptionFailed),
        }
    }

    /// Decrypt ciphertext with associated data.
    pub fn decrypt(&self, nonce: &Nonce, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let payload = aead::Payload {
            msg: ciphertext,
            aad,
        };
        match &self.inner {
            CipherInner::Aes(cipher) => cipher
                .decrypt(AesNonce::from_slice(nonce), payload)
                .map_err(|_| CryptoError::DecryptionFailed),
            CipherInner::ChaCha(cipher) => cipher
                .decrypt(ChachaNonce::from_slice(nonce), payload)
                .map_err(|_| CryptoError::DecryptionFailed),
        }
    }

    /// Generate a cryptographically random nonce.
    pub fn generate_nonce() -> Nonce {
        let mut nonce = [0u8; 12];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        nonce
    }
}

impl Drop for SwarmCipher {
    fn drop(&mut self) {
        self.key_copy.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> SymmetricKey {
        SymmetricKey::from_bytes([0x42; 32])
    }

    #[test]
    fn roundtrip_aes256gcm() {
        let cipher = SwarmCipher::with_suite(&test_key(), CipherSuite::Aes256Gcm);
        let nonce = SwarmCipher::generate_nonce();
        let plaintext = b"hello bunny swarm";
        let aad = b"metadata";

        let ciphertext = cipher.encrypt(&nonce, plaintext, aad).unwrap();
        let decrypted = cipher.decrypt(&nonce, &ciphertext, aad).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn roundtrip_chacha20() {
        let cipher = SwarmCipher::with_suite(&test_key(), CipherSuite::ChaCha20Poly1305);
        let nonce = SwarmCipher::generate_nonce();
        let plaintext = b"hello bunny swarm";
        let aad = b"metadata";

        let ciphertext = cipher.encrypt(&nonce, plaintext, aad).unwrap();
        let decrypted = cipher.decrypt(&nonce, &ciphertext, aad).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let cipher1 = SwarmCipher::with_suite(&test_key(), CipherSuite::Aes256Gcm);
        let cipher2 =
            SwarmCipher::with_suite(&SymmetricKey::from_bytes([0x99; 32]), CipherSuite::Aes256Gcm);
        let nonce = SwarmCipher::generate_nonce();

        let ciphertext = cipher1.encrypt(&nonce, b"secret", b"aad").unwrap();
        assert!(cipher2.decrypt(&nonce, &ciphertext, b"aad").is_err());
    }

    #[test]
    fn tampered_aad_fails() {
        let cipher = SwarmCipher::with_suite(&test_key(), CipherSuite::ChaCha20Poly1305);
        let nonce = SwarmCipher::generate_nonce();

        let ciphertext = cipher.encrypt(&nonce, b"secret", b"good-aad").unwrap();
        assert!(cipher.decrypt(&nonce, &ciphertext, b"bad-aad").is_err());
    }

    #[test]
    fn auto_detect_returns_valid_suite() {
        let suite = CipherSuite::auto_detect();
        assert!(suite == CipherSuite::Aes256Gcm || suite == CipherSuite::ChaCha20Poly1305);
    }

    #[test]
    fn cipher_suite_roundtrip() {
        assert_eq!(CipherSuite::from_u8(0x01).unwrap(), CipherSuite::Aes256Gcm);
        assert_eq!(
            CipherSuite::from_u8(0x02).unwrap(),
            CipherSuite::ChaCha20Poly1305
        );
        assert!(CipherSuite::from_u8(0xFF).is_err());
    }
}
