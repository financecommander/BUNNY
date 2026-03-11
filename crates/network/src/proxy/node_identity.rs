//! Node identity and role binding for the swarm topology.
//!
//! Each VM in the cluster gets a [`NodeManifest`] that declares its role,
//! capabilities, and network bindings. The manifest is signed by the node's
//! Ed25519 key and verified by peers during channel establishment.

use std::collections::HashMap;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode, TrustPolicy};
use bunny_crypto::types::AgentId;

/// High-level swarm role — maps to deployment topology.
///
/// More specific than `NodeRole` (which is a crypto-level concept).
/// `SwarmRole` captures what the node *does* in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SwarmRole {
    /// AI Portal gateway — terminates external TLS, creates encrypted
    /// internal channels. First point of contact for all external traffic.
    Portal,
    /// Swarm mainframe — central coordinator, runs Orchestra/OpenClaw,
    /// dispatches tasks, manages agent lifecycle.
    Coordinator,
    /// GPU inference worker — runs Ollama/Triton, executes model inference.
    /// May be temporarily unavailable (capacity constraints).
    GpuWorker,
    /// Calculus tools node — provides computation APIs (tensor ops,
    /// statistics, similarity search, symbolic evaluation).
    ComputeNode,
    /// Orchestra workflow engine — parses .orc files, dispatches task graphs
    /// across the swarm. Secondary coordinator for structured workflows.
    Orchestra,
    /// ProbFlow adaptive layer — uncertainty scoring, confidence-based routing.
    /// Evaluates model outputs and adjusts dispatch thresholds dynamically.
    ProbFlow,
}

impl SwarmRole {
    /// Map to the crypto-level `NodeRole` for trust policy enforcement.
    pub fn to_node_role(self) -> NodeRole {
        match self {
            SwarmRole::Portal => NodeRole::Gateway,
            SwarmRole::Coordinator => NodeRole::Orchestrator,
            SwarmRole::GpuWorker => NodeRole::Worker,
            SwarmRole::ComputeNode => NodeRole::Worker,
            SwarmRole::Orchestra => NodeRole::Orchestrator,
            SwarmRole::ProbFlow => NodeRole::Worker,
        }
    }

    /// Default trust policy: which roles can this node communicate with?
    pub fn default_trust_policy(self) -> TrustPolicy {
        match self {
            // Portal trusts coordinator (primary) and workers (health checks).
            SwarmRole::Portal => TrustPolicy {
                allowed_roles: vec![
                    NodeRole::Orchestrator,
                    NodeRole::Worker,
                    NodeRole::Gateway,
                ],
                require_pq: true,
                max_session_age_ms: 3_600_000, // 1 hour
                max_envelope_age_ms: 60_000,   // 1 minute
            },
            // Coordinator + Orchestra trust everyone — central authorities.
            SwarmRole::Coordinator | SwarmRole::Orchestra => TrustPolicy {
                allowed_roles: vec![
                    NodeRole::Gateway,
                    NodeRole::Worker,
                    NodeRole::Agent,
                    NodeRole::Orchestrator,
                ],
                require_pq: true,
                max_session_age_ms: 3_600_000,
                max_envelope_age_ms: 60_000,
            },
            // Workers only trust orchestrator and gateway.
            SwarmRole::GpuWorker | SwarmRole::ComputeNode | SwarmRole::ProbFlow => TrustPolicy {
                allowed_roles: vec![NodeRole::Orchestrator, NodeRole::Gateway],
                require_pq: true,
                max_session_age_ms: 1_800_000, // 30 min (tighter)
                max_envelope_age_ms: 30_000,   // 30 sec
            },
        }
    }
}

/// Network binding for a node — where it listens and where peers connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeBinding {
    /// Internal VPC address (e.g., 10.142.0.4:3000).
    pub internal: SocketAddr,
    /// External/public address if applicable (e.g., 34.148.140.31:3000).
    pub external: Option<SocketAddr>,
    /// BUNNY sidecar listen address (e.g., 127.0.0.1:9000).
    pub sidecar: SocketAddr,
}

/// Full identity manifest for a swarm node.
///
/// Created at node startup, signed, and shared with peers during
/// channel establishment. Includes role, capabilities, and bindings.
pub struct NodeManifest {
    /// The underlying crypto-level node identity.
    pub node: SwarmNode,
    /// What this node does in the cluster.
    pub role: SwarmRole,
    /// Network addresses.
    pub binding: NodeBinding,
    /// Human-readable name (e.g., "swarm-mainframe").
    pub name: String,
    /// Key-value metadata (zone, image, provider, etc.).
    pub metadata: HashMap<String, String>,
}

impl NodeManifest {
    /// Create a new node manifest with a fresh identity.
    pub fn generate(
        role: SwarmRole,
        name: impl Into<String>,
        binding: NodeBinding,
    ) -> Self {
        let capabilities = Self::default_capabilities(role);
        let trust = role.default_trust_policy();
        let node = SwarmNode::generate(role.to_node_role(), capabilities)
            .with_trust_policy(trust);

        Self {
            node,
            role,
            binding,
            name: name.into(),
            metadata: HashMap::new(),
        }
    }

    /// Add metadata key-value pair (builder pattern).
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get this node's agent ID.
    pub fn agent_id(&self) -> &AgentId {
        self.node.agent_id()
    }

    /// Produce a signed announcement for peer discovery.
    pub fn announcement(&self) -> bunny_crypto::identity::SignedAnnouncement {
        self.node.signed_announcement()
    }

    /// Default capabilities based on role.
    fn default_capabilities(role: SwarmRole) -> NodeCapabilities {
        match role {
            SwarmRole::Portal => NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 1000,
                cipher_suites: vec![
                    bunny_crypto::cipher::CipherSuite::Aes256Gcm,
                    bunny_crypto::cipher::CipherSuite::ChaCha20Poly1305,
                ],
            },
            SwarmRole::Coordinator => NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 500,
                cipher_suites: vec![
                    bunny_crypto::cipher::CipherSuite::Aes256Gcm,
                    bunny_crypto::cipher::CipherSuite::ChaCha20Poly1305,
                ],
            },
            SwarmRole::GpuWorker => NodeCapabilities {
                models: vec![
                    "llama3.1:8b".to_string(),
                ],
                has_gpu: true,
                max_sessions: 50,
                cipher_suites: vec![
                    bunny_crypto::cipher::CipherSuite::Aes256Gcm,
                ],
            },
            SwarmRole::ComputeNode => NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 200,
                cipher_suites: vec![
                    bunny_crypto::cipher::CipherSuite::Aes256Gcm,
                    bunny_crypto::cipher::CipherSuite::ChaCha20Poly1305,
                ],
            },
            SwarmRole::Orchestra => NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 300,
                cipher_suites: vec![
                    bunny_crypto::cipher::CipherSuite::Aes256Gcm,
                    bunny_crypto::cipher::CipherSuite::ChaCha20Poly1305,
                ],
            },
            SwarmRole::ProbFlow => NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 200,
                cipher_suites: vec![
                    bunny_crypto::cipher::CipherSuite::Aes256Gcm,
                    bunny_crypto::cipher::CipherSuite::ChaCha20Poly1305,
                ],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_binding() -> NodeBinding {
        NodeBinding {
            internal: "10.142.0.4:3000".parse().unwrap(),
            external: Some("34.148.140.31:3000".parse().unwrap()),
            sidecar: "127.0.0.1:9000".parse().unwrap(),
        }
    }

    #[test]
    fn generate_portal_manifest() {
        let m = NodeManifest::generate(
            SwarmRole::Portal,
            "fc-ai-portal",
            test_binding(),
        );
        assert_eq!(m.role, SwarmRole::Portal);
        assert_eq!(m.name, "fc-ai-portal");
        assert_eq!(m.role.to_node_role(), NodeRole::Gateway);
    }

    #[test]
    fn generate_coordinator_manifest() {
        let m = NodeManifest::generate(
            SwarmRole::Coordinator,
            "swarm-mainframe",
            NodeBinding {
                internal: "10.142.0.4:3000".parse().unwrap(),
                external: Some("34.148.140.31:3000".parse().unwrap()),
                sidecar: "127.0.0.1:9001".parse().unwrap(),
            },
        );
        assert_eq!(m.role, SwarmRole::Coordinator);
        let ann = m.announcement();
        assert!(ann.verify().is_ok());
    }

    #[test]
    fn role_trust_policies() {
        let portal_trust = SwarmRole::Portal.default_trust_policy();
        assert!(portal_trust.allowed_roles.contains(&NodeRole::Orchestrator));
        assert!(portal_trust.require_pq);

        let worker_trust = SwarmRole::GpuWorker.default_trust_policy();
        assert!(!worker_trust.allowed_roles.contains(&NodeRole::Worker));
        assert!(worker_trust.allowed_roles.contains(&NodeRole::Orchestrator));
    }

    #[test]
    fn manifest_metadata() {
        let m = NodeManifest::generate(SwarmRole::GpuWorker, "swarm-gpu", test_binding())
            .with_metadata("zone", "us-east1-b")
            .with_metadata("gpu", "nvidia-l4");
        assert_eq!(m.metadata.get("zone").unwrap(), "us-east1-b");
        assert_eq!(m.metadata.get("gpu").unwrap(), "nvidia-l4");
    }

    #[test]
    fn generate_orchestra_manifest() {
        let m = NodeManifest::generate(
            SwarmRole::Orchestra,
            "orchestra-engine",
            test_binding(),
        );
        assert_eq!(m.role, SwarmRole::Orchestra);
        assert_eq!(m.name, "orchestra-engine");
        assert_eq!(m.role.to_node_role(), NodeRole::Orchestrator);
        let ann = m.announcement();
        assert!(ann.verify().is_ok());
    }

    #[test]
    fn generate_probflow_manifest() {
        let m = NodeManifest::generate(
            SwarmRole::ProbFlow,
            "probflow-adaptive",
            test_binding(),
        );
        assert_eq!(m.role, SwarmRole::ProbFlow);
        assert_eq!(m.name, "probflow-adaptive");
        assert_eq!(m.role.to_node_role(), NodeRole::Worker);
        let ann = m.announcement();
        assert!(ann.verify().is_ok());
    }

    #[test]
    fn orchestra_trusts_workers() {
        let trust = SwarmRole::Orchestra.default_trust_policy();
        assert!(trust.allowed_roles.contains(&NodeRole::Worker));
        assert!(trust.allowed_roles.contains(&NodeRole::Gateway));
        assert!(trust.allowed_roles.contains(&NodeRole::Agent));
        assert!(trust.require_pq);
    }

    #[test]
    fn probflow_trusts_orchestrator() {
        let trust = SwarmRole::ProbFlow.default_trust_policy();
        assert!(trust.allowed_roles.contains(&NodeRole::Orchestrator));
        assert!(trust.allowed_roles.contains(&NodeRole::Gateway));
        assert!(!trust.allowed_roles.contains(&NodeRole::Worker));
    }

    #[test]
    fn announcement_is_signed() {
        let m = NodeManifest::generate(SwarmRole::ComputeNode, "calculus-web", test_binding());
        let ann = m.announcement();
        assert!(ann.verify().is_ok());
    }
}
