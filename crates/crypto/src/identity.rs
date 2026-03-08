use ed25519_dalek::{Signer, SigningKey, VerifyingKey, Signature, Verifier};
use ml_kem::{EncodedSizeUser, KemCore, MlKem768};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

use crate::error::{CryptoError, Result};
use crate::types::AgentId;

/// Role a node plays in the swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeRole {
    /// Gateway node — decrypts at execution boundary, routes to Triton.
    Gateway,
    /// Worker node — executes ternary inference via Triton runtime.
    Worker,
    /// Orchestrator — dispatches tasks from super-duper-spork / Orchestra.
    Orchestrator,
    /// Agent — security agent (Threat Hunter, AI Guardian, etc.)
    Agent,
    /// Relay — forwards encrypted traffic without decryption.
    Relay,
}

/// Capabilities a node advertises to the swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    /// Triton models this node can execute.
    pub models: Vec<String>,
    /// Whether this node has GPU.
    pub has_gpu: bool,
    /// Maximum concurrent inference sessions.
    pub max_sessions: u32,
    /// Supported cipher suites.
    pub cipher_suites: Vec<crate::cipher::CipherSuite>,
}

impl NodeCapabilities {
    /// Compute a deterministic hash of capabilities for quick comparison.
    pub fn capability_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for model in &self.models {
            hasher.update(model.as_bytes());
            hasher.update(b"|");
        }
        hasher.update(if self.has_gpu { b"gpu" } else { b"cpu" });
        hasher.update(&self.max_sessions.to_be_bytes());
        hasher.finalize().into()
    }
}

/// Trust policy controlling what this node will accept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPolicy {
    /// Only accept sessions from nodes with these roles.
    pub allowed_roles: Vec<NodeRole>,
    /// Require post-quantum key exchange.
    pub require_pq: bool,
    /// Maximum session age before mandatory rekey (ms).
    pub max_session_age_ms: u64,
    /// Maximum envelope age to accept (ms).
    pub max_envelope_age_ms: u64,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self {
            allowed_roles: vec![
                NodeRole::Gateway,
                NodeRole::Worker,
                NodeRole::Orchestrator,
                NodeRole::Agent,
                NodeRole::Relay,
            ],
            require_pq: true,
            max_session_age_ms: 3_600_000, // 1 hour
            max_envelope_age_ms: 60_000,   // 60 seconds
        }
    }
}

/// Public identity of a swarm node — safe to share on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub node_id: AgentId,
    pub role: NodeRole,
    pub signing_key: [u8; 32],
    pub kem_key: Vec<u8>,
    pub capabilities: NodeCapabilities,
    pub capability_hash: [u8; 32],
}

/// Private key material for a swarm node — never leaves the node.
pub struct NodeKeyPair {
    signing: SigningKey,
    kem_dk: <MlKem768 as KemCore>::DecapsulationKey,
    kem_ek: <MlKem768 as KemCore>::EncapsulationKey,
}

impl Drop for NodeKeyPair {
    fn drop(&mut self) {
        // SigningKey implements Zeroize internally via ed25519-dalek
        let _ = &self.signing;
    }
}

/// A fully initialized swarm node with identity and key material.
pub struct SwarmNode {
    pub identity: NodeIdentity,
    keys: NodeKeyPair,
    pub trust_policy: TrustPolicy,
}

impl SwarmNode {
    /// Generate a new swarm node with fresh keys.
    pub fn generate(role: NodeRole, capabilities: NodeCapabilities) -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();

        let (kem_dk, kem_ek) = MlKem768::generate(&mut OsRng);
        let capability_hash = capabilities.capability_hash();

        let identity = NodeIdentity {
            node_id: AgentId::new(),
            role,
            signing_key: verifying.to_bytes(),
            kem_key: kem_ek.as_bytes().to_vec(),
            capabilities,
            capability_hash,
        };

        let keys = NodeKeyPair {
            signing,
            kem_dk,
            kem_ek,
        };

        Self {
            identity,
            keys,
            trust_policy: TrustPolicy::default(),
        }
    }

    /// Set a custom trust policy.
    pub fn with_trust_policy(mut self, policy: TrustPolicy) -> Self {
        self.trust_policy = policy;
        self
    }

    /// Sign arbitrary data with this node's signing key.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        self.keys.signing.sign(data).to_bytes().to_vec()
    }

    /// Verify a signature from a peer using their public identity.
    pub fn verify_peer(
        peer: &NodeIdentity,
        data: &[u8],
        signature: &[u8],
    ) -> Result<()> {
        let verifying = VerifyingKey::from_bytes(&peer.signing_key)
            .map_err(|e| CryptoError::KeyExchangeFailed(format!("invalid signing key: {e}")))?;
        let sig_bytes: [u8; 64] = signature
            .try_into()
            .map_err(|_| CryptoError::ValidationFailed("invalid signature length".into()))?;
        let sig = Signature::from_bytes(&sig_bytes);
        verifying
            .verify(data, &sig)
            .map_err(|_| CryptoError::ValidationFailed("signature verification failed".into()))
    }

    /// Check if a peer's role is allowed by this node's trust policy.
    pub fn is_peer_trusted(&self, peer: &NodeIdentity) -> bool {
        self.trust_policy.allowed_roles.contains(&peer.role)
    }

    /// Get the node's AgentId (for use with existing session/envelope APIs).
    pub fn agent_id(&self) -> &AgentId {
        &self.identity.node_id
    }

    /// Get the KEM decapsulation key (for responding to key exchange).
    pub(crate) fn kem_dk(&self) -> &<MlKem768 as KemCore>::DecapsulationKey {
        &self.keys.kem_dk
    }

    /// Get the KEM encapsulation key.
    pub(crate) fn kem_ek(&self) -> &<MlKem768 as KemCore>::EncapsulationKey {
        &self.keys.kem_ek
    }

    /// Create a signed identity announcement for broadcasting to the swarm.
    pub fn signed_announcement(&self) -> SignedAnnouncement {
        let payload = serde_json::to_vec(&self.identity).unwrap();
        let signature = self.sign(&payload);
        SignedAnnouncement {
            identity: self.identity.clone(),
            signature,
        }
    }
}

/// A node identity with a signature proving ownership of the signing key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedAnnouncement {
    pub identity: NodeIdentity,
    pub signature: Vec<u8>,
}

impl SignedAnnouncement {
    /// Verify the announcement is genuinely signed by the claimed node.
    pub fn verify(&self) -> Result<()> {
        let payload = serde_json::to_vec(&self.identity)
            .map_err(|e| CryptoError::ValidationFailed(format!("serialization: {e}")))?;
        SwarmNode::verify_peer(&self.identity, &payload, &self.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::CipherSuite;

    fn test_capabilities() -> NodeCapabilities {
        NodeCapabilities {
            models: vec!["threat_classifier".into(), "packet_filter".into()],
            has_gpu: false,
            max_sessions: 16,
            cipher_suites: vec![CipherSuite::Aes256Gcm, CipherSuite::ChaCha20Poly1305],
        }
    }

    #[test]
    fn generate_and_sign_verify() {
        let node = SwarmNode::generate(NodeRole::Worker, test_capabilities());
        let data = b"test message";
        let sig = node.sign(data);

        // Self-verify via public identity
        SwarmNode::verify_peer(&node.identity, data, &sig).unwrap();
    }

    #[test]
    fn bad_signature_fails() {
        let node = SwarmNode::generate(NodeRole::Worker, test_capabilities());
        let sig = node.sign(b"real data");

        let result = SwarmNode::verify_peer(&node.identity, b"tampered data", &sig);
        assert!(result.is_err());
    }

    #[test]
    fn signed_announcement_roundtrip() {
        let node = SwarmNode::generate(NodeRole::Gateway, test_capabilities());
        let announcement = node.signed_announcement();

        // Verify the announcement
        announcement.verify().unwrap();

        // Serialize/deserialize
        let json = serde_json::to_string(&announcement).unwrap();
        let restored: SignedAnnouncement = serde_json::from_str(&json).unwrap();
        restored.verify().unwrap();

        assert_eq!(restored.identity.node_id, node.identity.node_id);
        assert_eq!(restored.identity.role, NodeRole::Gateway);
    }

    #[test]
    fn trust_policy_role_check() {
        let gateway = SwarmNode::generate(
            NodeRole::Gateway,
            test_capabilities(),
        ).with_trust_policy(TrustPolicy {
            allowed_roles: vec![NodeRole::Worker, NodeRole::Agent],
            ..Default::default()
        });

        let worker = SwarmNode::generate(NodeRole::Worker, test_capabilities());
        let orchestrator = SwarmNode::generate(NodeRole::Orchestrator, test_capabilities());

        assert!(gateway.is_peer_trusted(&worker.identity));
        assert!(!gateway.is_peer_trusted(&orchestrator.identity));
    }

    #[test]
    fn capability_hash_deterministic() {
        let caps = test_capabilities();
        let h1 = caps.capability_hash();
        let h2 = caps.capability_hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_capabilities_different_hash() {
        let caps1 = test_capabilities();
        let caps2 = NodeCapabilities {
            models: vec!["malware_classifier".into()],
            has_gpu: true,
            max_sessions: 4,
            cipher_suites: vec![CipherSuite::ChaCha20Poly1305],
        };
        assert_ne!(caps1.capability_hash(), caps2.capability_hash());
    }
}
