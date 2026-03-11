//! Encrypted proxy layer for BUNNY node-to-node transport.
//!
//! Provides a TCP-level proxy that intercepts plaintext HTTP traffic between
//! swarm nodes and wraps it in ML-KEM + AES-256-GCM encrypted channels.
//!
//! Architecture: each node runs a BUNNY sidecar that listens on localhost.
//! Application code (Python/FastAPI) connects to localhost instead of direct
//! IPs. The sidecar handles key exchange, encryption, and forwarding.
//!
//! ```text
//! App (Python) → localhost:port → BUNNY proxy (encrypt)
//!     → ML-KEM channel → BUNNY proxy on target (decrypt) → target service
//! ```

pub mod node_identity;
pub mod channel;
pub mod sidecar;
pub mod envelope_sign;
pub mod tool_call;
pub mod tool_registry;
pub mod dispatch;
pub mod gpu_worker;
pub mod inference_dispatch;
pub mod openclaw_bridge;
pub mod cloud_fallback;
pub mod codespace_registry;
pub mod agent_manifest;

pub use node_identity::{NodeManifest, NodeBinding, SwarmRole};
pub use channel::{EncryptedChannel, ChannelConfig};
pub use sidecar::{SidecarProxy, ProxyRoute};
pub use envelope_sign::{SignedRequest, RequestSigner, RequestVerifier};

pub use tool_call::{ToolCall, ToolCallPriority, ToolResult};
pub use tool_registry::{ToolDefinition, ToolRegistry};
pub use dispatch::{
    seal_tool_call, seal_tool_result, open_tool_call, open_tool_result,
    TOOL_CALL_PATH, TOOL_RESULT_PATH,
};
pub use gpu_worker::{
    GpuCapabilityFlags, WorkerLabels, WorkerState,
    GpuWorkerRegistration, RuntimeKind, RuntimeHealthInfo,
    LoadedModel, WorkerHeartbeat,
};
pub use inference_dispatch::{
    InferenceRequest, InferenceResponse,
    seal_inference_request, open_inference_request,
    seal_inference_response, open_inference_response,
    seal_heartbeat, open_heartbeat, establish_gpu_channel,
    INFERENCE_DISPATCH_PATH, INFERENCE_RESPONSE_PATH, HEARTBEAT_PATH,
};
pub use openclaw_bridge::{
    ToolRoute, classify_tool_route, classify_tool_route_with_fallback,
    tool_call_to_inference_request, inference_response_to_tool_result,
    dispatch_tool_to_gpu, receive_gpu_response, dispatch_with_fallback,
    INFERENCE_TOOL, GPU_ROUTABLE_TOOLS,
};
pub use cloud_fallback::{
    CloudProvider, CloudProviderConfig, CloudFallbackProvider,
    build_fallback_from_env, cloud_response_to_tool_result,
};
pub use codespace_registry::{
    Codespace, CodespaceCapabilities, CodespaceState,
    CodespaceRegistration, CodespaceHeartbeat, CodespaceRegistry,
    CodespaceSnapshot,
    create_spork_manifest, create_orchestra_manifest,
    create_probflow_manifest, create_portal_manifest,
    create_calculus_manifest,
};
pub use agent_manifest::{
    AgentDescriptor, ModelDescriptor, AgentManifest,
    AgentManifestBuilder, AGENT_MANIFEST_PATH,
};
