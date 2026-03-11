//! Signed request envelopes for authenticated inter-node communication.
//!
//! Every request flowing through the BUNNY proxy is wrapped in a
//! [`SignedRequest`] that proves the sender's identity and prevents
//! tampering. This is the inner application-level authentication —
//! on top of the transport-level encryption.
//!
//! Stack:
//! ```text
//! App payload (HTTP request bytes)
//!   → SignedRequest (Ed25519 signature + metadata)
//!     → EncryptedChannel.seal() (AES-256-GCM / CloakedEnvelope)
//!       → TCP wire
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use bunny_crypto::identity::{NodeIdentity, SwarmNode};
use bunny_crypto::types::AgentId;

use crate::error::{NetworkError, Result};

/// A signed request envelope — wraps any payload with sender proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedRequest {
    /// Sender's agent ID.
    pub sender: AgentId,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Monotonic request counter (per-sender).
    pub sequence: u64,
    /// Target service path (e.g., "/api/v1/slack/events").
    pub target: String,
    /// The actual payload bytes (HTTP request, tool call, etc.).
    pub payload: Vec<u8>,
    /// Ed25519 signature over (sender || timestamp || sequence || target || payload).
    pub signature: Vec<u8>,
}

impl SignedRequest {
    /// Serialize the fields that are signed (everything except signature).
    fn signing_material(
        sender: &AgentId,
        timestamp_ms: u64,
        sequence: u64,
        target: &str,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut material = Vec::with_capacity(16 + 8 + 8 + target.len() + payload.len());
        material.extend_from_slice(sender.as_bytes());
        material.extend_from_slice(&timestamp_ms.to_be_bytes());
        material.extend_from_slice(&sequence.to_be_bytes());
        material.extend_from_slice(target.as_bytes());
        material.extend_from_slice(payload);
        material
    }

    /// Verify this request was signed by the claimed sender.
    pub fn verify(&self, peer_identity: &NodeIdentity) -> Result<()> {
        let material = Self::signing_material(
            &self.sender,
            self.timestamp_ms,
            self.sequence,
            &self.target,
            &self.payload,
        );

        SwarmNode::verify_peer(peer_identity, &material, &self.signature)
            .map_err(|e| NetworkError::Protocol(format!("signature verification failed: {e}")))
    }

    /// Check if the request is within acceptable time bounds.
    pub fn is_fresh(&self, max_age_ms: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Allow max_age_ms in the past and 30 seconds in the future (clock skew).
        let age = now.saturating_sub(self.timestamp_ms);
        let future = self.timestamp_ms.saturating_sub(now);

        age <= max_age_ms && future <= 30_000
    }
}

/// Builds and signs outbound requests.
pub struct RequestSigner {
    /// Our node identity (for signing).
    node: SwarmNode,
    /// Monotonic sequence counter.
    sequence: u64,
}

impl RequestSigner {
    /// Create a signer backed by a node's Ed25519 key.
    pub fn new(node: SwarmNode) -> Self {
        Self { node, sequence: 0 }
    }

    /// Sign a request payload.
    pub fn sign(&mut self, target: impl Into<String>, payload: Vec<u8>) -> SignedRequest {
        let sender = self.node.agent_id().clone();
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let sequence = self.sequence;
        self.sequence += 1;

        let target = target.into();
        let material = SignedRequest::signing_material(
            &sender, timestamp_ms, sequence, &target, &payload,
        );
        let signature = self.node.sign(&material);

        SignedRequest {
            sender,
            timestamp_ms,
            sequence,
            target,
            payload,
            signature,
        }
    }

    /// Current sequence number (for diagnostics).
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Verifies incoming signed requests.
pub struct RequestVerifier {
    /// Maximum acceptable age for incoming requests.
    max_age_ms: u64,
    /// Track highest seen sequence per sender (anti-replay).
    seen_sequences: std::collections::HashMap<AgentId, u64>,
}

impl RequestVerifier {
    /// Create a verifier with the given freshness window.
    pub fn new(max_age_ms: u64) -> Self {
        Self {
            max_age_ms,
            seen_sequences: std::collections::HashMap::new(),
        }
    }

    /// Verify a signed request: check signature, freshness, and replay.
    pub fn verify(&mut self, request: &SignedRequest, peer: &NodeIdentity) -> Result<()> {
        // 1. Verify Ed25519 signature.
        request.verify(peer)?;

        // 2. Check timestamp freshness.
        if !request.is_fresh(self.max_age_ms) {
            return Err(NetworkError::Protocol(
                format!("request too old or from future (ts={})", request.timestamp_ms)
            ));
        }

        // 3. Anti-replay: sequence must advance.
        let last_seq = self.seen_sequences.get(&request.sender).copied().unwrap_or(0);
        if request.sequence < last_seq {
            return Err(NetworkError::Protocol(
                format!(
                    "replay detected: seq {} < last seen {}",
                    request.sequence, last_seq,
                )
            ));
        }
        self.seen_sequences.insert(request.sender.clone(), request.sequence);

        Ok(())
    }

    /// Number of tracked senders.
    pub fn tracked_senders(&self) -> usize {
        self.seen_sequences.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};

    fn make_node(role: NodeRole) -> SwarmNode {
        SwarmNode::generate(
            role,
            NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            },
        )
    }

    #[test]
    fn sign_and_verify() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);

        let req = signer.sign("/api/v1/chat", b"hello world".to_vec());
        assert_eq!(req.sequence, 0);
        assert_eq!(req.target, "/api/v1/chat");
        assert_eq!(req.payload, b"hello world");

        // Verify succeeds with correct identity.
        assert!(req.verify(&identity).is_ok());
    }

    #[test]
    fn tampered_payload_fails() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);

        let mut req = signer.sign("/api/v1/chat", b"hello".to_vec());
        req.payload = b"tampered".to_vec();

        assert!(req.verify(&identity).is_err());
    }

    #[test]
    fn wrong_identity_fails() {
        let node_a = make_node(NodeRole::Gateway);
        let node_b = make_node(NodeRole::Worker);
        let identity_b = node_b.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node_a);

        let req = signer.sign("/api/v1/chat", b"hello".to_vec());

        // Verifying with B's identity should fail.
        assert!(req.verify(&identity_b).is_err());
    }

    #[test]
    fn sequence_advances() {
        let node = make_node(NodeRole::Orchestrator);
        let mut signer = RequestSigner::new(node);

        let r0 = signer.sign("/a", vec![]);
        let r1 = signer.sign("/b", vec![]);
        let r2 = signer.sign("/c", vec![]);

        assert_eq!(r0.sequence, 0);
        assert_eq!(r1.sequence, 1);
        assert_eq!(r2.sequence, 2);
        assert_eq!(signer.sequence(), 3);
    }

    #[test]
    fn freshness_check() {
        let node = make_node(NodeRole::Gateway);
        let mut signer = RequestSigner::new(node);

        let req = signer.sign("/test", vec![]);
        assert!(req.is_fresh(60_000)); // within 60 seconds

        // Manually forge an old request.
        let old = SignedRequest {
            sender: req.sender,
            timestamp_ms: 1_000_000, // year 1970
            sequence: 99,
            target: "/old".into(),
            payload: vec![],
            signature: vec![], // won't verify anyway
        };
        assert!(!old.is_fresh(60_000));
    }

    #[test]
    fn verifier_detects_replay() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);
        let mut verifier = RequestVerifier::new(60_000);

        let r0 = signer.sign("/a", vec![]);
        let r1 = signer.sign("/b", vec![]);

        // Verify r1 first (seq=1).
        assert!(verifier.verify(&r1, &identity).is_ok());

        // Then r0 (seq=0) — should be rejected as replay.
        assert!(verifier.verify(&r0, &identity).is_err());
    }

    #[test]
    fn verifier_allows_advancing_sequence() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);
        let mut verifier = RequestVerifier::new(60_000);

        let r0 = signer.sign("/a", vec![]);
        let r1 = signer.sign("/b", vec![]);
        let r2 = signer.sign("/c", vec![]);

        assert!(verifier.verify(&r0, &identity).is_ok());
        assert!(verifier.verify(&r1, &identity).is_ok());
        assert!(verifier.verify(&r2, &identity).is_ok());
        assert_eq!(verifier.tracked_senders(), 1);
    }

    #[test]
    fn serialization_roundtrip() {
        let node = make_node(NodeRole::Gateway);
        let mut signer = RequestSigner::new(node);

        let req = signer.sign("/api/v1/infer", b"model data".to_vec());
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: SignedRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.sender, req.sender);
        assert_eq!(deserialized.target, req.target);
        assert_eq!(deserialized.payload, req.payload);
        assert_eq!(deserialized.signature, req.signature);
    }
}
