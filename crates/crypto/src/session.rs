use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cipher::SwarmCipher;
use crate::cloaked::{CloakedEnvelope, CloakedPayload, OpaqueRouteTag};
use crate::envelope::SwarmEnvelope;
use crate::error::{CryptoError, Result};
use crate::types::{AgentId, SessionId};

/// Sliding window size for replay protection (in sequence numbers).
/// Must be ≤ 128 since we use a u128 bitmap.
const REPLAY_WINDOW_SIZE: u64 = 128;

/// An established encrypted session between two agents.
pub struct SwarmSession {
    pub session_id: SessionId,
    pub peer: AgentId,
    pub cipher: SwarmCipher,
    pub created_at: u64,
    pub last_activity: u64,
    pub send_sequence: u64,
    /// High-water mark: the highest sequence number received so far.
    pub recv_sequence: u64,
    /// Bitmap tracking which of the last REPLAY_WINDOW_SIZE sequences
    /// before recv_sequence have been seen. Bit 0 = recv_sequence itself,
    /// bit 1 = recv_sequence - 1, etc.
    pub(crate) recv_bitmap: u128,
    /// Whether we have received at least one message.
    pub(crate) recv_initialized: bool,
}

impl SwarmSession {
    /// Encrypt a payload for this session's peer.
    pub fn seal(&mut self, sender: &AgentId, payload: &[u8]) -> Result<SwarmEnvelope> {
        let seq = self.send_sequence;
        self.send_sequence += 1;
        self.update_activity();
        SwarmEnvelope::seal(&self.cipher, sender, &self.session_id, seq, payload)
    }

    /// Encrypt a payload into a cloaked envelope (metadata-minimized).
    pub fn seal_cloaked(
        &mut self,
        sender: &AgentId,
        route_tag: OpaqueRouteTag,
        payload: &[u8],
    ) -> Result<CloakedEnvelope> {
        let seq = self.send_sequence;
        self.send_sequence += 1;
        self.update_activity();
        CloakedEnvelope::seal(&self.cipher, sender, &self.session_id, seq, route_tag, payload)
    }

    /// Sliding-window replay check. Returns Ok(()) if the sequence is fresh,
    /// or Err if it is a replay or too old.
    fn check_and_record_sequence(&mut self, seq: u64) -> Result<()> {
        if !self.recv_initialized {
            // First message ever — accept it, initialize the window.
            self.recv_sequence = seq;
            self.recv_bitmap = 1; // bit 0 marks this sequence as seen
            self.recv_initialized = true;
            return Ok(());
        }

        if seq > self.recv_sequence {
            // Advance the window: shift bitmap by the gap
            let gap = seq - self.recv_sequence;
            if gap >= REPLAY_WINDOW_SIZE {
                // Everything in the old window is too old; reset bitmap.
                self.recv_bitmap = 1;
            } else {
                self.recv_bitmap <<= gap;
                self.recv_bitmap |= 1; // mark new high-water as seen
            }
            self.recv_sequence = seq;
            Ok(())
        } else {
            // seq <= recv_sequence — check if within the window
            let age = self.recv_sequence - seq;
            if age >= REPLAY_WINDOW_SIZE {
                return Err(CryptoError::InvalidEnvelope(
                    "replay detected: sequence too old for window".into(),
                ));
            }
            let bit = 1u128 << age;
            if self.recv_bitmap & bit != 0 {
                return Err(CryptoError::InvalidEnvelope("replay detected".into()));
            }
            // Mark as seen
            self.recv_bitmap |= bit;
            Ok(())
        }
    }

    /// Decrypt a cloaked envelope from this session's peer.
    pub fn open_cloaked(&mut self, envelope: &CloakedEnvelope) -> Result<CloakedPayload> {
        // Sliding-window replay protection
        self.check_and_record_sequence(envelope.header.sequence)?;
        let payload = envelope.open(&self.cipher)?;
        self.update_activity();
        Ok(payload)
    }

    /// Decrypt an envelope from this session's peer.
    pub fn open(&mut self, envelope: &SwarmEnvelope) -> Result<Vec<u8>> {
        // Sliding-window replay protection
        self.check_and_record_sequence(envelope.sequence)?;
        let plaintext = envelope.open(&self.cipher)?;
        self.update_activity();
        Ok(plaintext)
    }

    fn update_activity(&mut self) {
        self.last_activity = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }

    /// Check if session has expired.
    pub fn is_expired(&self, max_age_ms: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        now - self.created_at > max_age_ms
    }
}

/// Thread-safe session store for managing multiple concurrent sessions.
#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, SwarmSession>>>,
    pub max_session_age_ms: u64,
}

impl SessionStore {
    pub fn new(max_session_age_ms: u64) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            max_session_age_ms,
        }
    }

    pub async fn insert(&self, session: SwarmSession) {
        let mut map = self.sessions.write().await;
        map.insert(session.session_id.clone(), session);
    }

    pub async fn get_mut<F, R>(&self, id: &SessionId, f: F) -> Option<R>
    where
        F: FnOnce(&mut SwarmSession) -> R,
    {
        let mut map = self.sessions.write().await;
        map.get_mut(id).map(f)
    }

    pub async fn remove(&self, id: &SessionId) {
        let mut map = self.sessions.write().await;
        map.remove(id);
    }

    pub async fn contains(&self, id: &SessionId) -> bool {
        let map = self.sessions.read().await;
        map.contains_key(id)
    }

    /// Remove all expired sessions.
    pub async fn prune_expired(&self) {
        let mut map = self.sessions.write().await;
        let max_age = self.max_session_age_ms;
        map.retain(|_, session| !session.is_expired(max_age));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::{CipherSuite, SwarmCipher};
    use crate::types::SymmetricKey;

    fn test_session() -> SwarmSession {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        SwarmSession {
            session_id: SessionId::new(),
            peer: AgentId::new(),
            cipher: SwarmCipher::with_suite(
                &SymmetricKey::from_bytes([0x42; 32]),
                CipherSuite::Aes256Gcm,
            ),
            created_at: now,
            last_activity: now,
            send_sequence: 0,
            recv_sequence: 0,
            recv_bitmap: 0,
            recv_initialized: false,
        }
    }

    #[test]
    fn seal_increments_sequence() {
        let mut session = test_session();
        let sender = AgentId::new();

        let env1 = session.seal(&sender, b"msg1").unwrap();
        let env2 = session.seal(&sender, b"msg2").unwrap();

        assert_eq!(env1.sequence, 0);
        assert_eq!(env2.sequence, 1);
        assert_eq!(session.send_sequence, 2);
    }

    #[test]
    fn replay_detection() {
        let key = SymmetricKey::from_bytes([0x42; 32]);
        let suite = CipherSuite::Aes256Gcm;
        let session_id = SessionId::new();
        let sender = AgentId::new();

        // Create two sessions with same key (simulating sender/receiver)
        let cipher = SwarmCipher::with_suite(&key, suite);
        let envelope = SwarmEnvelope::seal(&cipher, &sender, &session_id, 5, b"msg").unwrap();

        let mut receiver = SwarmSession {
            session_id: session_id.clone(),
            peer: sender.clone(),
            cipher: SwarmCipher::with_suite(&key, suite),
            created_at: 0,
            last_activity: 0,
            send_sequence: 0,
            recv_sequence: 5, // already seen up to sequence 5
            recv_bitmap: 0b111111, // sequences 0-5 seen
            recv_initialized: true,
        };

        // Should reject replay
        assert!(receiver.open(&envelope).is_err());
    }

    #[test]
    fn sliding_window_allows_out_of_order() {
        let key = SymmetricKey::from_bytes([0x42; 32]);
        let suite = CipherSuite::Aes256Gcm;
        let session_id = SessionId::new();
        let sender = AgentId::new();

        let mut session = SwarmSession {
            session_id: session_id.clone(),
            peer: sender.clone(),
            cipher: SwarmCipher::with_suite(&key, suite),
            created_at: 0,
            last_activity: 0,
            send_sequence: 0,
            recv_sequence: 0,
            recv_bitmap: 0,
            recv_initialized: false,
        };

        // Receive seq 5 first
        let cipher = SwarmCipher::with_suite(&key, suite);
        let env5 = SwarmEnvelope::seal(&cipher, &sender, &session_id, 5, b"msg5").unwrap();
        session.open(&env5).unwrap();

        // Receive seq 3 (out of order, within window) — should succeed
        let env3 = SwarmEnvelope::seal(&cipher, &sender, &session_id, 3, b"msg3").unwrap();
        session.open(&env3).unwrap();

        // Replay seq 3 again — should fail
        let env3_dup = SwarmEnvelope::seal(&cipher, &sender, &session_id, 3, b"msg3").unwrap();
        assert!(session.open(&env3_dup).is_err());

        // Seq 0 is within the window (5-0 = 5 < 128) — should succeed
        let env0 = SwarmEnvelope::seal(&cipher, &sender, &session_id, 0, b"msg0").unwrap();
        session.open(&env0).unwrap();

        // Replay seq 0 — should fail
        let env0_dup = SwarmEnvelope::seal(&cipher, &sender, &session_id, 0, b"msg0").unwrap();
        assert!(session.open(&env0_dup).is_err());
    }

    #[tokio::test]
    async fn session_store_lifecycle() {
        let store = SessionStore::new(3_600_000); // 1 hour

        let session = test_session();
        let id = session.session_id.clone();

        assert!(!store.contains(&id).await);

        store.insert(session).await;
        assert!(store.contains(&id).await);

        store.remove(&id).await;
        assert!(!store.contains(&id).await);
    }
}
