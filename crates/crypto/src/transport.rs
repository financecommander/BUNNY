use tracing::info;

use crate::cipher::{CipherSuite, SwarmCipher};
use crate::envelope::SwarmEnvelope;
use crate::error::{CryptoError, Result};
use crate::identity::{NodeIdentity, SignedAnnouncement, SwarmNode};
use crate::kdf::derive_session_key;
use crate::session::{SessionStore, SwarmSession};
use crate::types::SessionId;

/// Handshake initiation message sent from initiator to responder.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HandshakeInit {
    pub announcement: SignedAnnouncement,
    pub x25519_public: [u8; 32],
    pub preferred_suite: CipherSuite,
}

/// Handshake response message sent from responder back to initiator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HandshakeAccept {
    pub announcement: SignedAnnouncement,
    pub x25519_public: [u8; 32],
    pub mlkem_ciphertext: Vec<u8>,
    pub agreed_suite: CipherSuite,
    pub session_id: SessionId,
}

/// Manages authenticated, encrypted node-to-node transport.
///
/// Flow:
/// 1. Initiator sends HandshakeInit (signed identity + ephemeral X25519 public)
/// 2. Responder verifies signature, checks trust, performs hybrid KEM
/// 3. Responder sends HandshakeAccept (signed identity + X25519 public + ML-KEM ciphertext)
/// 4. Initiator verifies signature, completes KEM, derives session key
/// 5. Both sides have an authenticated, encrypted session
pub struct NodeTransport {
    node: SwarmNode,
    sessions: SessionStore,
}

impl NodeTransport {
    pub fn new(node: SwarmNode) -> Self {
        let max_age = node.trust_policy.max_session_age_ms;
        Self {
            node,
            sessions: SessionStore::new(max_age),
        }
    }

    /// Initiate a handshake to a peer node.
    /// Returns the HandshakeInit to send and the ephemeral secret (kept locally).
    pub fn initiate_handshake(&self) -> (HandshakeInit, x25519_dalek::EphemeralSecret) {
        let ephemeral = x25519_dalek::EphemeralSecret::random_from_rng(rand::rngs::OsRng);
        let public = x25519_dalek::PublicKey::from(&ephemeral);

        let init = HandshakeInit {
            announcement: self.node.signed_announcement(),
            x25519_public: public.to_bytes(),
            preferred_suite: CipherSuite::auto_detect(),
        };

        (init, ephemeral)
    }

    /// Accept an incoming handshake from a peer.
    /// Verifies the peer's identity and trust, establishes a session.
    pub async fn accept_handshake(
        &self,
        init: &HandshakeInit,
    ) -> Result<HandshakeAccept> {
        // 1. Verify peer's signed announcement
        init.announcement.verify()?;

        // 2. Check trust policy
        if !self.node.is_peer_trusted(&init.announcement.identity) {
            return Err(CryptoError::ValidationFailed(format!(
                "peer role {:?} not trusted",
                init.announcement.identity.role
            )));
        }

        // 3. Generate ephemeral X25519 keypair
        let ephemeral = x25519_dalek::EphemeralSecret::random_from_rng(rand::rngs::OsRng);
        let public = x25519_dalek::PublicKey::from(&ephemeral);

        // 4. X25519 DH
        let peer_x25519 = x25519_dalek::PublicKey::from(init.x25519_public);
        let shared_classical = ephemeral.diffie_hellman(&peer_x25519);

        // 5. ML-KEM encapsulate using peer's KEM key
        use ml_kem::kem::Encapsulate;
        use ml_kem::{KemCore, MlKem768, EncodedSizeUser};
        let ek_array = hybrid_array::Array::try_from(
            init.announcement.identity.kem_key.as_slice(),
        )
        .map_err(|_| CryptoError::KemEncapsulationFailed)?;
        let mlkem_ek = <MlKem768 as KemCore>::EncapsulationKey::from_bytes(&ek_array);
        let (mlkem_ct, shared_pq) = mlkem_ek
            .encapsulate(&mut rand::rngs::OsRng)
            .map_err(|_| CryptoError::KemEncapsulationFailed)?;

        // 6. Derive session key from combined secrets
        let mut ikm = Vec::with_capacity(64);
        ikm.extend_from_slice(shared_classical.as_bytes());
        ikm.extend_from_slice(shared_pq.as_ref());
        let session_key = derive_session_key(&ikm, b"bunny-node-session")?;

        // 7. Create session
        let session_id = SessionId::new();
        let agreed_suite = init.preferred_suite;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let session = SwarmSession {
            session_id: session_id.clone(),
            peer: init.announcement.identity.node_id.clone(),
            cipher: SwarmCipher::with_suite(&session_key, agreed_suite),
            created_at: now,
            last_activity: now,
            send_sequence: 0,
            recv_sequence: 0,
        };
        self.sessions.insert(session).await;

        info!(
            peer = ?init.announcement.identity.node_id,
            role = ?init.announcement.identity.role,
            session = ?session_id,
            "handshake accepted"
        );

        // 8. Build response
        let accept = HandshakeAccept {
            announcement: self.node.signed_announcement(),
            x25519_public: public.to_bytes(),
            mlkem_ciphertext: mlkem_ct.as_slice().to_vec(),
            agreed_suite,
            session_id,
        };

        Ok(accept)
    }

    /// Complete the handshake after receiving the responder's accept.
    pub async fn complete_handshake(
        &self,
        accept: &HandshakeAccept,
        ephemeral_secret: x25519_dalek::EphemeralSecret,
    ) -> Result<SessionId> {
        // 1. Verify responder's signed announcement
        accept.announcement.verify()?;

        // 2. Check trust policy
        if !self.node.is_peer_trusted(&accept.announcement.identity) {
            return Err(CryptoError::ValidationFailed(format!(
                "peer role {:?} not trusted",
                accept.announcement.identity.role
            )));
        }

        // 3. X25519 DH
        let peer_x25519 = x25519_dalek::PublicKey::from(accept.x25519_public);
        let shared_classical = ephemeral_secret.diffie_hellman(&peer_x25519);

        // 4. ML-KEM decapsulate
        use ml_kem::kem::Decapsulate;
        let mlkem_ct_array =
            hybrid_array::Array::try_from(accept.mlkem_ciphertext.as_slice())
                .map_err(|_| CryptoError::KemDecapsulationFailed)?;
        let shared_pq = self
            .node
            .kem_dk()
            .decapsulate(&mlkem_ct_array)
            .map_err(|_| CryptoError::KemDecapsulationFailed)?;

        // 5. Derive session key
        let mut ikm = Vec::with_capacity(64);
        ikm.extend_from_slice(shared_classical.as_bytes());
        ikm.extend_from_slice(shared_pq.as_ref());
        let session_key = derive_session_key(&ikm, b"bunny-node-session")?;

        // 6. Create session
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let session = SwarmSession {
            session_id: accept.session_id.clone(),
            peer: accept.announcement.identity.node_id.clone(),
            cipher: SwarmCipher::with_suite(&session_key, accept.agreed_suite),
            created_at: now,
            last_activity: now,
            send_sequence: 0,
            recv_sequence: 0,
        };
        self.sessions.insert(session).await;

        info!(
            peer = ?accept.announcement.identity.node_id,
            session = ?accept.session_id,
            "handshake completed"
        );

        Ok(accept.session_id.clone())
    }

    /// Send an encrypted message to a peer on an established session.
    pub async fn send(
        &self,
        session_id: &SessionId,
        payload: &[u8],
    ) -> Result<SwarmEnvelope> {
        let agent_id = self.node.agent_id().clone();
        let result = self
            .sessions
            .get_mut(session_id, |session| session.seal(&agent_id, payload))
            .await;

        match result {
            Some(Ok(envelope)) => Ok(envelope),
            Some(Err(e)) => Err(e),
            None => Err(CryptoError::SessionNotFound(session_id.clone())),
        }
    }

    /// Receive and decrypt an envelope from a peer.
    pub async fn receive(
        &self,
        envelope: &SwarmEnvelope,
    ) -> Result<Vec<u8>> {
        let result = self
            .sessions
            .get_mut(&envelope.session_id, |session| session.open(envelope))
            .await;

        match result {
            Some(Ok(plaintext)) => Ok(plaintext),
            Some(Err(e)) => Err(e),
            None => Err(CryptoError::SessionNotFound(envelope.session_id.clone())),
        }
    }

    /// Rekey an existing session (generate new session key, keep session ID).
    pub async fn rekey(&self, session_id: &SessionId) -> Result<()> {
        // Generate fresh key material
        let new_key = crate::types::SymmetricKey::from_bytes({
            let mut buf = [0u8; 32];
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut buf);
            buf
        });

        let result = self
            .sessions
            .get_mut(session_id, |session| {
                let suite = session.cipher.suite();
                session.cipher = SwarmCipher::with_suite(&new_key, suite);
                session.send_sequence = 0;
                session.recv_sequence = 0;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                session.created_at = now;
                session.last_activity = now;
            })
            .await;

        match result {
            Some(()) => {
                info!(session = ?session_id, "session rekeyed");
                Ok(())
            }
            None => Err(CryptoError::SessionNotFound(session_id.clone())),
        }
    }

    /// Prune expired sessions.
    pub async fn prune_expired(&self) {
        self.sessions.prune_expired().await;
    }

    pub fn node_identity(&self) -> &NodeIdentity {
        &self.node.identity
    }

    pub fn session_store(&self) -> &SessionStore {
        &self.sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{NodeCapabilities, NodeRole};
    use crate::cipher::CipherSuite;

    fn test_caps() -> NodeCapabilities {
        NodeCapabilities {
            models: vec!["threat_classifier".into()],
            has_gpu: false,
            max_sessions: 8,
            cipher_suites: vec![CipherSuite::Aes256Gcm],
        }
    }

    #[tokio::test]
    async fn full_handshake_and_messaging() {
        let gateway = NodeTransport::new(
            SwarmNode::generate(NodeRole::Gateway, test_caps()),
        );
        let worker = NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, test_caps()),
        );

        // Worker initiates handshake to gateway
        let (init, ephemeral) = worker.initiate_handshake();

        // Gateway accepts
        let accept = gateway.accept_handshake(&init).await.unwrap();

        // Worker completes
        let session_id = worker.complete_handshake(&accept, ephemeral).await.unwrap();

        // Worker sends encrypted message
        let envelope = worker.send(&session_id, b"MODEL_CALL: ternary_llm").await.unwrap();

        // Gateway receives and decrypts
        let plaintext = gateway.receive(&envelope).await.unwrap();
        assert_eq!(plaintext, b"MODEL_CALL: ternary_llm");

        // Gateway sends encrypted response
        let response_env = gateway.send(&session_id, b"logits: [0.9, 0.1]").await.unwrap();

        // Worker receives response
        let response = worker.receive(&response_env).await.unwrap();
        assert_eq!(response, b"logits: [0.9, 0.1]");
    }

    #[tokio::test]
    async fn untrusted_role_rejected() {
        let gateway = NodeTransport::new(
            SwarmNode::generate(NodeRole::Gateway, test_caps())
                .with_trust_policy(crate::identity::TrustPolicy {
                    allowed_roles: vec![NodeRole::Worker], // only workers
                    ..Default::default()
                }),
        );
        let orchestrator = NodeTransport::new(
            SwarmNode::generate(NodeRole::Orchestrator, test_caps()),
        );

        let (init, _) = orchestrator.initiate_handshake();
        let result = gateway.accept_handshake(&init).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rekey_resets_sequences() {
        let node_a = NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, test_caps()),
        );
        let node_b = NodeTransport::new(
            SwarmNode::generate(NodeRole::Gateway, test_caps()),
        );

        let (init, eph) = node_a.initiate_handshake();
        let accept = node_b.accept_handshake(&init).await.unwrap();
        let session_id = node_a.complete_handshake(&accept, eph).await.unwrap();

        // Send a few messages
        node_a.send(&session_id, b"msg1").await.unwrap();
        node_a.send(&session_id, b"msg2").await.unwrap();

        // Rekey
        node_a.rekey(&session_id).await.unwrap();

        // Can still send after rekey (sequences reset)
        let env = node_a.send(&session_id, b"post-rekey").await.unwrap();
        assert_eq!(env.sequence, 0);
    }
}
