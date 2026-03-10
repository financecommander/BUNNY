use thiserror::Error;

pub type Result<T> = std::result::Result<T, CalculusError>;

#[derive(Debug, Error)]
pub enum CalculusError {
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("empty input")]
    EmptyInput,

    #[error("invalid threshold: {0}")]
    InvalidThreshold(String),

    #[error("division by zero")]
    DivisionByZero,

    #[error("insufficient data: need {needed}, have {have}")]
    InsufficientData { needed: usize, have: usize },

    #[error("triton error: {0}")]
    Triton(#[from] bunny_triton::TritonError),
}
