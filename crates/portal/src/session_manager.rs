use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use bunny_crypto::types::AgentId;

/// External client session — maps API tokens to internal swarm identities.
///
/// Each client that connects through the portal gets a `ClientSession` that
/// tracks their requests and links to an internal `AgentId` for crypto sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    /// Portal-issued session token (UUID).
    pub token: uuid::Uuid,
    /// Internal agent identity for this client.
    pub agent_id: AgentId,
    /// When this session was created.
    pub created_at: u64,
    /// Last activity timestamp.
    pub last_activity: u64,
    /// Total requests processed.
    pub request_count: u64,
    /// Total inference time across all requests (microseconds).
    pub total_inference_us: u64,
}

impl ClientSession {
    pub fn new() -> Self {
        let now = now_ms();
        Self {
            token: uuid::Uuid::new_v4(),
            agent_id: AgentId::new(),
            created_at: now,
            last_activity: now,
            request_count: 0,
            total_inference_us: 0,
        }
    }

    /// Check if the session has been idle too long.
    pub fn is_expired(&self, max_idle_ms: u64) -> bool {
        now_ms().saturating_sub(self.last_activity) > max_idle_ms
    }

    /// Record a completed request.
    pub fn record_request(&mut self, inference_time_us: u64) {
        self.last_activity = now_ms();
        self.request_count += 1;
        self.total_inference_us += inference_time_us;
    }

    /// Average inference time per request (microseconds).
    pub fn avg_inference_us(&self) -> u64 {
        if self.request_count == 0 {
            0
        } else {
            self.total_inference_us / self.request_count
        }
    }
}

/// Manages external client sessions for the portal.
///
/// Thread-safe via DashMap. Handles session creation, lookup, expiry,
/// and cleanup. Each session maps an API token to an internal `AgentId`
/// used for PQ-encrypted swarm communication.
#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<DashMap<uuid::Uuid, ClientSession>>,
    max_idle_ms: u64,
}

impl SessionManager {
    /// Create a new session manager.
    ///
    /// `max_idle_ms`: sessions idle longer than this are expired (default 30 min).
    pub fn new(max_idle_ms: u64) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            max_idle_ms,
        }
    }

    /// Create a new client session, returning the token and agent ID.
    pub fn create_session(&self) -> (uuid::Uuid, AgentId) {
        let session = ClientSession::new();
        let token = session.token;
        let agent_id = session.agent_id.clone();
        info!(token = %token, "client session created");
        self.sessions.insert(token, session);
        (token, agent_id)
    }

    /// Look up a session by token, touching the last activity.
    pub fn get_session(&self, token: &uuid::Uuid) -> Option<ClientSession> {
        if let Some(mut entry) = self.sessions.get_mut(token) {
            if entry.is_expired(self.max_idle_ms) {
                drop(entry);
                self.sessions.remove(token);
                warn!(token = %token, "session expired on access");
                return None;
            }
            entry.last_activity = now_ms();
            Some(entry.clone())
        } else {
            None
        }
    }

    /// Get the internal AgentId for a session token.
    pub fn agent_id_for(&self, token: &uuid::Uuid) -> Option<AgentId> {
        self.get_session(token).map(|s| s.agent_id)
    }

    /// Record a completed request on a session.
    pub fn record_request(&self, token: &uuid::Uuid, inference_time_us: u64) {
        if let Some(mut entry) = self.sessions.get_mut(token) {
            entry.record_request(inference_time_us);
        }
    }

    /// Remove a session.
    pub fn remove_session(&self, token: &uuid::Uuid) -> Option<ClientSession> {
        self.sessions.remove(token).map(|(_, s)| s)
    }

    /// Evict all expired sessions. Returns count of evicted sessions.
    pub fn evict_expired(&self) -> usize {
        let before = self.sessions.len();
        self.sessions
            .retain(|_, session| !session.is_expired(self.max_idle_ms));
        let evicted = before - self.sessions.len();
        if evicted > 0 {
            debug!(count = evicted, "evicted expired sessions");
        }
        evicted
    }

    /// Active session count.
    pub fn active_count(&self) -> usize {
        self.sessions.len()
    }

    /// All active session tokens.
    pub fn active_tokens(&self) -> Vec<uuid::Uuid> {
        self.sessions.iter().map(|r| *r.key()).collect()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_lookup() {
        let mgr = SessionManager::new(1_800_000); // 30 min
        let (token, agent_id) = mgr.create_session();

        assert_eq!(mgr.active_count(), 1);

        let session = mgr.get_session(&token).unwrap();
        assert_eq!(session.agent_id, agent_id);
        assert_eq!(session.request_count, 0);
    }

    #[test]
    fn record_request() {
        let mgr = SessionManager::new(1_800_000);
        let (token, _) = mgr.create_session();

        mgr.record_request(&token, 5000);
        mgr.record_request(&token, 3000);

        let session = mgr.get_session(&token).unwrap();
        assert_eq!(session.request_count, 2);
        assert_eq!(session.avg_inference_us(), 4000);
    }

    #[test]
    fn expired_session_evicted_on_access() {
        let mgr = SessionManager::new(1); // 1ms timeout
        let (token, _) = mgr.create_session();

        // Force expiry by waiting.
        std::thread::sleep(std::time::Duration::from_millis(5));

        assert!(mgr.get_session(&token).is_none());
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn evict_expired() {
        let mgr = SessionManager::new(1); // 1ms timeout
        mgr.create_session();
        mgr.create_session();

        std::thread::sleep(std::time::Duration::from_millis(5));

        let evicted = mgr.evict_expired();
        assert_eq!(evicted, 2);
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn remove_session() {
        let mgr = SessionManager::new(1_800_000);
        let (token, _) = mgr.create_session();

        let removed = mgr.remove_session(&token);
        assert!(removed.is_some());
        assert_eq!(mgr.active_count(), 0);
    }
}
