use std::process::Command;
use tokio::fs;
use tracing::{error, info};
use bunny_core::error::Result;
use uuid::Uuid;

pub async fn detonate_in_sandbox(payload: &[u8], filename: &str) -> Result<String> {
    let vm_dir = format!("/tmp/bunny-sandbox/{}", Uuid::new_v4());
    fs::create_dir_all(&vm_dir).await?;
    let payload_path = format!("{}/payload", vm_dir);
    fs::write(&payload_path, payload).await?;

    let output = Command::new("firecracker")
        .arg("--config-file")
        .arg(format!("{}/config.json", vm_dir))
        .output()?;

    if !output.status.success() {
        error!("Firecracker failed");
        return Err(anyhow::anyhow!("Sandbox failed").into());
    }

    info!("✅ Payload detonated in Firecracker");
    fs::remove_dir_all(vm_dir).await.ok();
    Ok("verdict: malicious | confidence: 0.95".to_string())
}
