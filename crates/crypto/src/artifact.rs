use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::info;

use crate::error::{CryptoError, Result};
use crate::identity::{NodeIdentity, SwarmNode};
use crate::types::AgentId;

/// Types of artifacts that flow through the swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactType {
    /// Packed 2-bit ternary model weights.
    TernaryWeights,
    /// Safetensors format model file.
    Safetensors,
    /// Shard manifest describing how a model is split.
    ShardManifest,
    /// KV-cache snapshot for session continuity.
    KvSnapshot,
    /// Federated learning checkpoint.
    Checkpoint,
    /// Compiled Triton kernel.
    TritonKernel,
}

/// A single link in the signature chain — one node's attestation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainLink {
    /// Who signed this link.
    pub signer: NodeIdentity,
    /// Ed25519 signature over the artifact hash + previous chain hash.
    pub signature: Vec<u8>,
    /// Hash of the chain state after this link (hash of previous + this signature).
    pub chain_hash: [u8; 32],
    /// Timestamp of this attestation.
    pub signed_at: u64,
    /// Optional annotation (e.g., "validated", "distributed", "stored").
    pub annotation: String,
}

/// Signed artifact manifest — proves provenance and integrity.
///
/// The chain works like a mini-blockchain:
/// 1. Creator signs the artifact hash → first ChainLink
/// 2. Each handler (relay, orchestrator, worker) counter-signs →
///    appends a ChainLink whose `chain_hash = SHA-256(prev_chain_hash || signature)`
/// 3. Any node can verify the full chain from creator to current holder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactManifest {
    /// SHA-256 hash of the raw artifact data.
    pub artifact_hash: [u8; 32],
    /// Size of the artifact in bytes.
    pub artifact_size: u64,
    /// What kind of artifact this is.
    pub artifact_type: ArtifactType,
    /// Human-readable name (e.g., model name, shard ID).
    pub name: String,
    /// Ordered chain of signatures from creator through each handler.
    pub chain: Vec<ChainLink>,
}

impl ArtifactManifest {
    /// Create a new manifest and sign it as the creator.
    pub fn create(
        artifact_data: &[u8],
        artifact_type: ArtifactType,
        name: String,
        signer: &SwarmNode,
    ) -> Self {
        let artifact_hash = Self::hash_data(artifact_data);
        let artifact_size = artifact_data.len() as u64;

        let mut manifest = Self {
            artifact_hash,
            artifact_size,
            artifact_type,
            name,
            chain: Vec::new(),
        };

        // Creator signs the artifact hash directly
        manifest.append_signature(signer, "created");

        info!(
            artifact_type = ?artifact_type,
            size = artifact_size,
            signer = ?signer.agent_id(),
            "artifact manifest created"
        );

        manifest
    }

    /// Append a signature to the chain (counter-sign as a handler).
    pub fn append_signature(&mut self, signer: &SwarmNode, annotation: &str) {
        let sign_payload = self.sign_payload();
        let signature = signer.sign(&sign_payload);

        let chain_hash = self.compute_chain_hash(&signature);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        self.chain.push(ChainLink {
            signer: signer.identity.clone(),
            signature,
            chain_hash,
            signed_at: now,
            annotation: annotation.to_string(),
        });
    }

    /// Verify the entire signature chain from creator to current holder.
    pub fn verify_chain(&self) -> Result<()> {
        if self.chain.is_empty() {
            return Err(CryptoError::ValidationFailed(
                "artifact has no signatures".into(),
            ));
        }

        let mut prev_chain_hash: Option<[u8; 32]> = None;

        for (i, link) in self.chain.iter().enumerate() {
            // Reconstruct the payload this link signed
            let sign_payload = if i == 0 {
                // First signer signs artifact_hash directly
                self.artifact_hash.to_vec()
            } else {
                // Subsequent signers sign artifact_hash || prev_chain_hash
                let mut payload = Vec::with_capacity(64);
                payload.extend_from_slice(&self.artifact_hash);
                payload.extend_from_slice(&prev_chain_hash.unwrap());
                payload
            };

            // Verify the signature
            SwarmNode::verify_peer(&link.signer, &sign_payload, &link.signature).map_err(
                |_| {
                    CryptoError::ValidationFailed(format!(
                        "signature verification failed at chain link {i} (signer: {:?})",
                        link.signer.node_id
                    ))
                },
            )?;

            // Verify chain hash continuity
            let expected_hash = if let Some(prev) = prev_chain_hash {
                let mut hasher = Sha256::new();
                hasher.update(prev);
                hasher.update(&link.signature);
                let h: [u8; 32] = hasher.finalize().into();
                h
            } else {
                let mut hasher = Sha256::new();
                hasher.update([0u8; 32]);
                hasher.update(&link.signature);
                let h: [u8; 32] = hasher.finalize().into();
                h
            };

            if link.chain_hash != expected_hash {
                return Err(CryptoError::ValidationFailed(format!(
                    "chain hash mismatch at link {i}"
                )));
            }

            prev_chain_hash = Some(link.chain_hash);
        }

        Ok(())
    }

    /// Verify the chain AND that the artifact data matches the manifest hash.
    pub fn verify_artifact(&self, artifact_data: &[u8]) -> Result<()> {
        let computed_hash = Self::hash_data(artifact_data);
        if computed_hash != self.artifact_hash {
            return Err(CryptoError::ValidationFailed(
                "artifact data hash mismatch".into(),
            ));
        }
        if artifact_data.len() as u64 != self.artifact_size {
            return Err(CryptoError::ValidationFailed(
                "artifact data size mismatch".into(),
            ));
        }
        self.verify_chain()
    }

    /// Get the creator (first signer) of this artifact.
    pub fn creator(&self) -> Option<&AgentId> {
        self.chain.first().map(|link| &link.signer.node_id)
    }

    /// Get the current holder (last signer) of this artifact.
    pub fn current_holder(&self) -> Option<&AgentId> {
        self.chain.last().map(|link| &link.signer.node_id)
    }

    /// Get the number of signatures in the chain.
    pub fn chain_length(&self) -> usize {
        self.chain.len()
    }

    /// Compute the payload to sign (artifact_hash || last chain_hash).
    fn sign_payload(&self) -> Vec<u8> {
        if let Some(last) = self.chain.last() {
            let mut payload = Vec::with_capacity(64);
            payload.extend_from_slice(&self.artifact_hash);
            payload.extend_from_slice(&last.chain_hash);
            payload
        } else {
            self.artifact_hash.to_vec()
        }
    }

    /// Compute the new chain hash after adding a signature.
    fn compute_chain_hash(&self, signature: &[u8]) -> [u8; 32] {
        let prev = self
            .chain
            .last()
            .map(|l| l.chain_hash)
            .unwrap_or([0u8; 32]);
        let mut hasher = Sha256::new();
        hasher.update(prev);
        hasher.update(signature);
        hasher.finalize().into()
    }

    fn hash_data(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::CipherSuite;
    use crate::identity::{NodeCapabilities, NodeRole, SwarmNode};

    fn test_caps() -> NodeCapabilities {
        NodeCapabilities {
            models: vec!["threat_classifier".into()],
            has_gpu: false,
            max_sessions: 8,
            cipher_suites: vec![CipherSuite::Aes256Gcm],
        }
    }

    #[test]
    fn create_and_verify_manifest() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let artifact_data = b"packed ternary weights 0xAA 0xBB 0xCC";

        let manifest = ArtifactManifest::create(
            artifact_data,
            ArtifactType::TernaryWeights,
            "threat_classifier_v1".into(),
            &creator,
        );

        assert_eq!(manifest.chain_length(), 1);
        assert_eq!(manifest.creator().unwrap(), creator.agent_id());
        manifest.verify_chain().unwrap();
        manifest.verify_artifact(artifact_data).unwrap();
    }

    #[test]
    fn multi_signer_chain() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let relay = SwarmNode::generate(NodeRole::Relay, test_caps());
        let worker = SwarmNode::generate(NodeRole::Worker, test_caps());

        let artifact_data = b"model shard data";

        let mut manifest = ArtifactManifest::create(
            artifact_data,
            ArtifactType::TernaryWeights,
            "big_model_shard_0".into(),
            &creator,
        );

        // Relay counter-signs
        manifest.append_signature(&relay, "relayed");
        assert_eq!(manifest.chain_length(), 2);

        // Worker counter-signs
        manifest.append_signature(&worker, "received");
        assert_eq!(manifest.chain_length(), 3);

        // Full chain verifies
        manifest.verify_chain().unwrap();
        manifest.verify_artifact(artifact_data).unwrap();

        // Check creator and holder
        assert_eq!(manifest.creator().unwrap(), creator.agent_id());
        assert_eq!(manifest.current_holder().unwrap(), worker.agent_id());
    }

    #[test]
    fn tampered_artifact_rejected() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let artifact_data = b"original weights";

        let manifest = ArtifactManifest::create(
            artifact_data,
            ArtifactType::TernaryWeights,
            "model_v1".into(),
            &creator,
        );

        // Tampered data fails verification
        let tampered = b"modified weights";
        assert!(manifest.verify_artifact(tampered).is_err());
    }

    #[test]
    fn tampered_signature_rejected() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let artifact_data = b"real data";

        let mut manifest = ArtifactManifest::create(
            artifact_data,
            ArtifactType::Safetensors,
            "safe_model".into(),
            &creator,
        );

        // Tamper with the signature
        if let Some(link) = manifest.chain.first_mut() {
            link.signature[0] ^= 0xFF;
        }

        assert!(manifest.verify_chain().is_err());
    }

    #[test]
    fn tampered_chain_hash_rejected() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let relay = SwarmNode::generate(NodeRole::Relay, test_caps());
        let artifact_data = b"weights";

        let mut manifest = ArtifactManifest::create(
            artifact_data,
            ArtifactType::TernaryWeights,
            "model".into(),
            &creator,
        );
        manifest.append_signature(&relay, "relayed");

        // Tamper with chain hash
        manifest.chain[1].chain_hash[0] ^= 0xFF;

        assert!(manifest.verify_chain().is_err());
    }

    #[test]
    fn empty_chain_rejected() {
        let manifest = ArtifactManifest {
            artifact_hash: [0u8; 32],
            artifact_size: 0,
            artifact_type: ArtifactType::Checkpoint,
            name: "empty".into(),
            chain: vec![],
        };

        assert!(manifest.verify_chain().is_err());
    }

    #[test]
    fn shard_manifest_type() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let manifest_data = b"{\"shards\": 8, \"model\": \"big_model\"}";

        let manifest = ArtifactManifest::create(
            manifest_data,
            ArtifactType::ShardManifest,
            "big_model_manifest".into(),
            &creator,
        );

        assert_eq!(manifest.artifact_type, ArtifactType::ShardManifest);
        manifest.verify_artifact(manifest_data).unwrap();
    }

    #[test]
    fn kv_snapshot_artifact() {
        let worker = SwarmNode::generate(NodeRole::Worker, test_caps());
        let kv_data = vec![0xCC; 1024]; // fake KV-cache

        let manifest = ArtifactManifest::create(
            &kv_data,
            ArtifactType::KvSnapshot,
            "session_42_kv".into(),
            &worker,
        );

        assert_eq!(manifest.artifact_type, ArtifactType::KvSnapshot);
        assert_eq!(manifest.artifact_size, 1024);
        manifest.verify_artifact(&kv_data).unwrap();
    }

    #[test]
    fn manifest_serialization_roundtrip() {
        let creator = SwarmNode::generate(NodeRole::Orchestrator, test_caps());
        let relay = SwarmNode::generate(NodeRole::Relay, test_caps());
        let data = b"artifact data";

        let mut manifest = ArtifactManifest::create(
            data,
            ArtifactType::TritonKernel,
            "compiled_kernel".into(),
            &creator,
        );
        manifest.append_signature(&relay, "distributed");

        // Serialize → deserialize
        let json = serde_json::to_string(&manifest).unwrap();
        let restored: ArtifactManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.artifact_hash, manifest.artifact_hash);
        assert_eq!(restored.chain_length(), 2);
        restored.verify_chain().unwrap();
        restored.verify_artifact(data).unwrap();
    }
}
