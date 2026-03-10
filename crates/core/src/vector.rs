use tracing::info;

/// Vector store wrapper for LanceDB.
pub struct BunnyVectorStore {
    _path: String,
}

impl BunnyVectorStore {
    pub async fn new(path: &str) -> anyhow::Result<Self> {
        info!(path = path, "vector store initialized");
        Ok(Self {
            _path: path.to_string(),
        })
    }
}
