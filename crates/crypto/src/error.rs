use thiserror::Error;

use crate::types::SessionId;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("AEAD encryption failed")]
    EncryptionFailed,

    #[error("AEAD decryption failed: authentication tag mismatch")]
    DecryptionFailed,

    #[error("key exchange failed: {0}")]
    KeyExchangeFailed(String),

    #[error("ML-KEM encapsulation failed")]
    KemEncapsulationFailed,

    #[error("ML-KEM decapsulation failed")]
    KemDecapsulationFailed,

    #[error("HKDF expand failed: invalid length")]
    KdfFailed,

    #[error("invalid envelope: {0}")]
    InvalidEnvelope(String),

    #[error("session not found: {0:?}")]
    SessionNotFound(SessionId),

    #[error("session expired")]
    SessionExpired,

    #[error("unsupported cipher suite: {0}")]
    UnsupportedCipherSuite(u8),

    #[error("invalid protocol version: expected {expected}, got {actual}")]
    ProtocolVersionMismatch { expected: u8, actual: u8 },

    #[error("gateway routing error: {0}")]
    GatewayRoutingError(String),

    #[error("payload validation failed: {0}")]
    ValidationFailed(String),
}

pub type Result<T> = std::result::Result<T, CryptoError>;
