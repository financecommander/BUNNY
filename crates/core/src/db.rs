use tracing::info;

/// Database wrapper stub.
///
/// Will be backed by Turso/libsql when the `libsql` dependency is added.
pub struct BunnyDb {
    url: String,
}

impl BunnyDb {
    pub async fn new() -> anyhow::Result<Self> {
        let url = std::env::var("TURSO_URL").unwrap_or_else(|_| "file:./data/bunny.db".into());
        info!("database stub initialized: {url}");
        Ok(Self { url })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub async fn save_protected_ips(&self, user_id: &str, ips: &[String]) -> anyhow::Result<()> {
        info!(user = user_id, count = ips.len(), "saved protected IPs (stub)");
        Ok(())
    }
}
