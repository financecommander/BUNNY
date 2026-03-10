use thiserror::Error;

pub type Result<T> = std::result::Result<T, PortalError>;

#[derive(Debug, Error)]
pub enum PortalError {
    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("model already loaded: {0}")]
    ModelAlreadyLoaded(String),

    #[error("worker not available for model: {0}")]
    NoWorkerAvailable(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("session expired")]
    SessionExpired,

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("dispatch timeout: task {0}")]
    DispatchTimeout(String),

    #[error("inference failed: {0}")]
    InferenceFailed(String),

    #[error("crypto error: {0}")]
    Crypto(#[from] bunny_crypto::CryptoError),

    #[error("triton error: {0}")]
    Triton(#[from] bunny_triton::TritonError),

    #[error("agent error: {0}")]
    Agent(#[from] bunny_agents::AgentError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("server error: {0}")]
    Server(String),
}
