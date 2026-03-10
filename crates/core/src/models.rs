use serde::{Deserialize, Serialize};

/// A device protected by the BUNNY network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedDevice {
    pub device_id: String,
    pub ip_address: String,
    pub hostname: Option<String>,
    pub protected: bool,
}

impl ProtectedDevice {
    pub fn new(device_id: String, ip_address: String) -> Self {
        Self {
            device_id,
            ip_address,
            hostname: None,
            protected: false,
        }
    }
}
