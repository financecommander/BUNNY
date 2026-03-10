use thiserror::Error;

pub type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("inference error: {0}")]
    Inference(String),

    #[error("model not loaded: {0}")]
    ModelNotLoaded(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("agent not found: {0}")]
    NotFound(String),

    #[error("dispatch error: {0}")]
    Dispatch(String),

    #[error("triton error: {0}")]
    Triton(#[from] bunny_triton::TritonError),

    #[error("crypto error: {0}")]
    Crypto(#[from] bunny_crypto::CryptoError),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
