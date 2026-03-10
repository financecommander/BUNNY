use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use bunny_crypto::identity::NodeRole;
use bunny_crypto::types::{AgentId, SessionId};

/// Information about a connected peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub agent_id: AgentId,
    pub role: NodeRole,
    pub session_id: SessionId,
    pub connected_at: u64,
    pub last_seen: u64,
    #[serde(skip)]
    pub connection: Option<quinn::Connection>,
}

impl PeerInfo {
    pub fn new(
        agent_id: AgentId,
        role: NodeRole,
        session_id: SessionId,
        connection: quinn::Connection,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        Self {
            agent_id,
            role,
            session_id,
            connected_at: now,
            last_seen: now,
            connection: Some(connection),
        }
    }

    pub fn touch(&mut self) {
        self.last_seen = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
    }
}

/// Thread-safe manager for tracking connected peers.
///
/// Uses `DashMap` for lock-free concurrent access from multiple
/// QUIC connection handlers.
#[derive(Clone)]
pub struct PeerManager {
    peers: Arc<DashMap<AgentId, PeerInfo>>,
    max_peers: usize,
}

impl PeerManager {
    pub fn new(max_peers: usize) -> Self {
        Self {
            peers: Arc::new(DashMap::new()),
            max_peers,
        }
    }

    /// Register a new peer. Returns error if at capacity.
    pub fn add_peer(&self, peer: PeerInfo) -> Result<(), String> {
        if self.peers.len() >= self.max_peers {
            return Err(format!(
                "peer limit reached ({}/{})",
                self.peers.len(),
                self.max_peers
            ));
        }
        self.peers.insert(peer.agent_id.clone(), peer);
        Ok(())
    }

    /// Remove a peer by agent ID.
    pub fn remove_peer(&self, agent_id: &AgentId) -> Option<PeerInfo> {
        self.peers.remove(agent_id).map(|(_, p)| p)
    }

    /// Get a peer's session ID.
    pub fn session_for(&self, agent_id: &AgentId) -> Option<SessionId> {
        self.peers.get(agent_id).map(|p| p.session_id.clone())
    }

    /// Get the QUIC connection for a peer.
    pub fn connection_for(&self, agent_id: &AgentId) -> Option<quinn::Connection> {
        self.peers
            .get(agent_id)
            .and_then(|p| p.connection.clone())
    }

    /// Update last-seen time for a peer.
    pub fn touch(&self, agent_id: &AgentId) {
        if let Some(mut peer) = self.peers.get_mut(agent_id) {
            peer.touch();
        }
    }

    /// Number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Check if a peer is connected.
    pub fn has_peer(&self, agent_id: &AgentId) -> bool {
        self.peers.contains_key(agent_id)
    }

    /// Remove peers that haven't been seen within `timeout_ms`.
    pub fn evict_stale(&self, timeout_ms: u64) -> Vec<AgentId> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut evicted = Vec::new();

        self.peers.retain(|_, peer| {
            let alive = now - peer.last_seen < timeout_ms;
            if !alive {
                evicted.push(peer.agent_id.clone());
            }
            alive
        });

        evicted
    }

    /// Get all peer agent IDs.
    pub fn all_peer_ids(&self) -> Vec<AgentId> {
        self.peers.iter().map(|r| r.key().clone()).collect()
    }

    /// Get all peers with a specific role.
    pub fn peers_by_role(&self, role: NodeRole) -> Vec<AgentId> {
        self.peers
            .iter()
            .filter(|r| r.role == role)
            .map(|r| r.key().clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_peer(role: NodeRole) -> PeerInfo {
        PeerInfo {
            agent_id: AgentId::new(),
            role,
            session_id: SessionId::new(),
            connected_at: 0,
            last_seen: 0,
            connection: None,
        }
    }

    #[test]
    fn add_and_remove_peers() {
        let mgr = PeerManager::new(10);
        let peer = mock_peer(NodeRole::Worker);
        let id = peer.agent_id.clone();

        mgr.add_peer(peer).unwrap();
        assert_eq!(mgr.peer_count(), 1);
        assert!(mgr.has_peer(&id));

        mgr.remove_peer(&id);
        assert_eq!(mgr.peer_count(), 0);
        assert!(!mgr.has_peer(&id));
    }

    #[test]
    fn max_peers_enforced() {
        let mgr = PeerManager::new(2);
        mgr.add_peer(mock_peer(NodeRole::Worker)).unwrap();
        mgr.add_peer(mock_peer(NodeRole::Worker)).unwrap();
        assert!(mgr.add_peer(mock_peer(NodeRole::Worker)).is_err());
    }

    #[test]
    fn peers_by_role() {
        let mgr = PeerManager::new(10);
        mgr.add_peer(mock_peer(NodeRole::Worker)).unwrap();
        mgr.add_peer(mock_peer(NodeRole::Worker)).unwrap();
        mgr.add_peer(mock_peer(NodeRole::Gateway)).unwrap();

        assert_eq!(mgr.peers_by_role(NodeRole::Worker).len(), 2);
        assert_eq!(mgr.peers_by_role(NodeRole::Gateway).len(), 1);
        assert_eq!(mgr.peers_by_role(NodeRole::Orchestrator).len(), 0);
    }

    #[test]
    fn session_lookup() {
        let mgr = PeerManager::new(10);
        let peer = mock_peer(NodeRole::Worker);
        let id = peer.agent_id.clone();
        let session = peer.session_id.clone();
        mgr.add_peer(peer).unwrap();

        assert_eq!(mgr.session_for(&id), Some(session));
        assert_eq!(mgr.session_for(&AgentId::new()), None);
    }
}
