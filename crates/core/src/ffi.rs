/// Flutter FFI bridge – exposes Rust backend functions to Flutter via
/// `flutter_rust_bridge`. Phase 1 contains type definitions and stubs;
/// Phase 2 connects them to the real agent implementations.

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ThreatEvent {
    pub id: String,
    pub timestamp_ms: i64,
    pub agent_name: String,
    pub severity: u8, // 0=critical, 1=medium, 2=anomaly
    pub description: String,
    pub latitude: f64,
    pub longitude: f64,
    pub confidence: f32,
    pub target_ip: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IotDevice {
    pub id: String,
    pub ip: String,
    pub label: String,
    pub device_type: u8, // 0=iot, 1=mobile, 2=desktop, 3=unknown
    pub license_status: u8, // 0=free, 1=paid, 2=unlicensed
    pub protection_enabled: bool,
    pub latitude: f64,
    pub longitude: f64,
    pub blocked_attempts: u32,
}

#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub inference_count: u64,
    pub last_event_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ThreatVerdict {
    pub severity: u8, // 0=critical, 1=medium, 2=anomaly
    pub confidence: f32,
    pub latency_ms: f32,
    pub model_version: String,
}

// ── Exported FFI functions ────────────────────────────────────────────────────

/// Return currently active threats (last 30 events).
pub fn get_active_threats() -> Vec<ThreatEvent> {
    // TODO Phase 2: pull from ThreatHunterAgent ring buffer
    vec![]
}

/// Return all known IoT devices on the network.
pub fn get_iot_devices() -> Vec<IotDevice> {
    // TODO Phase 2: pull from IoTFirewallAgent device table
    vec![]
}

/// Return live status for each of the 5 BUNNY agents.
pub fn get_agent_status() -> Vec<AgentStatus> {
    // TODO Phase 2: query agent supervisor
    vec![
        AgentStatus { id: "threat-hunter".into(), name: "Threat Hunter".into(), active: true, inference_count: 0, last_event_ms: 0 },
        AgentStatus { id: "sandbox".into(), name: "Sandbox".into(), active: true, inference_count: 0, last_event_ms: 0 },
        AgentStatus { id: "iot-firewall".into(), name: "IoT Firewall".into(), active: true, inference_count: 0, last_event_ms: 0 },
        AgentStatus { id: "ai-guardian".into(), name: "AI Guardian".into(), active: true, inference_count: 0, last_event_ms: 0 },
        AgentStatus { id: "learning".into(), name: "Learning & Adaptation".into(), active: true, inference_count: 0, last_event_ms: 0 },
    ]
}

/// Run Triton ternary inference on raw packet bytes.
pub fn infer_threat(packet: Vec<u8>) -> ThreatVerdict {
    // TODO Phase 2: load .tern model and run 2-bit quantised forward pass
    let _ = packet;
    ThreatVerdict {
        severity: 2,
        confidence: 0.0,
        latency_ms: 0.0,
        model_version: "tern-v1.0-stub".into(),
    }
}

/// Return IPs currently covered by a paid or free licence.
pub fn get_licensed_ips() -> Vec<String> {
    // TODO Phase 2: query licensing::Licensing
    vec![]
}
