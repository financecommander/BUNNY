use thiserror::Error;

pub type Result<T> = std::result::Result<T, NetworkError>;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("connection error: {0}")]
    Connection(String),

    #[error("handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("framing error: {0}")]
    FramingError(String),

    #[error("invalid frame type: {0:#x}")]
    InvalidFrameType(u8),

    #[error("server error: {0}")]
    ServerError(String),

    #[error("crypto error: {0}")]
    CryptoError(#[from] bunny_crypto::CryptoError),

    #[error("quinn connection error: {0}")]
    QuinnConnection(#[from] quinn::ConnectionError),

    #[error("quinn write error: {0}")]
    QuinnWrite(#[from] quinn::WriteError),

    #[error("quinn read error: {0}")]
    QuinnReadError(#[from] quinn::ReadError),

    #[error("quinn read-to-end error: {0}")]
    QuinnReadToEnd(#[from] quinn::ReadToEndError),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("timeout")]
    Timeout,

    #[error("shutdown")]
    Shutdown,
}
