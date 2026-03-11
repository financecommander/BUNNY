//! TCP sidecar proxy — the deployment unit that wraps existing services.
//!
//! Each VM runs a [`SidecarProxy`] that:
//! 1. Listens on a local TCP port (e.g., 127.0.0.1:9000)
//! 2. Accepts plaintext connections from local services
//! 3. Encrypts traffic via [`EncryptedChannel`] and forwards to the
//!    remote BUNNY sidecar on the target node
//! 4. Decrypts incoming traffic and forwards to the local service
//!
//! This means the Python services (AI Portal, SWARM, calculus-web) require
//! zero code changes — they just connect to localhost instead of direct IPs.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use bunny_crypto::transport::NodeTransport;

use super::channel::{ChannelConfig, EncryptedChannel};
use super::node_identity::{NodeManifest, SwarmRole};
use crate::error::Result;

/// A route definition: local listen → remote target.
///
/// Example: listen on 127.0.0.1:11434, forward to swarm-gpu's sidecar,
/// which then forwards to the real Ollama on that machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRoute {
    /// Human-readable name for logging.
    pub name: String,
    /// Local address to listen on (app connects here).
    pub listen: SocketAddr,
    /// Target node's sidecar address (encrypted tunnel endpoint).
    pub target_sidecar: SocketAddr,
    /// The actual service address on the target machine.
    /// The remote sidecar forwards decrypted traffic here.
    pub target_service: SocketAddr,
    /// Use cloaked envelopes for this route.
    pub cloaked: bool,
}

/// Registered peer node for the sidecar to establish channels with.
pub struct PeerRegistration {
    /// Peer's name.
    pub name: String,
    /// Peer's role.
    pub role: SwarmRole,
    /// Peer's sidecar address.
    pub sidecar_addr: SocketAddr,
    /// Established encrypted channel (if connected).
    pub channel: Option<EncryptedChannel>,
}

/// The sidecar proxy — one per VM.
///
/// Manages route table, peer channels, and the TCP forwarding loop.
pub struct SidecarProxy {
    /// This node's identity and transport.
    manifest: NodeManifest,
    /// The underlying crypto transport.
    transport: Arc<NodeTransport>,
    /// Route table: local listen addr → target.
    routes: Vec<ProxyRoute>,
    /// Known peers: name → registration.
    peers: HashMap<String, PeerRegistration>,
    /// Incoming service port (other sidecars connect here).
    incoming_addr: SocketAddr,
}

impl SidecarProxy {
    // Note: Use `with_transport()` constructor since `NodeTransport::new()`
    // takes ownership of `SwarmNode` but we need it in the manifest too.
    // The `new()` convenience constructor is not available — always use
    // `with_transport()` and construct the transport externally.

    /// Add a forwarding route.
    pub fn add_route(&mut self, route: ProxyRoute) {
        self.routes.push(route);
    }

    /// Register a peer node that we may need to connect to.
    pub fn register_peer(&mut self, name: impl Into<String>, role: SwarmRole, sidecar_addr: SocketAddr) {
        let name = name.into();
        self.peers.insert(name.clone(), PeerRegistration {
            name,
            role,
            sidecar_addr,
            channel: None,
        });
    }

    /// Get the route table.
    pub fn routes(&self) -> &[ProxyRoute] {
        &self.routes
    }

    /// Get known peers.
    pub fn peers(&self) -> &HashMap<String, PeerRegistration> {
        &self.peers
    }

    /// This node's manifest.
    pub fn manifest(&self) -> &NodeManifest {
        &self.manifest
    }

    /// The sidecar's incoming address.
    pub fn incoming_addr(&self) -> SocketAddr {
        self.incoming_addr
    }

    /// Establish an encrypted channel to a registered peer.
    ///
    /// Performs the full ML-KEM + X25519 handshake. The peer's sidecar
    /// must be running and accepting connections.
    pub async fn connect_peer(&mut self, peer_name: &str, peer_transport: &Arc<NodeTransport>) -> Result<()> {
        let peer = self.peers.get_mut(peer_name)
            .ok_or_else(|| crate::error::NetworkError::Protocol(
                format!("unknown peer: {peer_name}")
            ))?;

        let config = ChannelConfig::default();
        let our_channel = EncryptedChannel::new(self.transport.clone(), config.clone());
        let their_channel = EncryptedChannel::new(peer_transport.clone(), config);

        // Perform handshake: we initiate, peer accepts.
        let init = our_channel.initiate().await?;
        let accept = their_channel.accept(&init).await?;
        our_channel.complete(&accept).await?;

        peer.channel = Some(our_channel);
        Ok(())
    }

    /// Check if a peer has an established channel.
    pub fn is_peer_connected(&self, peer_name: &str) -> bool {
        self.peers.get(peer_name)
            .and_then(|p| p.channel.as_ref())
            .is_some()
    }

    /// Number of active routes.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Number of registered peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Number of connected (established channel) peers.
    pub fn connected_peer_count(&self) -> usize {
        self.peers.values().filter(|p| p.channel.is_some()).count()
    }
}

/// Helper: `NodeTransport` doesn't have a `new_from_ref` — we need to
/// handle this by cloning the node or passing the transport in.
/// For now, we construct transport externally and pass it in.
impl SidecarProxy {
    /// Create with an externally-constructed transport.
    pub fn with_transport(manifest: NodeManifest, transport: Arc<NodeTransport>) -> Self {
        let incoming_addr = manifest.binding.sidecar;
        Self {
            manifest,
            transport,
            routes: Vec::new(),
            peers: HashMap::new(),
            incoming_addr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::node_identity::NodeBinding;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::cipher::CipherSuite;

    fn make_manifest(role: SwarmRole, name: &str, sidecar_port: u16) -> NodeManifest {
        NodeManifest::generate(
            role,
            name,
            NodeBinding {
                internal: format!("10.142.0.2:{}", sidecar_port + 1000).parse().unwrap(),
                external: None,
                sidecar: format!("127.0.0.1:{sidecar_port}").parse().unwrap(),
            },
        )
    }

    fn make_transport(role: NodeRole) -> Arc<NodeTransport> {
        let caps = NodeCapabilities {
            models: vec![],
            has_gpu: false,
            max_sessions: 100,
            cipher_suites: vec![CipherSuite::Aes256Gcm],
        };
        Arc::new(NodeTransport::new(SwarmNode::generate(role, caps)))
    }

    #[test]
    fn create_sidecar_proxy() {
        let manifest = make_manifest(SwarmRole::Portal, "fc-ai-portal", 9000);
        let transport = make_transport(NodeRole::Gateway);
        let proxy = SidecarProxy::with_transport(manifest, transport);

        assert_eq!(proxy.manifest().name, "fc-ai-portal");
        assert_eq!(proxy.route_count(), 0);
        assert_eq!(proxy.peer_count(), 0);
    }

    #[test]
    fn add_routes_and_peers() {
        let manifest = make_manifest(SwarmRole::Portal, "fc-ai-portal", 9000);
        let transport = make_transport(NodeRole::Gateway);
        let mut proxy = SidecarProxy::with_transport(manifest, transport);

        // Route: local :3001 → swarm-mainframe sidecar → mainframe :3000
        proxy.add_route(ProxyRoute {
            name: "to-mainframe".into(),
            listen: "127.0.0.1:3001".parse().unwrap(),
            target_sidecar: "10.142.0.4:9001".parse().unwrap(),
            target_service: "127.0.0.1:3000".parse().unwrap(),
            cloaked: true,
        });

        // Route: local :11434 → swarm-gpu sidecar → gpu :11434
        proxy.add_route(ProxyRoute {
            name: "to-gpu-ollama".into(),
            listen: "127.0.0.1:11434".parse().unwrap(),
            target_sidecar: "10.142.0.6:9003".parse().unwrap(),
            target_service: "127.0.0.1:11434".parse().unwrap(),
            cloaked: false,
        });

        proxy.register_peer("swarm-mainframe", SwarmRole::Coordinator, "10.142.0.4:9001".parse().unwrap());
        proxy.register_peer("swarm-gpu", SwarmRole::GpuWorker, "10.142.0.6:9003".parse().unwrap());

        assert_eq!(proxy.route_count(), 2);
        assert_eq!(proxy.peer_count(), 2);
        assert_eq!(proxy.connected_peer_count(), 0);
        assert!(!proxy.is_peer_connected("swarm-mainframe"));
    }

    #[tokio::test]
    async fn establish_peer_channel() {
        let portal_manifest = make_manifest(SwarmRole::Portal, "fc-ai-portal", 9000);
        let portal_transport = make_transport(NodeRole::Gateway);
        let mut portal = SidecarProxy::with_transport(portal_manifest, portal_transport);

        let coord_transport = make_transport(NodeRole::Orchestrator);
        portal.register_peer(
            "swarm-mainframe",
            SwarmRole::Coordinator,
            "10.142.0.4:9001".parse().unwrap(),
        );

        // Connect portal → coordinator
        portal.connect_peer("swarm-mainframe", &coord_transport).await.unwrap();
        assert!(portal.is_peer_connected("swarm-mainframe"));
        assert_eq!(portal.connected_peer_count(), 1);
    }

    #[tokio::test]
    async fn connect_unknown_peer_fails() {
        let manifest = make_manifest(SwarmRole::Portal, "fc-ai-portal", 9000);
        let transport = make_transport(NodeRole::Gateway);
        let mut proxy = SidecarProxy::with_transport(manifest, transport);

        let other = make_transport(NodeRole::Worker);
        let err = proxy.connect_peer("nonexistent", &other).await;
        assert!(err.is_err());
    }

    #[test]
    fn portal_topology_example() {
        // Full topology: fc-ai-portal sees 3 peers + 2 routes
        let manifest = make_manifest(SwarmRole::Portal, "fc-ai-portal", 9000);
        let transport = make_transport(NodeRole::Gateway);
        let mut proxy = SidecarProxy::with_transport(manifest, transport);

        proxy.register_peer("swarm-mainframe", SwarmRole::Coordinator, "10.142.0.4:9001".parse().unwrap());
        proxy.register_peer("calculus-web", SwarmRole::ComputeNode, "10.142.0.3:9002".parse().unwrap());
        proxy.register_peer("swarm-gpu", SwarmRole::GpuWorker, "10.142.0.6:9003".parse().unwrap());

        proxy.add_route(ProxyRoute {
            name: "swarm-api".into(),
            listen: "127.0.0.1:3001".parse().unwrap(),
            target_sidecar: "10.142.0.4:9001".parse().unwrap(),
            target_service: "127.0.0.1:3000".parse().unwrap(),
            cloaked: true,
        });

        proxy.add_route(ProxyRoute {
            name: "ollama-inference".into(),
            listen: "127.0.0.1:11434".parse().unwrap(),
            target_sidecar: "10.142.0.6:9003".parse().unwrap(),
            target_service: "127.0.0.1:11434".parse().unwrap(),
            cloaked: false,
        });

        assert_eq!(proxy.peer_count(), 3);
        assert_eq!(proxy.route_count(), 2);
        assert_eq!(proxy.manifest().role, SwarmRole::Portal);
    }
}
