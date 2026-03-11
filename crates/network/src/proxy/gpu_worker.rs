//! GPU worker registration, identity, and runtime health for Phase 4.
//!
//! Implements Workstreams 1, 3, and 5 of the Phase 4 directive:
//! - WS1: GPU node manifest with GpuWorker trust policy
//! - WS3: Worker registration with coordinator + heartbeat
//! - WS5: Runtime health probe abstraction
//!
//! The GPU worker is a live inference node (swarm-gpu / 10.142.0.6)
//! that runs Ollama, Triton, or another local runtime. It registers
//! with swarm-mainframe (Coordinator) and receives signed encrypted
//! inference dispatch requests.

use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::channel::EncryptedChannel;
use super::node_identity::{NodeBinding, NodeManifest, SwarmRole};

// ── WS1: GPU Node Identity ──────────────────────────────────────────

/// Capability flags for a GPU worker node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuCapabilityFlags {
    pub supports_gpu_inference: bool,
    pub supports_gateway: bool,
    pub supports_orchestration: bool,
    pub supports_calculus_tools: bool,
}

impl GpuCapabilityFlags {
    /// Default flags for a GPU worker: only inference.
    pub fn gpu_worker() -> Self {
        Self {
            supports_gpu_inference: true,
            supports_gateway: false,
            supports_orchestration: false,
            supports_calculus_tools: false,
        }
    }
}

/// Labels attached to a GPU worker for routing/filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLabels {
    labels: Vec<String>,
}

impl WorkerLabels {
    /// Default labels for the L4 GPU worker.
    pub fn gpu_l4() -> Self {
        Self {
            labels: vec![
                "gpu".into(),
                "inference".into(),
                "worker".into(),
                "ollama".into(),
                "l4".into(),
            ],
        }
    }

    pub fn contains(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    pub fn add(&mut self, label: impl Into<String>) {
        let label = label.into();
        if !self.labels.contains(&label) {
            self.labels.push(label);
        }
    }

    pub fn as_slice(&self) -> &[String] {
        &self.labels
    }
}

/// Create the live GPU node manifest for swarm-gpu.
///
/// Role: GpuWorker
/// Internal: 10.142.0.6
/// Sidecar: 127.0.0.1:9003
pub fn create_gpu_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::GpuWorker,
        "swarm-gpu",
        NodeBinding {
            internal: "10.142.0.6:11434".parse().unwrap(),
            external: Some("35.227.111.161:11434".parse().unwrap()),
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
    .with_metadata("gpu", "nvidia-l4")
    .with_metadata("zone", "us-east1-b")
    .with_metadata("runtime", "ollama")
}

/// Create the coordinator manifest for swarm-mainframe.
pub fn create_mainframe_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::Coordinator,
        "swarm-mainframe",
        NodeBinding {
            internal: "10.142.0.4:3000".parse().unwrap(),
            external: Some("34.148.140.31:3000".parse().unwrap()),
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
}

// ── WS3: Worker Registration ────────────────────────────────────────

/// Worker registration state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerState {
    /// Registered but no encrypted channel yet.
    Registered,
    /// Encrypted channel established, waiting for health.
    Connected,
    /// Channel established + runtime healthy → ready for dispatch.
    Healthy,
    /// Channel up but runtime unhealthy → dispatch blocked.
    Degraded,
    /// Channel lost or heartbeat timeout → not available.
    Unavailable,
}

impl WorkerState {
    /// Whether the worker can accept inference dispatch.
    pub fn is_dispatchable(self) -> bool {
        self == Self::Healthy
    }
}

/// Registration record for a GPU worker in the coordinator.
pub struct GpuWorkerRegistration {
    /// Node name.
    pub name: String,
    /// Swarm role.
    pub role: SwarmRole,
    /// Sidecar address.
    pub sidecar_addr: SocketAddr,
    /// Capability flags.
    pub capabilities: GpuCapabilityFlags,
    /// Labels for routing.
    pub labels: WorkerLabels,
    /// Current worker state.
    pub state: WorkerState,
    /// Runtime health info (if known).
    pub runtime: Option<RuntimeHealthInfo>,
    /// Last heartbeat received.
    pub last_heartbeat: Option<Instant>,
    /// Heartbeat interval expected.
    pub heartbeat_interval: Duration,
    /// Encrypted channel to this worker (if established).
    pub channel: Option<EncryptedChannel>,
}

impl GpuWorkerRegistration {
    /// Create a new registration.
    pub fn new(
        name: impl Into<String>,
        sidecar_addr: SocketAddr,
    ) -> Self {
        Self {
            name: name.into(),
            role: SwarmRole::GpuWorker,
            sidecar_addr,
            capabilities: GpuCapabilityFlags::gpu_worker(),
            labels: WorkerLabels::gpu_l4(),
            state: WorkerState::Registered,
            runtime: None,
            last_heartbeat: None,
            heartbeat_interval: Duration::from_secs(30),
            channel: None,
        }
    }

    /// Record a heartbeat.
    pub fn record_heartbeat(&mut self, runtime_info: Option<RuntimeHealthInfo>) {
        self.last_heartbeat = Some(Instant::now());
        self.runtime = runtime_info.clone();

        // Update state based on channel + runtime health
        if self.channel.is_some() {
            match &runtime_info {
                Some(info) if info.healthy => {
                    self.state = WorkerState::Healthy;
                }
                Some(_) => {
                    self.state = WorkerState::Degraded;
                }
                None => {
                    // No runtime info means we can't verify — stay connected
                    self.state = WorkerState::Connected;
                }
            }
        }
    }

    /// Check if heartbeat has timed out.
    pub fn is_heartbeat_stale(&self) -> bool {
        match self.last_heartbeat {
            Some(last) => last.elapsed() > self.heartbeat_interval * 3,
            None => true, // never received a heartbeat
        }
    }

    /// Mark as unavailable (heartbeat timeout).
    pub fn mark_unavailable(&mut self) {
        self.state = WorkerState::Unavailable;
    }

    /// Establish encrypted channel.
    pub fn set_channel(&mut self, channel: EncryptedChannel) {
        self.channel = Some(channel);
        if self.state == WorkerState::Registered {
            self.state = WorkerState::Connected;
        }
    }

    /// Can this worker accept dispatch?
    pub fn is_dispatchable(&self) -> bool {
        self.state.is_dispatchable()
    }
}

// ── WS5: Runtime Health ─────────────────────────────────────────────

/// Kind of inference runtime on the GPU worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeKind {
    /// Ollama model serving.
    Ollama,
    /// Triton / TensorRT inference server.
    Triton,
    /// BUNNY native ternary engine.
    BunnyTernary,
    /// Other / unknown runtime.
    Other(String),
}

/// Information about a loaded model on the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedModel {
    /// Model name (e.g., "llama3.1:8b").
    pub name: String,
    /// Whether the model is ready for inference.
    pub ready: bool,
    /// Model size in bytes (if known).
    pub size_bytes: Option<u64>,
}

/// Runtime health information from a GPU worker.
///
/// Included in heartbeat messages so the coordinator knows
/// whether to dispatch inference to this worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHealthInfo {
    /// Whether the runtime is healthy overall.
    pub healthy: bool,
    /// Runtime kind.
    pub kind: RuntimeKind,
    /// Models currently loaded.
    pub loaded_models: Vec<LoadedModel>,
    /// GPU utilization (0.0 to 1.0).
    pub gpu_utilization: f32,
    /// GPU memory used in bytes.
    pub gpu_memory_used: u64,
    /// GPU memory total in bytes.
    pub gpu_memory_total: u64,
    /// Unix timestamp of the health check.
    pub checked_at_ms: u64,
}

impl RuntimeHealthInfo {
    /// Create a healthy Ollama runtime info.
    pub fn ollama_healthy(models: Vec<LoadedModel>) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            healthy: true,
            kind: RuntimeKind::Ollama,
            loaded_models: models,
            gpu_utilization: 0.0,
            gpu_memory_used: 0,
            gpu_memory_total: 24_000_000_000, // L4 ~24GB
            checked_at_ms: now,
        }
    }

    /// Create an unhealthy runtime (runtime unreachable).
    pub fn unhealthy(kind: RuntimeKind) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            healthy: false,
            kind,
            loaded_models: Vec::new(),
            gpu_utilization: 0.0,
            gpu_memory_used: 0,
            gpu_memory_total: 0,
            checked_at_ms: now,
        }
    }

    /// Check if a specific model is loaded and ready.
    pub fn has_model_ready(&self, model_name: &str) -> bool {
        self.loaded_models.iter().any(|m| m.name == model_name && m.ready)
    }

    /// Serialize for heartbeat payload.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from heartbeat payload.
    pub fn from_json(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

/// Heartbeat message from GPU worker → coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerHeartbeat {
    /// Worker node name.
    pub node_name: String,
    /// Unix timestamp.
    pub timestamp_ms: u64,
    /// Runtime health (if available).
    pub runtime: Option<RuntimeHealthInfo>,
    /// Worker state as seen by the worker.
    pub state: WorkerState,
}

impl WorkerHeartbeat {
    /// Create a heartbeat with runtime info.
    pub fn new(node_name: impl Into<String>, runtime: Option<RuntimeHealthInfo>) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let state = match &runtime {
            Some(r) if r.healthy => WorkerState::Healthy,
            Some(_) => WorkerState::Degraded,
            None => WorkerState::Connected,
        };

        Self {
            node_name: node_name.into(),
            timestamp_ms: now,
            runtime,
            state,
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_json(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::transport::NodeTransport;
    use super::super::channel::ChannelConfig;
    use super::super::sidecar::SidecarProxy;

    // ── WS1: GPU Node Identity ──────────────────────────────────────

    #[test]
    fn create_gpu_node_manifest() {
        let manifest = create_gpu_manifest(9003);

        assert_eq!(manifest.name, "swarm-gpu");
        assert_eq!(manifest.role, SwarmRole::GpuWorker);
        assert_eq!(manifest.role.to_node_role(), NodeRole::Worker);
        assert_eq!(manifest.binding.sidecar, "127.0.0.1:9003".parse().unwrap());
        assert_eq!(manifest.metadata.get("gpu").unwrap(), "nvidia-l4");
        assert_eq!(manifest.metadata.get("zone").unwrap(), "us-east1-b");
    }

    #[test]
    fn gpu_manifest_generates_signed_announcement() {
        let manifest = create_gpu_manifest(9003);
        let ann = manifest.announcement();
        assert!(ann.verify().is_ok());
    }

    #[test]
    fn mainframe_accepts_gpu_role() {
        let gpu_trust = SwarmRole::GpuWorker.default_trust_policy();
        // GPU trusts Orchestrator (mainframe)
        assert!(gpu_trust.allowed_roles.contains(&NodeRole::Orchestrator));
        // GPU trusts Gateway (portal)
        assert!(gpu_trust.allowed_roles.contains(&NodeRole::Gateway));
        // GPU does NOT trust other Workers directly
        assert!(!gpu_trust.allowed_roles.contains(&NodeRole::Worker));

        let coord_trust = SwarmRole::Coordinator.default_trust_policy();
        // Coordinator trusts Workers (GPU)
        assert!(coord_trust.allowed_roles.contains(&NodeRole::Worker));
    }

    #[test]
    fn capability_flags() {
        let caps = GpuCapabilityFlags::gpu_worker();
        assert!(caps.supports_gpu_inference);
        assert!(!caps.supports_gateway);
        assert!(!caps.supports_orchestration);
        assert!(!caps.supports_calculus_tools);
    }

    #[test]
    fn worker_labels() {
        let mut labels = WorkerLabels::gpu_l4();
        assert!(labels.contains("gpu"));
        assert!(labels.contains("inference"));
        assert!(labels.contains("ollama"));
        assert!(labels.contains("l4"));
        assert!(!labels.contains("triton"));

        labels.add("triton");
        assert!(labels.contains("triton"));

        // No duplicate
        labels.add("gpu");
        assert_eq!(labels.as_slice().iter().filter(|l| l.as_str() == "gpu").count(), 1);
    }

    // ── WS3: Worker Registration ────────────────────────────────────

    #[test]
    fn worker_registration_lifecycle() {
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );

        // Initial state
        assert_eq!(reg.state, WorkerState::Registered);
        assert!(!reg.is_dispatchable());
        assert!(reg.is_heartbeat_stale()); // no heartbeat yet

        // Simulate channel establishment
        let transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, NodeCapabilities {
                models: vec!["llama3.1:8b".into()],
                has_gpu: true,
                max_sessions: 50,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        let channel = EncryptedChannel::new(transport, ChannelConfig::default());
        reg.set_channel(channel);
        assert_eq!(reg.state, WorkerState::Connected);
        assert!(!reg.is_dispatchable()); // connected but no health yet

        // Receive healthy heartbeat
        let runtime = RuntimeHealthInfo::ollama_healthy(vec![
            LoadedModel {
                name: "llama3.1:8b".into(),
                ready: true,
                size_bytes: Some(4_500_000_000),
            },
        ]);
        reg.record_heartbeat(Some(runtime));
        assert_eq!(reg.state, WorkerState::Healthy);
        assert!(reg.is_dispatchable()); // now ready!
        assert!(!reg.is_heartbeat_stale());
    }

    #[test]
    fn degraded_state_blocks_dispatch() {
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );

        let transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, NodeCapabilities {
                models: vec![], has_gpu: true, max_sessions: 50,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        reg.set_channel(EncryptedChannel::new(transport, ChannelConfig::default()));

        // Unhealthy runtime
        let runtime = RuntimeHealthInfo::unhealthy(RuntimeKind::Ollama);
        reg.record_heartbeat(Some(runtime));
        assert_eq!(reg.state, WorkerState::Degraded);
        assert!(!reg.is_dispatchable());
    }

    #[test]
    fn unavailable_on_heartbeat_timeout() {
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );

        // No heartbeat ever → stale
        assert!(reg.is_heartbeat_stale());

        // Mark unavailable
        reg.mark_unavailable();
        assert_eq!(reg.state, WorkerState::Unavailable);
        assert!(!reg.is_dispatchable());
    }

    // ── WS5: Runtime Health ─────────────────────────────────────────

    #[test]
    fn healthy_ollama_runtime() {
        let info = RuntimeHealthInfo::ollama_healthy(vec![
            LoadedModel {
                name: "llama3.1:8b".into(),
                ready: true,
                size_bytes: Some(4_500_000_000),
            },
            LoadedModel {
                name: "qwen2.5-coder:7b".into(),
                ready: true,
                size_bytes: None,
            },
        ]);

        assert!(info.healthy);
        assert_eq!(info.kind, RuntimeKind::Ollama);
        assert!(info.has_model_ready("llama3.1:8b"));
        assert!(info.has_model_ready("qwen2.5-coder:7b"));
        assert!(!info.has_model_ready("nonexistent"));
    }

    #[test]
    fn unhealthy_runtime() {
        let info = RuntimeHealthInfo::unhealthy(RuntimeKind::Ollama);
        assert!(!info.healthy);
        assert!(info.loaded_models.is_empty());
        assert!(!info.has_model_ready("anything"));
    }

    #[test]
    fn runtime_health_json_roundtrip() {
        let info = RuntimeHealthInfo::ollama_healthy(vec![
            LoadedModel { name: "llama3.1:8b".into(), ready: true, size_bytes: Some(4_500_000_000) },
        ]);

        let json = info.to_json();
        let restored = RuntimeHealthInfo::from_json(&json).unwrap();

        assert!(restored.healthy);
        assert_eq!(restored.kind, RuntimeKind::Ollama);
        assert!(restored.has_model_ready("llama3.1:8b"));
    }

    #[test]
    fn heartbeat_json_roundtrip() {
        let runtime = RuntimeHealthInfo::ollama_healthy(vec![
            LoadedModel { name: "llama3.1:8b".into(), ready: true, size_bytes: None },
        ]);
        let hb = WorkerHeartbeat::new("swarm-gpu", Some(runtime));

        assert_eq!(hb.node_name, "swarm-gpu");
        assert_eq!(hb.state, WorkerState::Healthy);

        let json = hb.to_json();
        let restored = WorkerHeartbeat::from_json(&json).unwrap();
        assert_eq!(restored.node_name, "swarm-gpu");
        assert_eq!(restored.state, WorkerState::Healthy);
    }

    #[test]
    fn heartbeat_degraded_when_unhealthy() {
        let runtime = RuntimeHealthInfo::unhealthy(RuntimeKind::Ollama);
        let hb = WorkerHeartbeat::new("swarm-gpu", Some(runtime));
        assert_eq!(hb.state, WorkerState::Degraded);
    }

    #[test]
    fn heartbeat_connected_when_no_runtime() {
        let hb = WorkerHeartbeat::new("swarm-gpu", None);
        assert_eq!(hb.state, WorkerState::Connected);
    }

    #[test]
    fn multiple_runtime_kinds() {
        let ollama = RuntimeHealthInfo::ollama_healthy(vec![]);
        assert_eq!(ollama.kind, RuntimeKind::Ollama);

        let triton = RuntimeHealthInfo {
            kind: RuntimeKind::Triton,
            ..RuntimeHealthInfo::unhealthy(RuntimeKind::Triton)
        };
        assert_eq!(triton.kind, RuntimeKind::Triton);

        let ternary = RuntimeHealthInfo {
            kind: RuntimeKind::BunnyTernary,
            ..RuntimeHealthInfo::unhealthy(RuntimeKind::BunnyTernary)
        };
        assert_eq!(ternary.kind, RuntimeKind::BunnyTernary);
    }

    // ── WS1+WS3: Coordinator accepts GPU ────────────────────────────

    #[test]
    fn coordinator_sidecar_registers_gpu_peer() {
        let coord_manifest = create_mainframe_manifest(9001);
        let coord_transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Orchestrator, NodeCapabilities {
                models: vec![], has_gpu: false, max_sessions: 500,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        let mut coord_proxy = SidecarProxy::with_transport(coord_manifest, coord_transport);

        // Register GPU as peer
        coord_proxy.register_peer(
            "swarm-gpu",
            SwarmRole::GpuWorker,
            "10.142.0.6:9003".parse().unwrap(),
        );

        assert_eq!(coord_proxy.peer_count(), 1);
        assert!(!coord_proxy.is_peer_connected("swarm-gpu"));
    }

    #[test]
    fn worker_state_dispatchable() {
        assert!(!WorkerState::Registered.is_dispatchable());
        assert!(!WorkerState::Connected.is_dispatchable());
        assert!(WorkerState::Healthy.is_dispatchable());
        assert!(!WorkerState::Degraded.is_dispatchable());
        assert!(!WorkerState::Unavailable.is_dispatchable());
    }
}
