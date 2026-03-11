//! Encrypted channel management between BUNNY nodes.
//!
//! A [`EncryptedChannel`] wraps the `NodeTransport` handshake flow into
//! a high-level API: initiate → accept → established. Once established,
//! the channel provides `seal()` / `open()` for bidirectional encrypted
//! messaging with automatic session management and rekeying.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use bunny_crypto::cloaked::{CloakedEnvelope, OpaqueRouteTag, TrafficPolicy};
use bunny_crypto::envelope::SwarmEnvelope;
use bunny_crypto::identity::NodeIdentity;
use bunny_crypto::types::SessionId;
use bunny_crypto::transport::{HandshakeAccept, HandshakeInit, NodeTransport};

use crate::error::{NetworkError, Result};

/// Configuration for an encrypted channel.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// How often to rotate keys (forward secrecy).
    pub rekey_interval: Duration,
    /// Use cloaked envelopes (hide sender/timing metadata).
    pub cloaked: bool,
    /// Traffic shaping policy.
    pub traffic_policy: TrafficPolicy,
    /// Maximum message size before chunking.
    pub max_message_bytes: usize,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            rekey_interval: Duration::from_secs(900), // 15 min
            cloaked: true,
            traffic_policy: TrafficPolicy::balanced(),
            max_message_bytes: 4 * 1024 * 1024, // 4 MB
        }
    }
}

impl ChannelConfig {
    /// High-security config: cloaked + full traffic shaping.
    pub fn high_security() -> Self {
        Self {
            rekey_interval: Duration::from_secs(300), // 5 min
            cloaked: true,
            traffic_policy: TrafficPolicy::cloaked(),
            max_message_bytes: 4 * 1024 * 1024,
        }
    }

    /// Performance config: standard envelopes, minimal overhead.
    pub fn performance() -> Self {
        Self {
            rekey_interval: Duration::from_secs(3600), // 1 hour
            cloaked: false,
            traffic_policy: TrafficPolicy::disabled(),
            max_message_bytes: 16 * 1024 * 1024, // 16 MB
        }
    }
}

/// State of a channel through its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel created, no handshake started.
    Idle,
    /// Handshake initiated, waiting for peer response.
    HandshakeSent,
    /// Handshake accepted, waiting for initiator to complete.
    HandshakeReceived,
    /// Channel established — ready for encrypted messaging.
    Established,
    /// Channel closed or failed.
    Closed,
}

/// An encrypted channel between two BUNNY nodes.
///
/// Manages the full lifecycle: handshake → messaging → rekeying → close.
/// Thread-safe via internal `Arc<RwLock<_>>` on the transport.
pub struct EncryptedChannel {
    /// Our node's transport layer (owns identity + session store).
    transport: Arc<NodeTransport>,
    /// Peer's public identity (set after handshake).
    peer_identity: RwLock<Option<NodeIdentity>>,
    /// Active session ID (set after handshake completes).
    session_id: RwLock<Option<SessionId>>,
    /// Current channel state.
    state: RwLock<ChannelState>,
    /// When the last rekey happened.
    last_rekey: RwLock<Instant>,
    /// Channel configuration.
    config: ChannelConfig,
    /// Ephemeral secret for handshake (only used during initiation).
    ephemeral_secret: RwLock<Option<x25519_dalek::EphemeralSecret>>,
}

impl EncryptedChannel {
    /// Create a new channel backed by an existing transport.
    pub fn new(transport: Arc<NodeTransport>, config: ChannelConfig) -> Self {
        Self {
            transport,
            peer_identity: RwLock::new(None),
            session_id: RwLock::new(None),
            state: RwLock::new(ChannelState::Idle),
            last_rekey: RwLock::new(Instant::now()),
            config,
            ephemeral_secret: RwLock::new(None),
        }
    }

    /// Current channel state.
    pub async fn state(&self) -> ChannelState {
        *self.state.read().await
    }

    /// Peer identity (available after handshake).
    pub async fn peer(&self) -> Option<NodeIdentity> {
        self.peer_identity.read().await.clone()
    }

    /// Active session ID (available after establishment).
    pub async fn session_id(&self) -> Option<SessionId> {
        self.session_id.read().await.clone()
    }

    // ── Handshake flow ─────────────────────────────────────────────────

    /// **Initiator step 1**: Start the handshake.
    ///
    /// Returns a `HandshakeInit` message to send to the peer.
    pub async fn initiate(&self) -> Result<HandshakeInit> {
        let (init, secret) = self.transport.initiate_handshake();
        *self.ephemeral_secret.write().await = Some(secret);
        *self.state.write().await = ChannelState::HandshakeSent;
        Ok(init)
    }

    /// **Responder step 2**: Accept an incoming handshake.
    ///
    /// Verifies the initiator's identity against our trust policy,
    /// performs hybrid key exchange, and returns the accept message.
    pub async fn accept(&self, init: &HandshakeInit) -> Result<HandshakeAccept> {
        let accept = self.transport.accept_handshake(init).await
            .map_err(|e| NetworkError::Protocol(format!("handshake accept failed: {e}")))?;

        // Store peer identity from the announcement.
        *self.peer_identity.write().await = Some(init.announcement.identity.clone());
        *self.session_id.write().await = Some(accept.session_id.clone());
        *self.state.write().await = ChannelState::Established;
        *self.last_rekey.write().await = Instant::now();

        Ok(accept)
    }

    /// **Initiator step 3**: Complete the handshake with peer's response.
    ///
    /// Performs our side of the hybrid key derivation and establishes
    /// the encrypted session.
    pub async fn complete(&self, accept: &HandshakeAccept) -> Result<()> {
        let secret: x25519_dalek::EphemeralSecret = self.ephemeral_secret.write().await.take()
            .ok_or_else(|| NetworkError::Protocol(
                "no ephemeral secret — was initiate() called?".into(),
            ))?;

        let sid = self.transport.complete_handshake(accept, secret).await
            .map_err(|e| NetworkError::Protocol(format!("handshake complete failed: {e}")))?;

        *self.peer_identity.write().await = Some(accept.announcement.identity.clone());
        *self.session_id.write().await = Some(sid);
        *self.state.write().await = ChannelState::Established;
        *self.last_rekey.write().await = Instant::now();

        Ok(())
    }

    // ── Messaging ──────────────────────────────────────────────────────

    /// Encrypt and seal a message for the peer.
    ///
    /// Uses cloaked envelopes if configured, otherwise standard envelopes.
    /// Automatically triggers rekeying if the interval has elapsed.
    pub async fn seal(&self, payload: &[u8]) -> Result<SealedMessage> {
        self.ensure_established().await?;
        self.maybe_rekey().await?;

        let sid: SessionId = self.session_id.read().await.clone()
            .ok_or_else(|| NetworkError::Protocol("no session".into()))?;

        if self.config.cloaked {
            let route_tag = OpaqueRouteTag::random();
            let envelope = self.transport.send_cloaked(&sid, route_tag, payload).await
                .map_err(|e| NetworkError::Protocol(format!("seal failed: {e}")))?;
            Ok(SealedMessage::Cloaked(envelope))
        } else {
            let envelope = self.transport.send(&sid, payload).await
                .map_err(|e| NetworkError::Protocol(format!("seal failed: {e}")))?;
            Ok(SealedMessage::Standard(envelope))
        }
    }

    /// Decrypt and open a received standard envelope.
    pub async fn open_standard(&self, envelope: &SwarmEnvelope) -> Result<Vec<u8>> {
        self.ensure_established().await?;
        self.transport.receive(envelope).await
            .map_err(|e| NetworkError::Protocol(format!("open failed: {e}")))
    }

    /// Decrypt and open a received cloaked envelope.
    pub async fn open_cloaked(&self, envelope: &CloakedEnvelope) -> Result<Vec<u8>> {
        self.ensure_established().await?;
        let payload = self.transport.receive_cloaked(envelope).await
            .map_err(|e| NetworkError::Protocol(format!("open failed: {e}")))?;
        Ok(payload.payload)
    }

    // ── Session management ─────────────────────────────────────────────

    /// Force a rekey (forward-secure key rotation).
    pub async fn rekey(&self) -> Result<()> {
        let sid: SessionId = self.session_id.read().await.clone()
            .ok_or_else(|| NetworkError::Protocol("no session".into()))?;

        self.transport.rekey(&sid).await
            .map_err(|e| NetworkError::Protocol(format!("rekey failed: {e}")))?;

        *self.last_rekey.write().await = Instant::now();
        Ok(())
    }

    /// Close the channel and destroy the session.
    pub async fn close(&self) {
        let sid_opt: Option<SessionId> = self.session_id.read().await.clone();
        if let Some(sid) = sid_opt {
            self.transport.session_store().remove(&sid).await;
        }
        *self.state.write().await = ChannelState::Closed;
        *self.session_id.write().await = None;
        *self.peer_identity.write().await = None;
    }

    // ── Internal helpers ───────────────────────────────────────────────

    async fn ensure_established(&self) -> Result<()> {
        let state = *self.state.read().await;
        if state != ChannelState::Established {
            return Err(NetworkError::Protocol(
                format!("channel not established (state: {state:?})")
            ));
        }
        Ok(())
    }

    async fn maybe_rekey(&self) -> Result<()> {
        let elapsed = self.last_rekey.read().await.elapsed();
        if elapsed >= self.config.rekey_interval {
            self.rekey().await?;
        }
        Ok(())
    }
}

/// A sealed (encrypted) message — either standard or cloaked.
#[derive(Debug)]
pub enum SealedMessage {
    /// Standard envelope with visible metadata.
    Standard(SwarmEnvelope),
    /// Cloaked envelope with hidden sender/timing.
    Cloaked(CloakedEnvelope),
}

impl SealedMessage {
    /// Serialize to wire bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            SealedMessage::Standard(e) => {
                let mut buf = vec![0u8]; // tag byte: 0 = standard
                buf.extend_from_slice(&e.to_bytes());
                buf
            }
            SealedMessage::Cloaked(e) => {
                let mut buf = vec![1u8]; // tag byte: 1 = cloaked
                buf.extend_from_slice(&e.to_bytes());
                buf
            }
        }
    }

    /// Deserialize from wire bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(NetworkError::Protocol("empty sealed message".into()));
        }
        match data[0] {
            0 => {
                let envelope = SwarmEnvelope::from_bytes(&data[1..])
                    .map_err(|e| NetworkError::Protocol(format!("bad standard envelope: {e}")))?;
                Ok(SealedMessage::Standard(envelope))
            }
            1 => {
                let envelope = CloakedEnvelope::from_bytes(&data[1..])
                    .map_err(|e| NetworkError::Protocol(format!("bad cloaked envelope: {e}")))?;
                Ok(SealedMessage::Cloaked(envelope))
            }
            tag => Err(NetworkError::Protocol(format!("unknown message tag: {tag}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::cipher::CipherSuite;

    fn make_transport(role: NodeRole) -> Arc<NodeTransport> {
        let caps = NodeCapabilities {
            models: vec![],
            has_gpu: false,
            max_sessions: 100,
            cipher_suites: vec![CipherSuite::Aes256Gcm],
        };
        let node = SwarmNode::generate(role, caps);
        Arc::new(NodeTransport::new(node))
    }

    #[tokio::test]
    async fn channel_starts_idle() {
        let transport = make_transport(NodeRole::Gateway);
        let ch = EncryptedChannel::new(transport, ChannelConfig::default());
        assert_eq!(ch.state().await, ChannelState::Idle);
        assert!(ch.peer().await.is_none());
    }

    #[tokio::test]
    async fn full_handshake_and_messaging() {
        // Portal (initiator) ↔ Coordinator (responder)
        let portal_t = make_transport(NodeRole::Gateway);
        let coord_t = make_transport(NodeRole::Orchestrator);

        let portal_ch = EncryptedChannel::new(portal_t, ChannelConfig::performance());
        let coord_ch = EncryptedChannel::new(coord_t, ChannelConfig::performance());

        // 1. Portal initiates
        let init = portal_ch.initiate().await.unwrap();
        assert_eq!(portal_ch.state().await, ChannelState::HandshakeSent);

        // 2. Coordinator accepts
        let accept = coord_ch.accept(&init).await.unwrap();
        assert_eq!(coord_ch.state().await, ChannelState::Established);

        // 3. Portal completes
        portal_ch.complete(&accept).await.unwrap();
        assert_eq!(portal_ch.state().await, ChannelState::Established);

        // 4. Portal sends encrypted message to coordinator
        let msg = b"POST /api/v1/slack/events HTTP/1.1\r\n\r\n{\"type\":\"message\"}";
        let sealed = portal_ch.seal(msg).await.unwrap();

        // 5. Coordinator decrypts
        match sealed {
            SealedMessage::Standard(ref env) => {
                let plaintext = coord_ch.open_standard(env).await.unwrap();
                assert_eq!(&plaintext, msg);
            }
            SealedMessage::Cloaked(ref env) => {
                let plaintext = coord_ch.open_cloaked(env).await.unwrap();
                assert_eq!(&plaintext, msg);
            }
        }
    }

    #[tokio::test]
    async fn cloaked_channel() {
        let portal_t = make_transport(NodeRole::Gateway);
        let coord_t = make_transport(NodeRole::Orchestrator);

        let portal_ch = EncryptedChannel::new(portal_t, ChannelConfig::high_security());
        let coord_ch = EncryptedChannel::new(coord_t, ChannelConfig::high_security());

        // Handshake
        let init = portal_ch.initiate().await.unwrap();
        let accept = coord_ch.accept(&init).await.unwrap();
        portal_ch.complete(&accept).await.unwrap();

        // Send cloaked message
        let msg = b"encrypted tool dispatch payload";
        let sealed = portal_ch.seal(msg).await.unwrap();

        // Must be cloaked since config says cloaked: true
        match sealed {
            SealedMessage::Cloaked(ref env) => {
                let plaintext = coord_ch.open_cloaked(env).await.unwrap();
                assert_eq!(&plaintext, &msg[..]);
            }
            _ => panic!("expected cloaked envelope"),
        }
    }

    #[tokio::test]
    async fn seal_before_handshake_fails() {
        let transport = make_transport(NodeRole::Gateway);
        let ch = EncryptedChannel::new(transport, ChannelConfig::default());
        let err = ch.seal(b"hello").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn close_resets_state() {
        let portal_t = make_transport(NodeRole::Gateway);
        let coord_t = make_transport(NodeRole::Orchestrator);

        let portal_ch = EncryptedChannel::new(portal_t, ChannelConfig::performance());
        let coord_ch = EncryptedChannel::new(coord_t, ChannelConfig::performance());

        let init = portal_ch.initiate().await.unwrap();
        let accept = coord_ch.accept(&init).await.unwrap();
        portal_ch.complete(&accept).await.unwrap();

        portal_ch.close().await;
        assert_eq!(portal_ch.state().await, ChannelState::Closed);
        assert!(portal_ch.peer().await.is_none());
        assert!(portal_ch.session_id().await.is_none());
    }

    #[test]
    fn sealed_message_roundtrip() {
        // Just test the tag-byte framing logic
        let data = vec![0u8, 1, 2, 3]; // too short for real envelope but tests tag
        let result = SealedMessage::from_bytes(&data);
        // Will fail because payload is too short, which is expected
        assert!(result.is_err());

        let result = SealedMessage::from_bytes(&[]);
        assert!(result.is_err());

        let result = SealedMessage::from_bytes(&[99]);
        assert!(result.is_err()); // unknown tag
    }
}
