pub mod client;
pub mod config;
pub mod connection;
pub mod error;
pub mod framing;
pub mod orchestration;
pub mod peer;
pub mod proxy;
pub mod server;
mod tls;

pub use client::{ConnectedPeer, SwarmClient};
pub use config::NetworkConfig;
pub use error::{NetworkError, Result};
pub use framing::{Frame, FrameType};
pub use orchestration::{SwarmCoordinator, WorkerPool};
pub use peer::{PeerInfo, PeerManager};
pub use proxy::{
    EncryptedChannel, NodeManifest, SidecarProxy, SignedRequest,
    ToolCall, ToolCallPriority, ToolResult, ToolDefinition, ToolRegistry,
    GpuCapabilityFlags, WorkerLabels, WorkerState,
    GpuWorkerRegistration, RuntimeKind, RuntimeHealthInfo,
    LoadedModel, WorkerHeartbeat,
    InferenceRequest, InferenceResponse,
    CloudProvider, CloudProviderConfig, CloudFallbackProvider,
    Codespace, CodespaceCapabilities, CodespaceState,
    CodespaceRegistration, CodespaceHeartbeat, CodespaceRegistry,
    CodespaceSnapshot,
    AgentDescriptor, ModelDescriptor, AgentManifest,
    AgentManifestBuilder, AGENT_MANIFEST_PATH,
};
pub use server::{InboundMessage, ServerHandle, SwarmServer};
