//! Codespace registry — generalized registration for all ecosystem nodes.
//!
//! Each codespace in the financecommander ecosystem registers with BUNNY,
//! establishing identity, capabilities, and encrypted channels. BUNNY/OpenClaw
//! is the **top-level authority** with unrestricted access to dispatch to
//! all codespaces, control VMs, and manage the swarm.
//!
//! Ecosystem codespaces:
//! - super-duper-spork (Coordinator) — swarm mainframe, control plane
//! - Triton (GpuWorker) — ternary NN inference engine
//! - Orchestra (Orchestra) — workflow DSL, .orc task graphs
//! - ProbFlow (ProbFlow) — adaptive layer, uncertainty scoring
//! - AI-PORTAL (Portal) — model registry, dashboards, gateway

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::channel::EncryptedChannel;
use super::node_identity::{NodeBinding, NodeManifest, SwarmRole};

// ── Codespace Identity ──────────────────────────────────────────────

/// Identifies which ecosystem codespace a node belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Codespace {
    /// financecommander/super-duper-spork — swarm mainframe, 17 agent castes.
    SuperDuperSpork,
    /// financecommander/Triton — ternary NN DSL + compiler.
    Triton,
    /// financecommander/Orchestra — workflow DSL (.orc task graphs).
    Orchestra,
    /// financecommander/ProbFlow — uncertainty scoring, adaptive routing.
    ProbFlow,
    /// financecommander/AI-PORTAL — model registry, eval dashboards.
    AiPortal,
    /// financecommander/BUNNY calculus — gradient/optimizer/similarity/stats/symbolic/tensor ops.
    Calculus,
}

impl Codespace {
    /// Map to the swarm role for trust policy enforcement.
    pub fn to_swarm_role(self) -> SwarmRole {
        match self {
            Self::SuperDuperSpork => SwarmRole::Coordinator,
            Self::Triton => SwarmRole::GpuWorker,
            Self::Orchestra => SwarmRole::Orchestra,
            Self::ProbFlow => SwarmRole::ProbFlow,
            Self::AiPortal => SwarmRole::Portal,
            Self::Calculus => SwarmRole::ComputeNode,
        }
    }

    /// Default node name for this codespace.
    pub fn default_node_name(self) -> &'static str {
        match self {
            Self::SuperDuperSpork => "swarm-mainframe",
            Self::Triton => "swarm-gpu",
            Self::Orchestra => "orchestra-engine",
            Self::ProbFlow => "probflow-adaptive",
            Self::AiPortal => "fc-ai-portal",
            Self::Calculus => "calculus-dispatcher",
        }
    }

    /// Well-known tools this codespace exposes.
    pub fn default_tools(self) -> Vec<&'static str> {
        match self {
            Self::SuperDuperSpork => vec![
                "send_email", "ghl_contact", "ghl_opportunity",
                "slack_message", "task_assign", "agent_dispatch",
            ],
            Self::Triton => vec![
                "inference", "ollama_generate", "ollama_chat",
                "model_inference", "ai_completion",
            ],
            Self::Orchestra => vec![
                "workflow_execute", "task_graph_run", "orc_parse",
                "step_dispatch", "workflow_status",
            ],
            Self::ProbFlow => vec![
                "uncertainty_score", "confidence_route",
                "threshold_adjust", "model_evaluate",
            ],
            Self::AiPortal => vec![
                "model_register", "model_list", "eval_submit",
                "telemetry_push", "dashboard_query",
            ],
            Self::Calculus => vec![
                "classify_request", "gradient_apply", "gradient_norm",
                "optimize_quantization", "cosine_similarity", "batch_stats",
                "iqr_outliers", "symbolic_parse", "tensor_ops",
                "route_to_swarm",
            ],
        }
    }

    /// All codespace variants.
    pub fn all() -> &'static [Codespace] {
        &[
            Self::SuperDuperSpork,
            Self::Triton,
            Self::Orchestra,
            Self::ProbFlow,
            Self::AiPortal,
            Self::Calculus,
        ]
    }
}

// ── Capabilities ────────────────────────────────────────────────────

/// Capability flags for a codespace node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodespaceCapabilities {
    /// Can run model inference (ternary/GPU/cloud).
    pub inference: bool,
    /// Can dispatch tool calls to other nodes.
    pub tool_dispatch: bool,
    /// Can orchestrate workflows / task graphs.
    pub orchestration: bool,
    /// Can register and serve model metadata.
    pub model_registry: bool,
    /// Can score output uncertainty and route adaptively.
    pub uncertainty_scoring: bool,
}

impl CodespaceCapabilities {
    /// Capabilities for super-duper-spork (swarm mainframe).
    pub fn super_duper_spork() -> Self {
        Self {
            inference: false,
            tool_dispatch: true,
            orchestration: true,
            model_registry: false,
            uncertainty_scoring: false,
        }
    }

    /// Capabilities for Triton (GPU inference).
    pub fn triton() -> Self {
        Self {
            inference: true,
            tool_dispatch: false,
            orchestration: false,
            model_registry: false,
            uncertainty_scoring: false,
        }
    }

    /// Capabilities for Orchestra (workflow engine).
    pub fn orchestra() -> Self {
        Self {
            inference: false,
            tool_dispatch: true,
            orchestration: true,
            model_registry: false,
            uncertainty_scoring: false,
        }
    }

    /// Capabilities for ProbFlow (adaptive layer).
    pub fn probflow() -> Self {
        Self {
            inference: false,
            tool_dispatch: false,
            orchestration: false,
            model_registry: false,
            uncertainty_scoring: true,
        }
    }

    /// Capabilities for AI-PORTAL (gateway + model registry).
    pub fn ai_portal() -> Self {
        Self {
            inference: false,
            tool_dispatch: true,
            orchestration: false,
            model_registry: true,
            uncertainty_scoring: false,
        }
    }

    /// Capabilities for Calculus (task dispatcher + compute).
    pub fn calculus() -> Self {
        Self {
            inference: false,
            tool_dispatch: true,
            orchestration: false,
            model_registry: false,
            uncertainty_scoring: false,
        }
    }

    /// Default capabilities for a codespace.
    pub fn for_codespace(cs: Codespace) -> Self {
        match cs {
            Codespace::SuperDuperSpork => Self::super_duper_spork(),
            Codespace::Triton => Self::triton(),
            Codespace::Orchestra => Self::orchestra(),
            Codespace::ProbFlow => Self::probflow(),
            Codespace::AiPortal => Self::ai_portal(),
            Codespace::Calculus => Self::calculus(),
        }
    }
}

// ── State Machine ───────────────────────────────────────────────────

/// Registration state for a codespace node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodespaceState {
    /// Registered but no encrypted channel yet.
    Registered,
    /// Encrypted channel established, waiting for health.
    Connected,
    /// Channel established + heartbeat received → ready.
    Healthy,
    /// Channel up but node reports issues.
    Degraded,
    /// Channel lost or heartbeat timeout.
    Unavailable,
}

impl CodespaceState {
    /// Whether the codespace can receive dispatch.
    pub fn is_reachable(self) -> bool {
        matches!(self, Self::Connected | Self::Healthy)
    }
}

// ── Registration ────────────────────────────────────────────────────

/// Registration record for a codespace node.
pub struct CodespaceRegistration {
    /// Node name (e.g., "swarm-mainframe").
    pub name: String,
    /// Which codespace this node belongs to.
    pub codespace: Codespace,
    /// Swarm role derived from codespace.
    pub role: SwarmRole,
    /// Sidecar address for encrypted proxy.
    pub sidecar_addr: SocketAddr,
    /// Current state.
    pub state: CodespaceState,
    /// Capability flags.
    pub capabilities: CodespaceCapabilities,
    /// Key-value metadata (zone, version, etc.).
    pub metadata: HashMap<String, String>,
    /// Last heartbeat received.
    pub last_heartbeat: Option<Instant>,
    /// Heartbeat interval expected.
    pub heartbeat_interval: Duration,
    /// Encrypted channel to this node (if established).
    pub channel: Option<EncryptedChannel>,
}

impl CodespaceRegistration {
    /// Create a new registration with defaults for the given codespace.
    pub fn new(
        name: impl Into<String>,
        codespace: Codespace,
        sidecar_addr: SocketAddr,
    ) -> Self {
        Self {
            name: name.into(),
            codespace,
            role: codespace.to_swarm_role(),
            sidecar_addr,
            state: CodespaceState::Registered,
            capabilities: CodespaceCapabilities::for_codespace(codespace),
            metadata: HashMap::new(),
            last_heartbeat: None,
            heartbeat_interval: Duration::from_secs(30),
            channel: None,
        }
    }

    /// Record a heartbeat with optional metadata.
    pub fn record_heartbeat(&mut self, meta: Option<HashMap<String, String>>) {
        self.last_heartbeat = Some(Instant::now());

        if let Some(m) = meta {
            for (k, v) in m {
                self.metadata.insert(k, v);
            }
        }

        // Transition state based on channel presence.
        if self.channel.is_some() {
            self.state = CodespaceState::Healthy;
        } else {
            self.state = CodespaceState::Connected;
        }
    }

    /// Check if heartbeat has timed out.
    pub fn is_heartbeat_stale(&self) -> bool {
        match self.last_heartbeat {
            Some(last) => last.elapsed() > self.heartbeat_interval * 3,
            None => true,
        }
    }

    /// Mark as unavailable (heartbeat timeout or disconnect).
    pub fn mark_unavailable(&mut self) {
        self.state = CodespaceState::Unavailable;
    }

    /// Establish encrypted channel.
    pub fn set_channel(&mut self, channel: EncryptedChannel) {
        self.channel = Some(channel);
        if self.state == CodespaceState::Registered {
            self.state = CodespaceState::Connected;
        }
    }

    /// Can this codespace receive dispatch?
    pub fn is_reachable(&self) -> bool {
        self.state.is_reachable()
    }

    /// Take a cloneable snapshot of this registration (no channel ref).
    pub fn snapshot(&self) -> CodespaceSnapshot {
        CodespaceSnapshot {
            name: self.name.clone(),
            codespace: self.codespace,
            role: self.role,
            state: self.state,
            capabilities: self.capabilities.clone(),
            has_channel: self.channel.is_some(),
            metadata: self.metadata.clone(),
            available_tools: self.codespace.default_tools()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

// ── Snapshot (cloneable view) ───────────────────────────────────────

/// Cloneable, serializable snapshot of a codespace registration.
///
/// Used for queries, API responses, and dashboard display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodespaceSnapshot {
    pub name: String,
    pub codespace: Codespace,
    pub role: SwarmRole,
    pub state: CodespaceState,
    pub capabilities: CodespaceCapabilities,
    pub has_channel: bool,
    pub metadata: HashMap<String, String>,
    pub available_tools: Vec<String>,
}

// ── Heartbeat ───────────────────────────────────────────────────────

/// Heartbeat from a codespace node → BUNNY coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodespaceHeartbeat {
    /// Node name.
    pub node_name: String,
    /// Which codespace.
    pub codespace: Codespace,
    /// Unix timestamp.
    pub timestamp_ms: u64,
    /// Node state as seen by the node.
    pub state: CodespaceState,
    /// Optional metadata (version, load, etc.).
    pub metadata: HashMap<String, String>,
}

impl CodespaceHeartbeat {
    /// Create a new heartbeat.
    pub fn new(
        node_name: impl Into<String>,
        codespace: Codespace,
        metadata: HashMap<String, String>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            node_name: node_name.into(),
            codespace,
            timestamp_ms: now,
            state: CodespaceState::Healthy,
            metadata,
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_json(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

// ── Registry ────────────────────────────────────────────────────────

/// Thread-safe registry of all codespace nodes.
///
/// BUNNY/OpenClaw has **unrestricted access** — every registered codespace
/// can be dispatched to without permission checks.
pub struct CodespaceRegistry {
    inner: RwLock<HashMap<String, CodespaceRegistration>>,
}

impl CodespaceRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Register a codespace node.
    pub fn register(&self, registration: CodespaceRegistration) {
        let name = registration.name.clone();
        self.inner.write().unwrap().insert(name, registration);
    }

    /// Unregister a codespace node.
    pub fn unregister(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    /// Get a snapshot of a registered codespace.
    pub fn get_snapshot(&self, name: &str) -> Option<CodespaceSnapshot> {
        self.inner.read().unwrap().get(name).map(|r| r.snapshot())
    }

    /// Update heartbeat for a named codespace.
    pub fn update_heartbeat(
        &self,
        name: &str,
        meta: Option<HashMap<String, String>>,
    ) -> bool {
        if let Some(reg) = self.inner.write().unwrap().get_mut(name) {
            reg.record_heartbeat(meta);
            true
        } else {
            false
        }
    }

    /// Find all registrations matching a codespace type.
    pub fn find_by_codespace(&self, cs: Codespace) -> Vec<CodespaceSnapshot> {
        self.inner
            .read()
            .unwrap()
            .values()
            .filter(|r| r.codespace == cs)
            .map(|r| r.snapshot())
            .collect()
    }

    /// Find all reachable codespaces.
    pub fn find_reachable(&self) -> Vec<CodespaceSnapshot> {
        self.inner
            .read()
            .unwrap()
            .values()
            .filter(|r| r.is_reachable())
            .map(|r| r.snapshot())
            .collect()
    }

    /// Find codespaces with a specific capability.
    pub fn find_by_capability<F>(&self, predicate: F) -> Vec<CodespaceSnapshot>
    where
        F: Fn(&CodespaceCapabilities) -> bool,
    {
        self.inner
            .read()
            .unwrap()
            .values()
            .filter(|r| predicate(&r.capabilities))
            .map(|r| r.snapshot())
            .collect()
    }

    /// Total registered codespaces.
    pub fn count(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }

    /// Snapshot of all registrations.
    pub fn all_snapshots(&self) -> Vec<CodespaceSnapshot> {
        self.inner
            .read()
            .unwrap()
            .values()
            .map(|r| r.snapshot())
            .collect()
    }
}

impl Default for CodespaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Manifest Generators ─────────────────────────────────────────────

/// Create node manifest for super-duper-spork (swarm mainframe).
pub fn create_spork_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::Coordinator,
        "swarm-mainframe",
        NodeBinding {
            internal: "10.142.0.4:3000".parse().unwrap(),
            external: Some("34.148.140.31:3000".parse().unwrap()),
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
    .with_metadata("codespace", "super-duper-spork")
    .with_metadata("role", "control-plane")
}

/// Create node manifest for Orchestra workflow engine.
pub fn create_orchestra_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::Orchestra,
        "orchestra-engine",
        NodeBinding {
            internal: "10.142.0.5:3000".parse().unwrap(),
            external: None,
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
    .with_metadata("codespace", "orchestra")
    .with_metadata("role", "workflow-engine")
}

/// Create node manifest for ProbFlow adaptive layer.
pub fn create_probflow_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::ProbFlow,
        "probflow-adaptive",
        NodeBinding {
            internal: "10.142.0.7:3000".parse().unwrap(),
            external: None,
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
    .with_metadata("codespace", "probflow")
    .with_metadata("role", "adaptive-layer")
}

/// Create node manifest for Calculus task dispatcher (Jack).
pub fn create_calculus_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::ComputeNode,
        "calculus-dispatcher",
        NodeBinding {
            internal: "10.142.0.9:3000".parse().unwrap(),
            external: None,
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
    .with_metadata("codespace", "calculus")
    .with_metadata("role", "task-dispatcher")
    .with_metadata("agent", "jack")
}

/// Create node manifest for AI-PORTAL gateway.
pub fn create_portal_manifest(sidecar_port: u16) -> NodeManifest {
    NodeManifest::generate(
        SwarmRole::Portal,
        "fc-ai-portal",
        NodeBinding {
            internal: "10.142.0.8:3000".parse().unwrap(),
            external: Some("34.148.8.51:443".parse().unwrap()),
            sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
        },
    )
    .with_metadata("codespace", "ai-portal")
    .with_metadata("role", "gateway")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::transport::NodeTransport;
    use super::super::channel::ChannelConfig;

    fn make_channel() -> EncryptedChannel {
        let transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, NodeCapabilities {
                models: vec![], has_gpu: false, max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        EncryptedChannel::new(transport, ChannelConfig::default())
    }

    // ── Codespace Enum ──────────────────────────────────────────────

    #[test]
    fn codespace_to_swarm_role_mapping() {
        assert_eq!(Codespace::SuperDuperSpork.to_swarm_role(), SwarmRole::Coordinator);
        assert_eq!(Codespace::Triton.to_swarm_role(), SwarmRole::GpuWorker);
        assert_eq!(Codespace::Orchestra.to_swarm_role(), SwarmRole::Orchestra);
        assert_eq!(Codespace::ProbFlow.to_swarm_role(), SwarmRole::ProbFlow);
        assert_eq!(Codespace::AiPortal.to_swarm_role(), SwarmRole::Portal);
        assert_eq!(Codespace::Calculus.to_swarm_role(), SwarmRole::ComputeNode);
    }

    #[test]
    fn codespace_default_tools() {
        let spork_tools = Codespace::SuperDuperSpork.default_tools();
        assert!(spork_tools.contains(&"send_email"));
        assert!(spork_tools.contains(&"agent_dispatch"));

        let triton_tools = Codespace::Triton.default_tools();
        assert!(triton_tools.contains(&"inference"));
        assert!(triton_tools.contains(&"ollama_chat"));

        let orchestra_tools = Codespace::Orchestra.default_tools();
        assert!(orchestra_tools.contains(&"workflow_execute"));

        let probflow_tools = Codespace::ProbFlow.default_tools();
        assert!(probflow_tools.contains(&"uncertainty_score"));

        let portal_tools = Codespace::AiPortal.default_tools();
        assert!(portal_tools.contains(&"model_register"));
    }

    #[test]
    fn codespace_all_variants() {
        assert_eq!(Codespace::all().len(), 6);
    }

    // ── Capabilities ────────────────────────────────────────────────

    #[test]
    fn codespace_capabilities_super_duper_spork() {
        let caps = CodespaceCapabilities::super_duper_spork();
        assert!(caps.tool_dispatch);
        assert!(caps.orchestration);
        assert!(!caps.inference);
        assert!(!caps.model_registry);
    }

    #[test]
    fn codespace_capabilities_triton() {
        let caps = CodespaceCapabilities::triton();
        assert!(caps.inference);
        assert!(!caps.tool_dispatch);
        assert!(!caps.orchestration);
    }

    #[test]
    fn codespace_capabilities_for_codespace() {
        let caps = CodespaceCapabilities::for_codespace(Codespace::AiPortal);
        assert!(caps.model_registry);
        assert!(caps.tool_dispatch);
        assert!(!caps.inference);
    }

    // ── State Machine ───────────────────────────────────────────────

    #[test]
    fn codespace_state_reachable() {
        assert!(!CodespaceState::Registered.is_reachable());
        assert!(CodespaceState::Connected.is_reachable());
        assert!(CodespaceState::Healthy.is_reachable());
        assert!(!CodespaceState::Degraded.is_reachable());
        assert!(!CodespaceState::Unavailable.is_reachable());
    }

    // ── Registration Lifecycle ──────────────────────────────────────

    #[test]
    fn codespace_registration_lifecycle() {
        let mut reg = CodespaceRegistration::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            "10.142.0.4:9001".parse().unwrap(),
        );

        // Initial state
        assert_eq!(reg.state, CodespaceState::Registered);
        assert!(!reg.is_reachable());
        assert!(reg.is_heartbeat_stale());
        assert_eq!(reg.role, SwarmRole::Coordinator);

        // Channel established
        reg.set_channel(make_channel());
        assert_eq!(reg.state, CodespaceState::Connected);
        assert!(reg.is_reachable());

        // Heartbeat received
        reg.record_heartbeat(Some(HashMap::from([
            ("version".into(), "2.0.0".into()),
        ])));
        assert_eq!(reg.state, CodespaceState::Healthy);
        assert!(reg.is_reachable());
        assert!(!reg.is_heartbeat_stale());
        assert_eq!(reg.metadata.get("version").unwrap(), "2.0.0");
    }

    #[test]
    fn codespace_unavailable() {
        let mut reg = CodespaceRegistration::new(
            "orchestra-engine",
            Codespace::Orchestra,
            "10.142.0.5:9004".parse().unwrap(),
        );

        reg.mark_unavailable();
        assert_eq!(reg.state, CodespaceState::Unavailable);
        assert!(!reg.is_reachable());
    }

    #[test]
    fn codespace_snapshot() {
        let mut reg = CodespaceRegistration::new(
            "probflow-adaptive",
            Codespace::ProbFlow,
            "10.142.0.7:9005".parse().unwrap(),
        );
        reg.set_channel(make_channel());

        let snap = reg.snapshot();
        assert_eq!(snap.name, "probflow-adaptive");
        assert_eq!(snap.codespace, Codespace::ProbFlow);
        assert_eq!(snap.role, SwarmRole::ProbFlow);
        assert!(snap.has_channel);
        assert!(snap.capabilities.uncertainty_scoring);
        assert!(snap.available_tools.contains(&"uncertainty_score".to_string()));
    }

    // ── Heartbeat ───────────────────────────────────────────────────

    #[test]
    fn codespace_heartbeat_json_roundtrip() {
        let hb = CodespaceHeartbeat::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            HashMap::from([("agents".into(), "17".into())]),
        );

        assert_eq!(hb.node_name, "swarm-mainframe");
        assert_eq!(hb.codespace, Codespace::SuperDuperSpork);
        assert_eq!(hb.state, CodespaceState::Healthy);

        let json = hb.to_json();
        let restored = CodespaceHeartbeat::from_json(&json).unwrap();
        assert_eq!(restored.node_name, "swarm-mainframe");
        assert_eq!(restored.codespace, Codespace::SuperDuperSpork);
        assert_eq!(restored.metadata.get("agents").unwrap(), "17");
    }

    // ── Registry ────────────────────────────────────────────────────

    #[test]
    fn codespace_registry_register_and_find() {
        let registry = CodespaceRegistry::new();

        registry.register(CodespaceRegistration::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            "10.142.0.4:9001".parse().unwrap(),
        ));
        registry.register(CodespaceRegistration::new(
            "orchestra-engine",
            Codespace::Orchestra,
            "10.142.0.5:9004".parse().unwrap(),
        ));

        assert_eq!(registry.count(), 2);
        assert!(!registry.is_empty());

        let snap = registry.get_snapshot("swarm-mainframe").unwrap();
        assert_eq!(snap.codespace, Codespace::SuperDuperSpork);

        assert!(registry.get_snapshot("nonexistent").is_none());
    }

    #[test]
    fn codespace_registry_find_by_codespace() {
        let registry = CodespaceRegistry::new();

        registry.register(CodespaceRegistration::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            "10.142.0.4:9001".parse().unwrap(),
        ));
        registry.register(CodespaceRegistration::new(
            "swarm-gpu",
            Codespace::Triton,
            "10.142.0.6:9003".parse().unwrap(),
        ));

        let spork = registry.find_by_codespace(Codespace::SuperDuperSpork);
        assert_eq!(spork.len(), 1);
        assert_eq!(spork[0].name, "swarm-mainframe");

        let orchestra = registry.find_by_codespace(Codespace::Orchestra);
        assert!(orchestra.is_empty());
    }

    #[test]
    fn codespace_registry_find_by_capability() {
        let registry = CodespaceRegistry::new();

        registry.register(CodespaceRegistration::new(
            "swarm-gpu",
            Codespace::Triton,
            "10.142.0.6:9003".parse().unwrap(),
        ));
        registry.register(CodespaceRegistration::new(
            "probflow-adaptive",
            Codespace::ProbFlow,
            "10.142.0.7:9005".parse().unwrap(),
        ));

        let inference_nodes = registry.find_by_capability(|c| c.inference);
        assert_eq!(inference_nodes.len(), 1);
        assert_eq!(inference_nodes[0].name, "swarm-gpu");

        let scoring_nodes = registry.find_by_capability(|c| c.uncertainty_scoring);
        assert_eq!(scoring_nodes.len(), 1);
        assert_eq!(scoring_nodes[0].name, "probflow-adaptive");
    }

    #[test]
    fn codespace_registry_unregister() {
        let registry = CodespaceRegistry::new();

        registry.register(CodespaceRegistration::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            "10.142.0.4:9001".parse().unwrap(),
        ));

        assert!(registry.unregister("swarm-mainframe"));
        assert!(!registry.unregister("swarm-mainframe")); // already gone
        assert!(registry.is_empty());
    }

    #[test]
    fn codespace_registry_update_heartbeat() {
        let registry = CodespaceRegistry::new();

        let mut reg = CodespaceRegistration::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            "10.142.0.4:9001".parse().unwrap(),
        );
        reg.set_channel(make_channel());
        registry.register(reg);

        let updated = registry.update_heartbeat(
            "swarm-mainframe",
            Some(HashMap::from([("load".into(), "0.3".into())])),
        );
        assert!(updated);

        let snap = registry.get_snapshot("swarm-mainframe").unwrap();
        assert_eq!(snap.state, CodespaceState::Healthy);
        assert_eq!(snap.metadata.get("load").unwrap(), "0.3");

        // Unknown node
        assert!(!registry.update_heartbeat("nonexistent", None));
    }

    #[test]
    fn codespace_registry_all_snapshots() {
        let registry = CodespaceRegistry::new();

        for cs in Codespace::all() {
            registry.register(CodespaceRegistration::new(
                cs.default_node_name(),
                *cs,
                "127.0.0.1:9000".parse().unwrap(),
            ));
        }

        assert_eq!(registry.count(), 6);
        let all = registry.all_snapshots();
        assert_eq!(all.len(), 6);
    }

    // ── Manifest Generators ─────────────────────────────────────────

    #[test]
    fn codespace_manifest_generators() {
        let spork = create_spork_manifest(9001);
        assert_eq!(spork.name, "swarm-mainframe");
        assert_eq!(spork.role, SwarmRole::Coordinator);
        assert_eq!(spork.metadata.get("codespace").unwrap(), "super-duper-spork");
        assert!(spork.announcement().verify().is_ok());

        let orchestra = create_orchestra_manifest(9004);
        assert_eq!(orchestra.name, "orchestra-engine");
        assert_eq!(orchestra.role, SwarmRole::Orchestra);
        assert_eq!(orchestra.metadata.get("codespace").unwrap(), "orchestra");

        let probflow = create_probflow_manifest(9005);
        assert_eq!(probflow.name, "probflow-adaptive");
        assert_eq!(probflow.role, SwarmRole::ProbFlow);

        let portal = create_portal_manifest(9006);
        assert_eq!(portal.name, "fc-ai-portal");
        assert_eq!(portal.role, SwarmRole::Portal);
        assert_eq!(portal.metadata.get("codespace").unwrap(), "ai-portal");
    }

    // ── Calculus Codespace ──────────────────────────────────────────

    #[test]
    fn codespace_calculus_basics() {
        assert_eq!(Codespace::Calculus.to_swarm_role(), SwarmRole::ComputeNode);
        assert_eq!(Codespace::Calculus.default_node_name(), "calculus-dispatcher");

        let tools = Codespace::Calculus.default_tools();
        assert!(tools.contains(&"classify_request"));
        assert!(tools.contains(&"gradient_apply"));
        assert!(tools.contains(&"cosine_similarity"));
        assert!(tools.contains(&"route_to_swarm"));
    }

    #[test]
    fn codespace_capabilities_calculus() {
        let caps = CodespaceCapabilities::calculus();
        assert!(caps.tool_dispatch);
        assert!(!caps.inference);
        assert!(!caps.orchestration);
        assert!(!caps.model_registry);
        assert!(!caps.uncertainty_scoring);

        let caps2 = CodespaceCapabilities::for_codespace(Codespace::Calculus);
        assert!(caps2.tool_dispatch);
    }

    #[test]
    fn codespace_calculus_manifest() {
        let manifest = create_calculus_manifest(9007);
        assert_eq!(manifest.name, "calculus-dispatcher");
        assert_eq!(manifest.role, SwarmRole::ComputeNode);
        assert_eq!(manifest.metadata.get("codespace").unwrap(), "calculus");
        assert_eq!(manifest.metadata.get("agent").unwrap(), "jack");
        assert!(manifest.announcement().verify().is_ok());
    }
}
