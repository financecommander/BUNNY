use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Database(String),

    #[error("vector store error: {0}")]
    VectorStore(String),

    #[error("licensing error: {0}")]
    Licensing(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
}
