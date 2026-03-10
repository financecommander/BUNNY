use crate::db::BunnyDb;
use crate::models::ProtectedDevice;
use tracing::info;

/// Licensing and monetization module.
///
/// Stripe integration is stubbed — will be enabled when `async-stripe`
/// dependency is added back.
pub struct Licensing {
    db: BunnyDb,
}

impl Licensing {
    pub async fn new() -> anyhow::Result<Self> {
        let db = BunnyDb::new().await?;
        Ok(Self { db })
    }

    pub async fn scan_and_upsell(&self, devices: Vec<ProtectedDevice>) -> String {
        let total = devices.len();
        let extra = total.saturating_sub(1);

        if extra > 0 {
            info!("{} extra IPs detected - $1 each = ${}/month", extra, extra);
            format!(
                "You have {} devices. First IP free. Protecting all would cost ${}/month.\nTap to upgrade.",
                total, extra
            )
        } else {
            "First IP protected free forever".to_string()
        }
    }

    pub async fn save_protected_ips(&self, user_id: &str, ips: Vec<String>) -> anyhow::Result<()> {
        self.db.save_protected_ips(user_id, &ips).await?;
        if ips.len() > 1 {
            info!("stripe subscription would be created for {} extra IPs", ips.len() - 1);
        }
        Ok(())
    }
}
