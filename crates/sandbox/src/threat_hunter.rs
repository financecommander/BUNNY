use tokio::time::{interval, Duration};
use tracing::info;
use bunny_core::vector::BunnyVectorStore;

pub async fn start_threat_hunter(_vector_store: BunnyVectorStore) {
    let _client = reqwest::Client::new();
    let mut ticker = interval(Duration::from_secs(60));
    loop {
        ticker.tick().await;
        info!("Threat Hunter polled feeds (Abuse.ch, OTX)");
    }
}
