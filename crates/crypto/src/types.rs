use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Protocol version for forward compatibility.
pub const PROTOCOL_VERSION: u8 = 1;

/// 256-bit symmetric key for AEAD ciphers.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct SymmetricKey(pub(crate) [u8; 32]);

impl SymmetricKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for SymmetricKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SymmetricKey([REDACTED])")
    }
}

/// Unique identifier for a swarm agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub uuid::Uuid);

impl AgentId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }
}

/// Unique identifier for a cryptographic session between two agents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub [u8; 16]);

impl SessionId {
    pub fn new() -> Self {
        let mut bytes = [0u8; 16];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// AEAD nonce (12 bytes for both AES-256-GCM and ChaCha20-Poly1305).
pub type Nonce = [u8; 12];
