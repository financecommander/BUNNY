pub mod firecracker;
pub mod threat_hunter;

pub use firecracker::detonate_in_sandbox;

use tracing::info;
use bunny_core::vector::BunnyVectorStore;

pub async fn startup() -> anyhow::Result<()> {
    let vector_store = BunnyVectorStore::new("./data/lance").await?;
    tokio::spawn(threat_hunter::start_threat_hunter(vector_store));
    info!("Sandbox + Threat Hunter ready");
    Ok(())
}
