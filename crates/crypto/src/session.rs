use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cipher::SwarmCipher;
use crate::cloaked::{CloakedEnvelope, CloakedPayload, OpaqueRouteTag};
use crate::envelope::SwarmEnvelope;
use crate::error::{CryptoError, Result};
use crate::types::{AgentId, SessionId};

/// An established encrypted session between two agents.
pub struct SwarmSession {
    pub session_id: SessionId,
    pub peer: AgentId,
    pub cipher: SwarmCipher,
    pub created_at: u64,
    pub last_activity: u64,
    pub send_sequence: u64,
    pub recv_sequence: u64,
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

    /// Decrypt a cloaked envelope from this session's peer.
    pub fn open_cloaked(&mut self, envelope: &CloakedEnvelope) -> Result<CloakedPayload> {
        // Replay protection
        if envelope.header.sequence <= self.recv_sequence && self.recv_sequence > 0 {
            return Err(CryptoError::InvalidEnvelope("replay detected".into()));
        }
        let payload = envelope.open(&self.cipher)?;
        self.recv_sequence = envelope.header.sequence;
        self.update_activity();
        Ok(payload)
    }

    /// Decrypt an envelope from this session's peer.
    pub fn open(&mut self, envelope: &SwarmEnvelope) -> Result<Vec<u8>> {
        // Replay protection
        if envelope.sequence <= self.recv_sequence && self.recv_sequence > 0 {
            return Err(CryptoError::InvalidEnvelope("replay detected".into()));
        }
        let plaintext = envelope.open(&self.cipher)?;
        self.recv_sequence = envelope.sequence;
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
            recv_sequence: 5, // already seen sequence 5
        };

        // Should reject replay
        assert!(receiver.open(&envelope).is_err());
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
