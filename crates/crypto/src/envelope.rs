use serde::{Deserialize, Serialize};

use crate::cipher::{CipherSuite, SwarmCipher};
use crate::error::{CryptoError, Result};
use crate::types::{AgentId, Nonce, SessionId, PROTOCOL_VERSION};

/// Encrypted message envelope for swarm communication.
///
/// Wire format (big-endian):
/// ```text
/// ┌─────────┬────────┬───────────┬──────────┬──────────┬────────────┬───────┬──────────────┐
/// │ version │ suite  │ sender_id │ session  │ sequence │ timestamp  │ nonce │  ciphertext  │
/// │  1 byte │ 1 byte │ 16 bytes  │ 16 bytes │ 8 bytes  │  8 bytes   │ 12 B  │  N bytes     │
/// └─────────┴────────┴───────────┴──────────┴──────────┴────────────┴───────┴──────────────┘
/// ```
///
/// AAD = version || suite || sender_id || session_id || sequence || timestamp
/// Ciphertext includes the 16-byte AEAD authentication tag appended by the cipher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmEnvelope {
    pub version: u8,
    pub suite: CipherSuite,
    pub sender: AgentId,
    pub session_id: SessionId,
    pub nonce: Nonce,
    pub ciphertext: Vec<u8>,
    pub sequence: u64,
    pub timestamp_ms: u64,
}

/// Fixed header size: 1 + 1 + 16 + 16 + 8 + 8 + 12 = 62 bytes
const HEADER_SIZE: usize = 62;

impl SwarmEnvelope {
    /// Seal a plaintext payload into an encrypted envelope.
    pub fn seal(
        cipher: &SwarmCipher,
        sender: &AgentId,
        session_id: &SessionId,
        sequence: u64,
        payload: &[u8],
    ) -> Result<Self> {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let nonce = SwarmCipher::generate_nonce();

        let mut envelope = Self {
            version: PROTOCOL_VERSION,
            suite: cipher.suite(),
            sender: sender.clone(),
            session_id: session_id.clone(),
            nonce,
            ciphertext: Vec::new(),
            sequence,
            timestamp_ms,
        };

        let aad = envelope.aad();
        envelope.ciphertext = cipher.encrypt(&nonce, payload, &aad)?;

        Ok(envelope)
    }

    /// Open an envelope, decrypting and authenticating the payload.
    pub fn open(&self, cipher: &SwarmCipher) -> Result<Vec<u8>> {
        if self.version != PROTOCOL_VERSION {
            return Err(CryptoError::ProtocolVersionMismatch {
                expected: PROTOCOL_VERSION,
                actual: self.version,
            });
        }
        let aad = self.aad();
        cipher.decrypt(&self.nonce, &self.ciphertext, &aad)
    }

    /// Compute the AAD bytes for this envelope.
    fn aad(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(50);
        aad.push(self.version);
        aad.push(self.suite.as_u8());
        aad.extend_from_slice(self.sender.as_bytes());
        aad.extend_from_slice(self.session_id.as_bytes());
        aad.extend_from_slice(&self.sequence.to_be_bytes());
        aad.extend_from_slice(&self.timestamp_ms.to_be_bytes());
        aad
    }

    /// Serialize to binary wire format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_SIZE + self.ciphertext.len());
        buf.push(self.version);
        buf.push(self.suite.as_u8());
        buf.extend_from_slice(self.sender.as_bytes());
        buf.extend_from_slice(self.session_id.as_bytes());
        buf.extend_from_slice(&self.sequence.to_be_bytes());
        buf.extend_from_slice(&self.timestamp_ms.to_be_bytes());
        buf.extend_from_slice(&self.nonce);
        buf.extend_from_slice(&self.ciphertext);
        buf
    }

    /// Deserialize from binary wire format.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            return Err(CryptoError::InvalidEnvelope(format!(
                "too short: {} bytes, need at least {}",
                data.len(),
                HEADER_SIZE
            )));
        }

        let version = data[0];
        let suite = CipherSuite::from_u8(data[1])?;

        let mut sender_bytes = [0u8; 16];
        sender_bytes.copy_from_slice(&data[2..18]);

        let mut session_bytes = [0u8; 16];
        session_bytes.copy_from_slice(&data[18..34]);

        let sequence = u64::from_be_bytes(data[34..42].try_into().unwrap());
        let timestamp_ms = u64::from_be_bytes(data[42..50].try_into().unwrap());

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&data[50..62]);

        let ciphertext = data[62..].to_vec();

        Ok(Self {
            version,
            suite,
            sender: AgentId(uuid::Uuid::from_bytes(sender_bytes)),
            session_id: SessionId(session_bytes),
            nonce,
            ciphertext,
            sequence,
            timestamp_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SymmetricKey;

    fn test_cipher() -> SwarmCipher {
        SwarmCipher::with_suite(&SymmetricKey::from_bytes([0x42; 32]), CipherSuite::Aes256Gcm)
    }

    #[test]
    fn seal_and_open_roundtrip() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();
        let payload = b"inference request payload";

        let envelope = SwarmEnvelope::seal(&cipher, &sender, &session, 1, payload).unwrap();
        let decrypted = envelope.open(&cipher).unwrap();
        assert_eq!(decrypted, payload);
    }

    #[test]
    fn binary_serialization_roundtrip() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();

        let envelope =
            SwarmEnvelope::seal(&cipher, &sender, &session, 42, b"test payload").unwrap();
        let bytes = envelope.to_bytes();
        let restored = SwarmEnvelope::from_bytes(&bytes).unwrap();

        assert_eq!(restored.version, envelope.version);
        assert_eq!(restored.suite, envelope.suite);
        assert_eq!(restored.sender, envelope.sender);
        assert_eq!(restored.session_id, envelope.session_id);
        assert_eq!(restored.sequence, envelope.sequence);
        assert_eq!(restored.nonce, envelope.nonce);
        assert_eq!(restored.ciphertext, envelope.ciphertext);

        // Verify decryption still works after deserialization
        let decrypted = restored.open(&cipher).unwrap();
        assert_eq!(decrypted, b"test payload");
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();

        let mut envelope =
            SwarmEnvelope::seal(&cipher, &sender, &session, 1, b"secret").unwrap();
        if let Some(byte) = envelope.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        assert!(envelope.open(&cipher).is_err());
    }

    #[test]
    fn tampered_sequence_fails() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();

        let mut envelope =
            SwarmEnvelope::seal(&cipher, &sender, &session, 1, b"secret").unwrap();
        envelope.sequence = 999; // tamper with AAD field
        assert!(envelope.open(&cipher).is_err());
    }

    #[test]
    fn too_short_bytes_rejected() {
        assert!(SwarmEnvelope::from_bytes(&[0u8; 10]).is_err());
    }
}
