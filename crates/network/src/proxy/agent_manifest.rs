//! Agent manifest — BUNNY's advertisement of available agents and models.
//!
//! When BUNNY registers with a codespace (AI-PORTAL, super-duper-spork,
//! Orchestra, ProbFlow), it publishes an [`AgentManifest`] describing all
//! loaded agents, available models, and exposed tools.
//!
//! Other codespaces consume this manifest to route inference requests,
//! task graph steps, and uncertainty queries to the correct BUNNY node.
//!
//! BUNNY/OpenClaw manifests are created with **unrestricted access** —
//! full authority over swarm dispatch, VM control, and all codespaces.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use bunny_agents::{AgentRole, ComputeBackend, ModelScale};

use super::codespace_registry::Codespace;
use super::node_identity::SwarmRole;

/// Well-known path for agent manifest exchange.
pub const AGENT_MANIFEST_PATH: &str = "/bunny/agent/manifest";

// ── Descriptors ─────────────────────────────────────────────────────

/// Describes a single agent available on a BUNNY node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescriptor {
    /// Agent name (e.g., "threat_hunter").
    pub name: String,
    /// Agent role.
    pub role: AgentRole,
    /// Model name used by this agent (e.g., "threat_classifier").
    pub model_name: String,
    /// Input dimension expected.
    pub input_dim: usize,
    /// Output dimension produced.
    pub output_dim: usize,
    /// Model scale tier.
    pub scale: ModelScale,
    /// Compute backend.
    pub backend: ComputeBackend,
}

/// Describes a model available on a BUNNY node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Model name.
    pub name: String,
    /// Total parameter count.
    pub total_params: usize,
    /// Scale classification (cell/micro/pico).
    pub scale: ModelScale,
    /// Recommended compute backend.
    pub backend: ComputeBackend,
    /// Input dimension.
    pub input_dim: usize,
    /// Output dimension.
    pub output_dim: usize,
    /// Number of layers.
    pub num_layers: usize,
    /// VRAM usage estimate in bytes (2-bit packed).
    pub vram_bytes: usize,
}

// ── Manifest ────────────────────────────────────────────────────────

/// Full agent/model manifest for a BUNNY node.
///
/// Published to all connected codespaces so they can discover and route
/// inference requests to the correct agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Node name (e.g., "bunny-edge-01").
    pub node_name: String,
    /// Which codespace this node belongs to.
    pub codespace: Codespace,
    /// Swarm role.
    pub role: SwarmRole,
    /// Available agents on this node.
    pub agents: Vec<AgentDescriptor>,
    /// Available models on this node.
    pub models: Vec<ModelDescriptor>,
    /// Tools this node can dispatch.
    pub available_tools: Vec<String>,
    /// Whether this node has unrestricted swarm access.
    ///
    /// When `true`, this node (BUNNY/OpenClaw) has full authority to
    /// dispatch to all codespaces, control VMs, and manage the swarm.
    pub unrestricted_access: bool,
    /// Key-value metadata.
    pub metadata: HashMap<String, String>,
}

impl AgentManifest {
    /// Serialize to JSON bytes.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from JSON bytes.
    pub fn from_json(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }

    /// Check if a named agent is in this manifest.
    pub fn has_agent(&self, name: &str) -> bool {
        self.agents.iter().any(|a| a.name == name)
    }

    /// Check if a named model is in this manifest.
    pub fn has_model(&self, name: &str) -> bool {
        self.models.iter().any(|m| m.name == name)
    }

    /// Number of agents.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Number of models.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }
}

// ── Builder ─────────────────────────────────────────────────────────

/// Fluent builder for [`AgentManifest`].
pub struct AgentManifestBuilder {
    node_name: String,
    codespace: Codespace,
    role: SwarmRole,
    agents: Vec<AgentDescriptor>,
    models: Vec<ModelDescriptor>,
    tools: Vec<String>,
    unrestricted: bool,
    metadata: HashMap<String, String>,
}

impl AgentManifestBuilder {
    /// Start building a manifest for the given node.
    pub fn new(
        node_name: impl Into<String>,
        codespace: Codespace,
    ) -> Self {
        let role = codespace.to_swarm_role();
        Self {
            node_name: node_name.into(),
            codespace,
            role,
            agents: Vec::new(),
            models: Vec::new(),
            tools: Vec::new(),
            unrestricted: false,
            metadata: HashMap::new(),
        }
    }

    /// Add a key-value metadata entry.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Mark this manifest as having unrestricted swarm access.
    ///
    /// BUNNY/OpenClaw manifests should always set this.
    pub fn with_unrestricted_access(mut self) -> Self {
        self.unrestricted = true;
        self
    }

    /// Add an agent descriptor.
    pub fn add_agent(&mut self, descriptor: AgentDescriptor) -> &mut Self {
        self.agents.push(descriptor);
        self
    }

    /// Add a model descriptor.
    pub fn add_model(&mut self, descriptor: ModelDescriptor) -> &mut Self {
        self.models.push(descriptor);
        self
    }

    /// Add a tool name.
    pub fn add_tool(&mut self, name: impl Into<String>) -> &mut Self {
        self.tools.push(name.into());
        self
    }

    /// Auto-populate agents from an [`AgentRunner`].
    ///
    /// Reads all registered agents and creates descriptors. Model dimensions
    /// default to 0 (populated by caller if known).
    pub fn build_from_runner(&mut self, runner: &bunny_agents::AgentRunner) -> &mut Self {
        for name in runner.registered_agents() {
            if let Some(agent) = runner.get_agent(name) {
                self.agents.push(AgentDescriptor {
                    name: agent.name().to_string(),
                    role: agent.role(),
                    model_name: agent.name().to_string(),
                    input_dim: 0,
                    output_dim: 0,
                    scale: ModelScale::Cell,
                    backend: ComputeBackend::DenseCpu,
                });
            }
        }
        self
    }

    /// Build the manifest.
    pub fn build(self) -> AgentManifest {
        AgentManifest {
            node_name: self.node_name,
            codespace: self.codespace,
            role: self.role,
            agents: self.agents,
            models: self.models,
            available_tools: self.tools,
            unrestricted_access: self.unrestricted,
            metadata: self.metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent() -> AgentDescriptor {
        AgentDescriptor {
            name: "threat_hunter".into(),
            role: AgentRole::ThreatHunter,
            model_name: "threat_classifier".into(),
            input_dim: 192,
            output_dim: 5,
            scale: ModelScale::Micro,
            backend: ComputeBackend::CudaPacked,
        }
    }

    fn sample_model() -> ModelDescriptor {
        ModelDescriptor {
            name: "threat_classifier".into(),
            total_params: 8_400_000,
            scale: ModelScale::Micro,
            backend: ComputeBackend::CudaPacked,
            input_dim: 192,
            output_dim: 5,
            num_layers: 4,
            vram_bytes: 2_104_096,
        }
    }

    // ── Descriptor Serde ────────────────────────────────────────────

    #[test]
    fn agent_descriptor_roundtrip() {
        let desc = sample_agent();
        let json = serde_json::to_vec(&desc).unwrap();
        let restored: AgentDescriptor = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored.name, "threat_hunter");
        assert_eq!(restored.role, AgentRole::ThreatHunter);
        assert_eq!(restored.scale, ModelScale::Micro);
    }

    #[test]
    fn model_descriptor_roundtrip() {
        let desc = sample_model();
        let json = serde_json::to_vec(&desc).unwrap();
        let restored: ModelDescriptor = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored.name, "threat_classifier");
        assert_eq!(restored.total_params, 8_400_000);
        assert_eq!(restored.num_layers, 4);
    }

    // ── Manifest Serde ──────────────────────────────────────────────

    #[test]
    fn agent_manifest_roundtrip() {
        let manifest = AgentManifestBuilder::new("bunny-edge-01", Codespace::Triton)
            .with_unrestricted_access()
            .with_metadata("zone", "us-east1-b")
            .build();

        let json = manifest.to_json();
        let restored = AgentManifest::from_json(&json).unwrap();
        assert_eq!(restored.node_name, "bunny-edge-01");
        assert_eq!(restored.codespace, Codespace::Triton);
        assert!(restored.unrestricted_access);
        assert_eq!(restored.metadata.get("zone").unwrap(), "us-east1-b");
    }

    // ── Builder ─────────────────────────────────────────────────────

    #[test]
    fn manifest_builder_basic() {
        let mut builder = AgentManifestBuilder::new(
            "bunny-edge-01",
            Codespace::SuperDuperSpork,
        ).with_unrestricted_access();

        builder.add_agent(sample_agent());
        builder.add_model(sample_model());
        builder.add_tool("inference");
        builder.add_tool("ollama_chat");

        let manifest = builder.build();

        assert_eq!(manifest.agent_count(), 1);
        assert_eq!(manifest.model_count(), 1);
        assert!(manifest.has_agent("threat_hunter"));
        assert!(manifest.has_model("threat_classifier"));
        assert!(manifest.unrestricted_access);
        assert_eq!(manifest.available_tools.len(), 2);
    }

    #[test]
    fn manifest_has_agent_lookup() {
        let mut builder = AgentManifestBuilder::new("node", Codespace::Triton);
        builder.add_agent(sample_agent());
        let manifest = builder.build();

        assert!(manifest.has_agent("threat_hunter"));
        assert!(!manifest.has_agent("nonexistent"));
    }

    #[test]
    fn manifest_has_model_lookup() {
        let mut builder = AgentManifestBuilder::new("node", Codespace::Triton);
        builder.add_model(sample_model());
        let manifest = builder.build();

        assert!(manifest.has_model("threat_classifier"));
        assert!(!manifest.has_model("nonexistent"));
    }

    #[test]
    fn empty_manifest() {
        let manifest = AgentManifestBuilder::new("empty-node", Codespace::ProbFlow).build();

        assert_eq!(manifest.agent_count(), 0);
        assert_eq!(manifest.model_count(), 0);
        assert!(!manifest.unrestricted_access);
        assert!(manifest.available_tools.is_empty());
    }

    #[test]
    fn manifest_bunny_unrestricted() {
        let manifest = AgentManifestBuilder::new("bunny-openclaw", Codespace::SuperDuperSpork)
            .with_unrestricted_access()
            .with_metadata("authority", "top-level")
            .build();

        assert!(manifest.unrestricted_access);
        assert_eq!(manifest.metadata.get("authority").unwrap(), "top-level");
        assert_eq!(manifest.role, SwarmRole::Coordinator);
    }

    // ── Builder from AgentRunner ────────────────────────────────────

    #[test]
    fn manifest_builder_from_runner() {
        use bunny_agents::{Agent, AgentInput, AgentOutput, AgentRunner};
        use bunny_crypto::types::AgentId;
        use std::sync::Arc;

        // Mock agent implementing the Agent trait (avoids bunny-triton dep).
        struct MockAgent;
        impl Agent for MockAgent {
            fn name(&self) -> &str { "mock_threat_hunter" }
            fn role(&self) -> AgentRole { AgentRole::ThreatHunter }
            fn system_prompt(&self) -> &str { "Classify threats" }
            fn process(&self, _input: &AgentInput) -> bunny_agents::Result<AgentOutput> {
                Ok(AgentOutput {
                    data: vec![0.0, 1.0],
                    prediction: 1,
                    inference_time_us: 100,
                    backend: ComputeBackend::DenseCpu,
                    result: None,
                })
            }
        }

        let mut runner = AgentRunner::new(AgentId::new());
        runner.register(Arc::new(MockAgent));

        // Build manifest from runner.
        let mut builder = AgentManifestBuilder::new("bunny-edge", Codespace::Triton)
            .with_unrestricted_access();
        builder.build_from_runner(&runner);
        let manifest = builder.build();

        assert_eq!(manifest.agent_count(), 1);
        assert!(manifest.has_agent("mock_threat_hunter"));
        assert!(manifest.unrestricted_access);
    }

    #[test]
    fn manifest_path_constant() {
        assert_eq!(AGENT_MANIFEST_PATH, "/bunny/agent/manifest");
    }
}
