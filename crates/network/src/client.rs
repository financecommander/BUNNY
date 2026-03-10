use std::net::SocketAddr;
use std::sync::Arc;

use quinn::Endpoint;
use tracing::info;

use bunny_crypto::transport::NodeTransport;
use bunny_crypto::types::SessionId;

use crate::config::NetworkConfig;
use crate::connection;
use crate::error::{NetworkError, Result};
use crate::tls;

/// QUIC client that connects to a swarm server, performs handshake,
/// and provides methods to send encrypted messages.
pub struct SwarmClient {
    endpoint: Endpoint,
    transport: Arc<NodeTransport>,
    config: NetworkConfig,
}

impl SwarmClient {
    /// Create a new QUIC client (binds to an ephemeral port).
    pub fn new(transport: NodeTransport, config: NetworkConfig) -> Result<Self> {
        let client_config = tls::client_config()?;

        let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())
            .map_err(|e| NetworkError::Connection(format!("client bind: {e}")))?;
        endpoint.set_default_client_config(client_config);

        Ok(Self {
            endpoint,
            transport: Arc::new(transport),
            config,
        })
    }

    /// Connect to a server, perform handshake, return a connected session.
    pub async fn connect(&self, server_addr: SocketAddr) -> Result<ConnectedPeer> {
        let conn = self
            .endpoint
            .connect(server_addr, "bunny")
            .map_err(|e| NetworkError::Connection(format!("connect: {e}")))?
            .await
            .map_err(|e| NetworkError::Connection(format!("connection failed: {e}")))?;

        let session_id = connection::initiate_handshake(
            &conn,
            &self.transport,
            self.config.max_frame_size,
        )
        .await?;

        info!(addr = %server_addr, session = ?session_id, "connected to server");

        Ok(ConnectedPeer {
            connection: conn,
            transport: self.transport.clone(),
            session_id,
        })
    }

    /// Close the client endpoint.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"client shutdown");
    }
}

/// An established connection to a remote peer.
pub struct ConnectedPeer {
    pub connection: quinn::Connection,
    transport: Arc<NodeTransport>,
    pub session_id: SessionId,
}

impl ConnectedPeer {
    /// Send an encrypted payload to the peer.
    pub async fn send(&self, payload: &[u8]) -> Result<()> {
        connection::send_envelope(
            &self.connection,
            &self.transport,
            &self.session_id,
            payload,
        )
        .await
    }

    /// Send a cloaked encrypted payload to the peer.
    pub async fn send_cloaked(
        &self,
        route_tag: bunny_crypto::OpaqueRouteTag,
        payload: &[u8],
    ) -> Result<()> {
        connection::send_cloaked_envelope(
            &self.connection,
            &self.transport,
            &self.session_id,
            route_tag,
            payload,
        )
        .await
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Close the connection.
    pub fn close(&self) {
        self.connection.close(0u32.into(), b"done");
    }
}
