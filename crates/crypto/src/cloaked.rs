use serde::{Deserialize, Serialize};

use crate::error::{CryptoError, Result};

/// Size buckets for packet normalization.
///
/// All packets are padded to the nearest bucket boundary before encryption
/// so that observers cannot distinguish payload types by size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeBucket {
    /// Control messages, heartbeats, rekey requests (≤256 bytes).
    ControlSmall = 256,
    /// Inference requests/responses, standard payloads (≤4096 bytes).
    InferenceStandard = 4096,
    /// Model shards, weight transfers, large tensors (≤65536 bytes).
    ArtifactLarge = 65536,
    /// Streaming windows, KV-cache snapshots (≤262144 bytes).
    StreamWindow = 262144,
}

impl SizeBucket {
    /// Select the smallest bucket that fits the given payload size.
    pub fn for_size(payload_len: usize) -> Result<Self> {
        if payload_len <= Self::ControlSmall as usize {
            Ok(Self::ControlSmall)
        } else if payload_len <= Self::InferenceStandard as usize {
            Ok(Self::InferenceStandard)
        } else if payload_len <= Self::ArtifactLarge as usize {
            Ok(Self::ArtifactLarge)
        } else if payload_len <= Self::StreamWindow as usize {
            Ok(Self::StreamWindow)
        } else {
            Err(CryptoError::InvalidEnvelope(format!(
                "payload too large for any bucket: {} bytes (max {})",
                payload_len,
                Self::StreamWindow as usize
            )))
        }
    }

    pub fn size(&self) -> usize {
        *self as usize
    }

    /// All bucket sizes in ascending order.
    pub fn all() -> &'static [usize] {
        &[256, 4096, 65536, 262144]
    }
}

/// Pad a payload to the target bucket size.
///
/// Format: `[real_len: 4 bytes BE] [payload] [random padding]`
///
/// The 4-byte length prefix is inside the encrypted envelope, invisible to observers.
pub fn pad_to_bucket(payload: &[u8]) -> Result<Vec<u8>> {
    let bucket = SizeBucket::for_size(payload.len() + 4)?; // +4 for length prefix
    let target = bucket.size();

    let mut buf = Vec::with_capacity(target);
    // Length prefix (inside encryption, not visible)
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);

    // Random padding to fill bucket
    let pad_len = target - buf.len();
    if pad_len > 0 {
        let mut padding = vec![0u8; pad_len];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut padding);
        buf.extend_from_slice(&padding);
    }

    Ok(buf)
}

/// Remove padding after decryption, recovering the original payload.
pub fn unpad(padded: &[u8]) -> Result<Vec<u8>> {
    if padded.len() < 4 {
        return Err(CryptoError::InvalidEnvelope(
            "padded payload too short for length prefix".into(),
        ));
    }

    let real_len = u32::from_be_bytes([padded[0], padded[1], padded[2], padded[3]]) as usize;

    if real_len + 4 > padded.len() {
        return Err(CryptoError::InvalidEnvelope(format!(
            "declared length {} exceeds padded buffer {}",
            real_len,
            padded.len() - 4
        )));
    }

    Ok(padded[4..4 + real_len].to_vec())
}

/// Traffic shaping policy — configurable per deployment.
///
/// Controls timing, coalescing, and cover traffic behavior.
/// Default is `disabled` (zero overhead, maximum throughput).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficPolicy {
    /// Enable dispatch jitter (randomized delay before sending).
    pub jitter_enabled: bool,
    /// Jitter range in milliseconds [min, max].
    pub jitter_range_ms: (u64, u64),

    /// Enable packet coalescing (batch small packets within a window).
    pub coalescing_enabled: bool,
    /// Coalescing window in milliseconds.
    pub coalescing_window_ms: u64,

    /// Enable burst smoothing (rate-limit outgoing packets).
    pub burst_smoothing_enabled: bool,
    /// Maximum packets per second when smoothing is active.
    pub max_packets_per_second: u32,

    /// Enable cover traffic (send dummy packets to mask real traffic patterns).
    pub cover_traffic_enabled: bool,
    /// Cover traffic rate: average dummy packets per second.
    pub cover_traffic_rate: f64,
}

impl TrafficPolicy {
    /// High-throughput mode: all shaping disabled, zero overhead.
    pub fn disabled() -> Self {
        Self {
            jitter_enabled: false,
            jitter_range_ms: (0, 0),
            coalescing_enabled: false,
            coalescing_window_ms: 0,
            burst_smoothing_enabled: false,
            max_packets_per_second: 0,
            cover_traffic_enabled: false,
            cover_traffic_rate: 0.0,
        }
    }

    /// Cloaked mode: all shaping features active.
    pub fn cloaked() -> Self {
        Self {
            jitter_enabled: true,
            jitter_range_ms: (5, 50),
            coalescing_enabled: true,
            coalescing_window_ms: 20,
            burst_smoothing_enabled: true,
            max_packets_per_second: 500,
            cover_traffic_enabled: true,
            cover_traffic_rate: 10.0,
        }
    }

    /// Balanced mode: jitter + smoothing, no cover traffic.
    pub fn balanced() -> Self {
        Self {
            jitter_enabled: true,
            jitter_range_ms: (1, 10),
            coalescing_enabled: false,
            coalescing_window_ms: 0,
            burst_smoothing_enabled: true,
            max_packets_per_second: 2000,
            cover_traffic_enabled: false,
            cover_traffic_rate: 0.0,
        }
    }

    /// Compute jitter delay for this policy (returns 0 if disabled).
    pub fn jitter_delay_ms(&self) -> u64 {
        if !self.jitter_enabled || self.jitter_range_ms.1 == 0 {
            return 0;
        }
        let (min, max) = self.jitter_range_ms;
        if min >= max {
            return min;
        }
        use rand::Rng;
        rand::rngs::OsRng.gen_range(min..=max)
    }

    /// Check if a packet should be sent now or delayed (burst smoothing).
    /// Returns true if the packet should be sent, false if it should wait.
    pub fn should_send(&self, packets_this_second: u32) -> bool {
        if !self.burst_smoothing_enabled || self.max_packets_per_second == 0 {
            return true;
        }
        packets_this_second < self.max_packets_per_second
    }
}

impl Default for TrafficPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Generate a cover traffic packet (random data, padded to ControlSmall bucket).
///
/// Cover packets are indistinguishable from real encrypted control messages
/// after encryption. They carry no real payload — the receiver discards them
/// after decryption by checking a magic byte.
///
/// Format: `[0x00 magic] [random fill]`
pub fn generate_cover_packet() -> Vec<u8> {
    let inner_len = SizeBucket::ControlSmall.size() - 4; // minus length prefix
    let mut data = vec![0u8; inner_len];
    // Magic byte 0x00 at position 0 marks this as cover traffic
    // Real payloads always start with a non-zero type byte
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut data[1..]);
    pad_to_bucket(&data).unwrap()
}

/// Check if a decrypted payload is cover traffic (should be discarded).
pub fn is_cover_traffic(unpadded: &[u8]) -> bool {
    !unpadded.is_empty() && unpadded[0] == 0x00
}

/// Opaque route tag for metadata-minimized transport headers.
///
/// This replaces human-readable routing info in the plaintext header.
/// The real route is encrypted inside the envelope payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OpaqueRouteTag(pub [u8; 16]);

impl OpaqueRouteTag {
    /// Generate a random opaque route tag.
    pub fn random() -> Self {
        let mut bytes = [0u8; 16];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Derive a deterministic tag from a model hash + session context.
    /// This allows the execution boundary to look up the real route
    /// without exposing model identity in the header.
    pub fn derive(model_hash: &[u8; 32], session_salt: &[u8; 16]) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(model_hash);
        hasher.update(session_salt);
        let hash = hasher.finalize();
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&hash[..16]);
        Self(tag)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Minimal plaintext header for cloaked transport.
///
/// Only these fields are visible before decryption:
/// ```text
/// ┌────────────┬──────────┬───────┬─────────────────┐
/// │ session_id │ sequence │ nonce │ opaque_route_tag │
/// │ 16 bytes   │ 8 bytes  │ 12 B  │ 16 bytes         │
/// └────────────┴──────────┴───────┴─────────────────┘
/// ```
///
/// Everything else (payload type, model hash, shard info, sender identity,
/// cipher suite, protocol version) is encrypted inside the ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloakedHeader {
    pub session_id: crate::types::SessionId,
    pub sequence: u64,
    pub nonce: crate::types::Nonce,
    pub route_tag: OpaqueRouteTag,
}

/// Total plaintext header size: 16 + 8 + 12 + 16 = 52 bytes
pub const CLOAKED_HEADER_SIZE: usize = 52;

impl CloakedHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(CLOAKED_HEADER_SIZE);
        buf.extend_from_slice(self.session_id.as_bytes());
        buf.extend_from_slice(&self.sequence.to_be_bytes());
        buf.extend_from_slice(&self.nonce);
        buf.extend_from_slice(self.route_tag.as_bytes());
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < CLOAKED_HEADER_SIZE {
            return Err(CryptoError::InvalidEnvelope(format!(
                "cloaked header too short: {} bytes, need {}",
                data.len(),
                CLOAKED_HEADER_SIZE
            )));
        }

        let mut session_bytes = [0u8; 16];
        session_bytes.copy_from_slice(&data[0..16]);

        let sequence = u64::from_be_bytes(data[16..24].try_into().unwrap());

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&data[24..36]);

        let mut route_tag = [0u8; 16];
        route_tag.copy_from_slice(&data[36..52]);

        Ok(Self {
            session_id: crate::types::SessionId(session_bytes),
            sequence,
            nonce,
            route_tag: OpaqueRouteTag(route_tag),
        })
    }
}

/// A cloaked envelope: minimal plaintext header + normalized encrypted payload.
///
/// Wire format:
/// ```text
/// ┌───────────────────────────┬─────────────────────────────────┐
/// │ CloakedHeader (52 bytes)  │ ciphertext (bucket-normalized)  │
/// └───────────────────────────┴─────────────────────────────────┘
/// ```
///
/// The ciphertext contains (after decryption):
/// - Full inner metadata (version, suite, sender, payload type, etc.)
/// - The real payload data
/// - Random padding to bucket boundary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloakedEnvelope {
    pub header: CloakedHeader,
    pub ciphertext: Vec<u8>,
}

impl CloakedEnvelope {
    /// Seal a payload into a cloaked envelope.
    ///
    /// 1. Builds inner metadata (version, suite, sender)
    /// 2. Concatenates inner metadata + payload
    /// 3. Pads to bucket boundary
    /// 4. Encrypts with AEAD (AAD = cloaked header bytes)
    /// 5. Wraps in CloakedHeader + ciphertext
    pub fn seal(
        cipher: &crate::cipher::SwarmCipher,
        sender: &crate::types::AgentId,
        session_id: &crate::types::SessionId,
        sequence: u64,
        route_tag: OpaqueRouteTag,
        payload: &[u8],
    ) -> Result<Self> {
        // Build inner metadata (encrypted, not visible)
        let mut inner = Vec::with_capacity(19 + payload.len());
        inner.push(crate::types::PROTOCOL_VERSION); // 1 byte
        inner.push(cipher.suite().as_u8()); // 1 byte
        inner.extend_from_slice(sender.as_bytes()); // 16 bytes
        // Timestamp
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        inner.extend_from_slice(&ts.to_be_bytes()); // 8 bytes
        inner.extend_from_slice(payload);

        // Pad to bucket boundary
        let padded = pad_to_bucket(&inner)?;

        // Generate nonce
        let nonce = crate::cipher::SwarmCipher::generate_nonce();

        // Build AAD from the cloaked header fields
        let header = CloakedHeader {
            session_id: session_id.clone(),
            sequence,
            nonce,
            route_tag,
        };
        let aad = header.to_bytes();

        // Encrypt
        let ciphertext = cipher.encrypt(&nonce, &padded, &aad)?;

        Ok(Self { header, ciphertext })
    }

    /// Open a cloaked envelope, returning the inner metadata and payload.
    ///
    /// Only call this at the execution boundary.
    pub fn open(
        &self,
        cipher: &crate::cipher::SwarmCipher,
    ) -> Result<CloakedPayload> {
        let aad = self.header.to_bytes();
        let padded = cipher.decrypt(&self.header.nonce, &self.ciphertext, &aad)?;

        // Unpad
        let inner = unpad(&padded)?;

        // Parse inner metadata
        if inner.len() < 26 {
            return Err(CryptoError::InvalidEnvelope(
                "inner payload too short".into(),
            ));
        }

        let version = inner[0];
        if version != crate::types::PROTOCOL_VERSION {
            return Err(CryptoError::ProtocolVersionMismatch {
                expected: crate::types::PROTOCOL_VERSION,
                actual: version,
            });
        }

        let suite = crate::cipher::CipherSuite::from_u8(inner[1])?;

        let mut sender_bytes = [0u8; 16];
        sender_bytes.copy_from_slice(&inner[2..18]);
        let sender = crate::types::AgentId(uuid::Uuid::from_bytes(sender_bytes));

        let timestamp_ms = u64::from_be_bytes(inner[18..26].try_into().unwrap());

        let payload = inner[26..].to_vec();

        Ok(CloakedPayload {
            version,
            suite,
            sender,
            timestamp_ms,
            payload,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let header_bytes = self.header.to_bytes();
        let mut buf = Vec::with_capacity(header_bytes.len() + self.ciphertext.len());
        buf.extend_from_slice(&header_bytes);
        buf.extend_from_slice(&self.ciphertext);
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < CLOAKED_HEADER_SIZE {
            return Err(CryptoError::InvalidEnvelope(
                "cloaked envelope too short".into(),
            ));
        }
        let header = CloakedHeader::from_bytes(&data[..CLOAKED_HEADER_SIZE])?;
        let ciphertext = data[CLOAKED_HEADER_SIZE..].to_vec();
        Ok(Self { header, ciphertext })
    }
}

/// Decrypted inner payload from a cloaked envelope.
/// Only visible at the execution boundary after decryption.
#[derive(Debug)]
pub struct CloakedPayload {
    pub version: u8,
    pub suite: crate::cipher::CipherSuite,
    pub sender: crate::types::AgentId,
    pub timestamp_ms: u64,
    pub payload: Vec<u8>,
}

/// Compartmentalization level — determines what metadata a role can access.
///
/// Enforces minimum-knowledge principle: each role sees only what it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compartment {
    /// Full access: sees sender, payload, route, all metadata.
    /// Roles: Gateway, Orchestrator at execution boundary.
    Full,
    /// Route-only: sees opaque route tag + encrypted payload (cannot decrypt).
    /// Roles: Relay — forwards packets without inspecting content.
    RouteOnly,
    /// Payload-only: sees decrypted payload but not full routing context.
    /// Roles: Worker — receives work, doesn't know the full orchestration graph.
    PayloadOnly,
}

impl Compartment {
    /// Determine the compartment for a given node role.
    pub fn for_role(role: crate::identity::NodeRole) -> Self {
        use crate::identity::NodeRole;
        match role {
            NodeRole::Gateway | NodeRole::Orchestrator => Self::Full,
            NodeRole::Relay => Self::RouteOnly,
            NodeRole::Worker | NodeRole::Agent => Self::PayloadOnly,
        }
    }

    /// Check if this compartment allows access to the sender identity.
    pub fn can_see_sender(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Check if this compartment allows access to the decrypted payload.
    pub fn can_see_payload(&self) -> bool {
        matches!(self, Self::Full | Self::PayloadOnly)
    }

    /// Check if this compartment allows access to route resolution.
    pub fn can_resolve_routes(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Check if this compartment allows access to timing metadata.
    pub fn can_see_timing(&self) -> bool {
        matches!(self, Self::Full)
    }
}

/// A compartmentalized view of a cloaked payload.
///
/// Restricts access to metadata based on the recipient's role.
/// This prevents information leakage across compartment boundaries.
#[derive(Debug)]
pub struct CompartmentalizedView {
    compartment: Compartment,
    inner: CloakedPayload,
}

impl CompartmentalizedView {
    /// Create a compartmentalized view of a cloaked payload.
    pub fn new(payload: CloakedPayload, role: crate::identity::NodeRole) -> Self {
        Self {
            compartment: Compartment::for_role(role),
            inner: payload,
        }
    }

    /// Get the compartment level.
    pub fn compartment(&self) -> Compartment {
        self.compartment
    }

    /// Get the sender identity (Full compartment only).
    pub fn sender(&self) -> Option<&crate::types::AgentId> {
        if self.compartment.can_see_sender() {
            Some(&self.inner.sender)
        } else {
            None
        }
    }

    /// Get the decrypted payload (Full and PayloadOnly compartments).
    pub fn payload(&self) -> Option<&[u8]> {
        if self.compartment.can_see_payload() {
            Some(&self.inner.payload)
        } else {
            None
        }
    }

    /// Get the timestamp (Full compartment only).
    pub fn timestamp_ms(&self) -> Option<u64> {
        if self.compartment.can_see_timing() {
            Some(self.inner.timestamp_ms)
        } else {
            None
        }
    }

    /// Get the protocol version (always visible).
    pub fn version(&self) -> u8 {
        self.inner.version
    }

    /// Get the cipher suite (always visible — needed for forwarding).
    pub fn suite(&self) -> crate::cipher::CipherSuite {
        self.inner.suite
    }

    /// Consume the view and extract the full payload (bypasses compartment).
    /// Only use at trust boundaries where the role has been verified.
    pub fn into_inner(self) -> CloakedPayload {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::{CipherSuite, SwarmCipher};
    use crate::types::{AgentId, SessionId, SymmetricKey};

    fn test_cipher() -> SwarmCipher {
        SwarmCipher::with_suite(&SymmetricKey::from_bytes([0x42; 32]), CipherSuite::Aes256Gcm)
    }

    // --- Bucket selection ---

    #[test]
    fn bucket_selection_control() {
        assert_eq!(SizeBucket::for_size(10).unwrap(), SizeBucket::ControlSmall);
        assert_eq!(SizeBucket::for_size(100).unwrap(), SizeBucket::ControlSmall);
        assert_eq!(SizeBucket::for_size(252).unwrap(), SizeBucket::ControlSmall);
    }

    #[test]
    fn bucket_selection_inference() {
        assert_eq!(
            SizeBucket::for_size(257).unwrap(),
            SizeBucket::InferenceStandard
        );
        assert_eq!(
            SizeBucket::for_size(4096).unwrap(),
            SizeBucket::InferenceStandard
        );
    }

    #[test]
    fn bucket_selection_artifact() {
        assert_eq!(
            SizeBucket::for_size(4097).unwrap(),
            SizeBucket::ArtifactLarge
        );
        assert_eq!(
            SizeBucket::for_size(65536).unwrap(),
            SizeBucket::ArtifactLarge
        );
    }

    #[test]
    fn bucket_selection_stream() {
        assert_eq!(
            SizeBucket::for_size(65537).unwrap(),
            SizeBucket::StreamWindow
        );
    }

    #[test]
    fn bucket_overflow_rejected() {
        assert!(SizeBucket::for_size(262145).is_err());
    }

    // --- Padding roundtrip ---

    #[test]
    fn pad_unpad_roundtrip_small() {
        let payload = b"heartbeat";
        let padded = pad_to_bucket(payload).unwrap();
        assert_eq!(padded.len(), SizeBucket::ControlSmall.size());
        let recovered = unpad(&padded).unwrap();
        assert_eq!(recovered, payload);
    }

    #[test]
    fn pad_unpad_roundtrip_medium() {
        let payload = vec![0xAA; 1000];
        let padded = pad_to_bucket(&payload).unwrap();
        assert_eq!(padded.len(), SizeBucket::InferenceStandard.size());
        let recovered = unpad(&padded).unwrap();
        assert_eq!(recovered, payload);
    }

    #[test]
    fn pad_unpad_roundtrip_large() {
        let payload = vec![0xBB; 50000];
        let padded = pad_to_bucket(&payload).unwrap();
        assert_eq!(padded.len(), SizeBucket::ArtifactLarge.size());
        let recovered = unpad(&padded).unwrap();
        assert_eq!(recovered, payload);
    }

    #[test]
    fn different_payloads_same_bucket_same_size() {
        let p1 = pad_to_bucket(b"short").unwrap();
        let p2 = pad_to_bucket(b"a slightly longer message").unwrap();
        // Both fit in ControlSmall
        assert_eq!(p1.len(), p2.len());
        assert_eq!(p1.len(), SizeBucket::ControlSmall.size());
    }

    // --- Cover traffic ---

    #[test]
    fn cover_packet_is_bucket_sized() {
        let cover = generate_cover_packet();
        assert_eq!(cover.len(), SizeBucket::ControlSmall.size());
    }

    #[test]
    fn cover_traffic_detected() {
        let cover = generate_cover_packet();
        let unpadded = unpad(&cover).unwrap();
        assert!(is_cover_traffic(&unpadded));
    }

    #[test]
    fn real_traffic_not_flagged_as_cover() {
        let payload = b"\x01real inference data";
        let padded = pad_to_bucket(payload).unwrap();
        let unpadded = unpad(&padded).unwrap();
        assert!(!is_cover_traffic(&unpadded));
    }

    // --- Opaque route tag ---

    #[test]
    fn derived_route_tag_deterministic() {
        let model_hash = [0xAA; 32];
        let salt = [0xBB; 16];
        let t1 = OpaqueRouteTag::derive(&model_hash, &salt);
        let t2 = OpaqueRouteTag::derive(&model_hash, &salt);
        assert_eq!(t1, t2);
    }

    #[test]
    fn different_models_different_tags() {
        let salt = [0xCC; 16];
        let t1 = OpaqueRouteTag::derive(&[0xAA; 32], &salt);
        let t2 = OpaqueRouteTag::derive(&[0xBB; 32], &salt);
        assert_ne!(t1, t2);
    }

    // --- Cloaked header ---

    #[test]
    fn cloaked_header_roundtrip() {
        let header = CloakedHeader {
            session_id: SessionId::new(),
            sequence: 42,
            nonce: SwarmCipher::generate_nonce(),
            route_tag: OpaqueRouteTag::random(),
        };
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), CLOAKED_HEADER_SIZE);

        let restored = CloakedHeader::from_bytes(&bytes).unwrap();
        assert_eq!(restored.session_id, header.session_id);
        assert_eq!(restored.sequence, 42);
        assert_eq!(restored.nonce, header.nonce);
        assert_eq!(restored.route_tag, header.route_tag);
    }

    // --- Cloaked envelope ---

    #[test]
    fn cloaked_envelope_seal_open() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();
        let tag = OpaqueRouteTag::random();

        let payload = b"inference request: classify traffic";
        let envelope =
            CloakedEnvelope::seal(&cipher, &sender, &session, 1, tag, payload).unwrap();

        // Only session_id, sequence, nonce, route_tag visible in header
        assert_eq!(envelope.header.session_id, session);
        assert_eq!(envelope.header.sequence, 1);
        assert_eq!(envelope.header.route_tag, tag);

        // Ciphertext is bucket-normalized (not payload size)
        // The padded inner is at least 256 bytes, + AEAD tag overhead
        assert!(envelope.ciphertext.len() >= SizeBucket::ControlSmall.size());

        // Open and verify
        let opened = envelope.open(&cipher).unwrap();
        assert_eq!(opened.payload, payload);
        assert_eq!(opened.sender, sender);
        assert_eq!(opened.version, crate::types::PROTOCOL_VERSION);
    }

    #[test]
    fn cloaked_envelope_binary_roundtrip() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();
        let tag = OpaqueRouteTag::random();

        let envelope =
            CloakedEnvelope::seal(&cipher, &sender, &session, 7, tag, b"test").unwrap();
        let bytes = envelope.to_bytes();
        let restored = CloakedEnvelope::from_bytes(&bytes).unwrap();

        let opened = restored.open(&cipher).unwrap();
        assert_eq!(opened.payload, b"test");
    }

    #[test]
    fn cloaked_envelope_hides_metadata() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();
        let tag = OpaqueRouteTag::random();

        let envelope =
            CloakedEnvelope::seal(&cipher, &sender, &session, 1, tag, b"secret").unwrap();
        let bytes = envelope.to_bytes();

        // Plaintext header: only session_id(16) + sequence(8) + nonce(12) + route_tag(16) = 52
        // The sender ID should NOT appear in the plaintext header
        let header_bytes = &bytes[..CLOAKED_HEADER_SIZE];
        let sender_bytes = sender.as_bytes();

        // Sender is NOT in the plaintext header
        assert!(!header_bytes
            .windows(16)
            .any(|w| w == sender_bytes));
    }

    #[test]
    fn cloaked_envelope_tampered_header_fails() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();
        let tag = OpaqueRouteTag::random();

        let mut envelope =
            CloakedEnvelope::seal(&cipher, &sender, &session, 1, tag, b"data").unwrap();

        // Tamper with sequence in header (part of AAD)
        envelope.header.sequence = 999;
        assert!(envelope.open(&cipher).is_err());
    }

    // --- Traffic policy ---

    #[test]
    fn disabled_policy_zero_jitter() {
        let policy = TrafficPolicy::disabled();
        assert_eq!(policy.jitter_delay_ms(), 0);
        assert!(policy.should_send(9999)); // always send
    }

    #[test]
    fn cloaked_policy_has_jitter() {
        let policy = TrafficPolicy::cloaked();
        let delay = policy.jitter_delay_ms();
        assert!(delay >= 5 && delay <= 50);
    }

    #[test]
    fn burst_smoothing_limits_rate() {
        let policy = TrafficPolicy::cloaked();
        assert!(policy.should_send(0));
        assert!(policy.should_send(499));
        assert!(!policy.should_send(500));
        assert!(!policy.should_send(1000));
    }

    #[test]
    fn balanced_policy_moderate() {
        let policy = TrafficPolicy::balanced();
        assert!(policy.jitter_enabled);
        assert!(!policy.cover_traffic_enabled);
        assert!(policy.burst_smoothing_enabled);
    }

    // --- Compartmentalization ---

    #[test]
    fn gateway_has_full_compartment() {
        use crate::identity::NodeRole;

        let compartment = Compartment::for_role(NodeRole::Gateway);
        assert_eq!(compartment, Compartment::Full);
        assert!(compartment.can_see_sender());
        assert!(compartment.can_see_payload());
        assert!(compartment.can_resolve_routes());
        assert!(compartment.can_see_timing());
    }

    #[test]
    fn relay_has_route_only_compartment() {
        use crate::identity::NodeRole;

        let compartment = Compartment::for_role(NodeRole::Relay);
        assert_eq!(compartment, Compartment::RouteOnly);
        assert!(!compartment.can_see_sender());
        assert!(!compartment.can_see_payload());
        assert!(!compartment.can_resolve_routes());
        assert!(!compartment.can_see_timing());
    }

    #[test]
    fn worker_has_payload_only_compartment() {
        use crate::identity::NodeRole;

        let compartment = Compartment::for_role(NodeRole::Worker);
        assert_eq!(compartment, Compartment::PayloadOnly);
        assert!(!compartment.can_see_sender());
        assert!(compartment.can_see_payload());
        assert!(!compartment.can_resolve_routes());
        assert!(!compartment.can_see_timing());
    }

    #[test]
    fn compartmentalized_view_restricts_access() {
        use crate::identity::NodeRole;

        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();
        let tag = OpaqueRouteTag::random();

        let envelope =
            CloakedEnvelope::seal(&cipher, &sender, &session, 1, tag, b"secret data").unwrap();
        let payload = envelope.open(&cipher).unwrap();

        // Worker can see payload but not sender or timing
        let worker_view = CompartmentalizedView::new(payload, NodeRole::Worker);
        assert!(worker_view.payload().is_some());
        assert_eq!(worker_view.payload().unwrap(), b"secret data");
        assert!(worker_view.sender().is_none());
        assert!(worker_view.timestamp_ms().is_none());

        // Re-do for gateway (Full access)
        let payload2 = envelope.open(&cipher).unwrap();
        let gw_view = CompartmentalizedView::new(payload2, NodeRole::Gateway);
        assert!(gw_view.payload().is_some());
        assert!(gw_view.sender().is_some());
        assert_eq!(gw_view.sender().unwrap(), &sender);
        assert!(gw_view.timestamp_ms().is_some());

        // Re-do for relay (no access to payload or sender)
        let payload3 = envelope.open(&cipher).unwrap();
        let relay_view = CompartmentalizedView::new(payload3, NodeRole::Relay);
        assert!(relay_view.payload().is_none());
        assert!(relay_view.sender().is_none());
        assert!(relay_view.timestamp_ms().is_none());
    }
}
