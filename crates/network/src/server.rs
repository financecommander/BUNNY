use std::sync::Arc;

use quinn::Endpoint;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use bunny_crypto::envelope::SwarmEnvelope;
use bunny_crypto::cloaked::CloakedEnvelope;
use bunny_crypto::transport::NodeTransport;

use crate::config::NetworkConfig;
use crate::connection;
use crate::error::{NetworkError, Result};
use crate::framing::{self, FrameType};
use crate::peer::{PeerInfo, PeerManager};
use crate::tls;

/// Received message from a peer, dispatched to the application layer.
#[derive(Debug)]
pub enum InboundMessage {
    /// A decrypted standard envelope payload.
    Envelope {
        peer: bunny_crypto::types::AgentId,
        plaintext: Vec<u8>,
    },
    /// A decrypted cloaked envelope payload.
    CloakedEnvelope {
        peer: bunny_crypto::types::AgentId,
        payload: bunny_crypto::cloaked::CloakedPayload,
    },
}

/// QUIC server that accepts incoming connections, performs handshakes,
/// and dispatches encrypted messages.
pub struct SwarmServer {
    endpoint: Endpoint,
    transport: Arc<NodeTransport>,
    peers: PeerManager,
    config: NetworkConfig,
}

impl SwarmServer {
    /// Create and bind a new QUIC server.
    pub fn new(
        transport: NodeTransport,
        config: NetworkConfig,
    ) -> Result<Self> {
        let server_config = tls::server_config(config.idle_timeout, config.keep_alive_interval)?;
        let endpoint = Endpoint::server(server_config, config.bind_addr)
            .map_err(|e| NetworkError::ServerError(format!("bind failed: {e}")))?;

        info!(addr = %config.bind_addr, "QUIC server listening");

        Ok(Self {
            endpoint,
            transport: Arc::new(transport),
            peers: PeerManager::new(config.max_peers),
            config,
        })
    }

    /// Run the accept loop, dispatching inbound messages to the returned channel.
    ///
    /// Spawns a task per connection. Returns a receiver for inbound messages
    /// and runs until the server is shut down.
    pub async fn run(self) -> (mpsc::Receiver<InboundMessage>, ServerHandle) {
        let (tx, rx) = mpsc::channel(256);
        let endpoint = self.endpoint.clone();

        let transport = self.transport;
        let peers = self.peers.clone();
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let conn = match incoming.await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("failed to accept connection: {e}");
                        continue;
                    }
                };

                let transport = transport.clone();
                let peers = peers.clone();
                let tx = tx.clone();
                let max_frame = config.max_frame_size;

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection(conn, &transport, &peers, &tx, max_frame).await
                    {
                        warn!("connection handler error: {e}");
                    }
                });
            }
        });

        (
            rx,
            ServerHandle {
                endpoint: self.endpoint,
                peers: self.peers,
                _task: handle,
            },
        )
    }

    /// Get a reference to the peer manager.
    pub fn peers(&self) -> &PeerManager {
        &self.peers
    }

    /// Get the local address this server is bound to.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.endpoint
            .local_addr()
            .map_err(|e| NetworkError::ServerError(format!("local addr: {e}")))
    }
}

/// Handle for controlling a running server.
pub struct ServerHandle {
    endpoint: Endpoint,
    peers: PeerManager,
    _task: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    /// Gracefully shut down the server.
    pub fn shutdown(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }

    /// Get the peer manager.
    pub fn peers(&self) -> &PeerManager {
        &self.peers
    }
}

/// Handle a single QUIC connection: perform handshake, then accept message streams.
async fn handle_connection(
    conn: quinn::Connection,
    transport: &NodeTransport,
    peers: &PeerManager,
    tx: &mpsc::Sender<InboundMessage>,
    max_frame_size: u32,
) -> Result<()> {
    // First bi-directional stream is the handshake
    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .map_err(|e| NetworkError::HandshakeFailed(format!("accept bi: {e}")))?;

    let (session_id, peer_role) =
        connection::accept_handshake(&mut send, &mut recv, transport, max_frame_size).await?;

    // Register peer with their AUTHENTICATED role (not our own)
    let peer_id = transport
        .session_store()
        .get_mut(&session_id, |s| s.peer.clone())
        .await
        .ok_or_else(|| NetworkError::HandshakeFailed("session lost after handshake".into()))?;

    let peer_info = PeerInfo::new(peer_id.clone(), peer_role, session_id, conn.clone());
    peers
        .add_peer(peer_info)
        .map_err(|e| NetworkError::Connection(e))?;

    info!(peer = ?peer_id, "peer connected");

    // Accept incoming uni-directional streams for messages
    loop {
        let recv_result = conn.accept_uni().await;
        let mut recv_stream = match recv_result {
            Ok(s) => s,
            Err(quinn::ConnectionError::ApplicationClosed(_)) => break,
            Err(quinn::ConnectionError::LocallyClosed) => break,
            Err(e) => {
                error!(peer = ?peer_id, "stream accept error: {e}");
                break;
            }
        };

        let frame = match framing::read_frame(&mut recv_stream, max_frame_size).await {
            Ok(f) => f,
            Err(e) => {
                warn!(peer = ?peer_id, "frame read error: {e}");
                continue;
            }
        };

        peers.touch(&peer_id);

        match frame.frame_type {
            FrameType::Envelope => {
                match SwarmEnvelope::from_bytes(&frame.payload) {
                    Ok(envelope) => match transport.receive(&envelope).await {
                        Ok(plaintext) => {
                            let _ = tx
                                .send(InboundMessage::Envelope {
                                    peer: peer_id.clone(),
                                    plaintext,
                                })
                                .await;
                        }
                        Err(e) => warn!(peer = ?peer_id, "decrypt error: {e}"),
                    },
                    Err(e) => warn!(peer = ?peer_id, "envelope parse error: {e}"),
                }
            }
            FrameType::CloakedEnvelope => {
                match CloakedEnvelope::from_bytes(&frame.payload) {
                    Ok(envelope) => match transport.receive_cloaked(&envelope).await {
                        Ok(payload) => {
                            let _ = tx
                                .send(InboundMessage::CloakedEnvelope {
                                    peer: peer_id.clone(),
                                    payload,
                                })
                                .await;
                        }
                        Err(e) => warn!(peer = ?peer_id, "cloaked decrypt error: {e}"),
                    },
                    Err(e) => warn!(peer = ?peer_id, "cloaked parse error: {e}"),
                }
            }
            other => {
                warn!(peer = ?peer_id, "unexpected frame type in message stream: {other:?}");
            }
        }
    }

    peers.remove_peer(&peer_id);
    info!(peer = ?peer_id, "peer disconnected");
    Ok(())
}
