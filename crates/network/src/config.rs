use std::net::SocketAddr;
use std::time::Duration;

/// Configuration for a QUIC-based swarm node.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Address to bind the QUIC server to.
    pub bind_addr: SocketAddr,
    /// Maximum number of concurrent peer connections.
    pub max_peers: usize,
    /// Connection idle timeout.
    pub idle_timeout: Duration,
    /// Handshake timeout.
    pub handshake_timeout: Duration,
    /// Maximum frame payload size (bytes).
    pub max_frame_size: u32,
    /// Keep-alive interval for QUIC connections.
    pub keep_alive_interval: Duration,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:4433".parse().unwrap(),
            max_peers: 128,
            idle_timeout: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(10),
            max_frame_size: 16 * 1024 * 1024, // 16 MiB
            keep_alive_interval: Duration::from_secs(5),
        }
    }
}

impl NetworkConfig {
    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = addr;
        self
    }

    pub fn with_max_peers(mut self, max: usize) -> Self {
        self.max_peers = max;
        self
    }

    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }
}
