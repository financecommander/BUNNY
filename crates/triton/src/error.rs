use thiserror::Error;

pub type Result<T> = std::result::Result<T, TritonError>;

#[derive(Debug, Error)]
pub enum TritonError {
    #[error("invalid ternary value: {0}")]
    InvalidValue(i8),

    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("invalid model format: {0}")]
    InvalidFormat(String),

    #[error("invalid magic bytes: expected BNNY")]
    InvalidMagic,

    #[error("layer index out of bounds: {index} (model has {total} layers)")]
    LayerOutOfBounds { index: usize, total: usize },

    #[error("empty model: no layers")]
    EmptyModel,

    #[error("shard error: {0}")]
    ShardError(String),

    #[error("safetensors error: {0}")]
    SafetensorsError(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}
