use crate::models::ProtectedDevice;
use bunny_core::db::BunnyDb;
use stripe::{Client, CreateCustomer, Customer, CreateSubscription, Subscription};
use tracing::info;

pub struct Licensing {
    db: BunnyDb,
    stripe_client: Client,
}

impl Licensing {
    pub async fn new() -> anyhow::Result<Self> {
        let db = BunnyDb::new().await?;
        let stripe_client = Client::new(std::env::var("STRIPE_SECRET_KEY").expect("STRIPE_SECRET_KEY required"));
        Ok(Self { db, stripe_client })
    }

    pub async fn scan_and_upsell(&self, devices: Vec<ProtectedDevice>) -> String {
        let total = devices.len();
        let protected = devices.iter().filter(|d| d.protected).count();
        let extra = total.saturating_sub(1); // first IP always free

        if extra > 0 {
            info!("💰 {} extra IPs detected — $1 each = ${}/month", extra, extra);
            format!("You have {} devices. First IP free. Protecting all would cost ${}/month.\nTap to upgrade.", total, extra)
        } else {
            "✅ First IP protected free forever".to_string()
        }
    }

    pub async fn save_protected_ips(&self, user_id: &str, ips: Vec<String>) -> anyhow::Result<()> {
        self.db.save_protected_ips(user_id, &ips).await?;
        // Create Stripe subscription if extra IPs
        if ips.len() > 1 {
            let customer = Customer::create(&self.stripe_client, CreateCustomer::new()).await?;
            let _sub = Subscription::create(&self.stripe_client, CreateSubscription::new(customer.id, vec![])).await?;
        }
        Ok(())
    }
}
