//! Inference dispatch — signed encrypted inference request/response envelopes.
//!
//! Implements Workstreams 2 and 4 of the Phase 4 directive:
//! - WS2: Encrypted channel mainframe ↔ GPU (handshake helpers + send/recv)
//! - WS4: Inference dispatch envelope (signed encrypted inference request/response)
//!
//! Flow:
//! ```text
//! Coordinator (mainframe)
//!   → InferenceRequest → sign → seal → EncryptedChannel → GPU worker
//!   ← InferenceResponse ← verify ← open ← EncryptedChannel ← GPU worker
//!
//! Heartbeat:
//!   GPU worker → WorkerHeartbeat → sign → seal → EncryptedChannel → Coordinator
//! ```

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use bunny_crypto::identity::NodeIdentity;
use bunny_crypto::types::AgentId;

use crate::error::{NetworkError, Result};
use crate::proxy::channel::{EncryptedChannel, SealedMessage};
use crate::proxy::dispatch::ProxyEnvelope;
use crate::proxy::envelope_sign::{RequestSigner, RequestVerifier};
use crate::proxy::gpu_worker::WorkerHeartbeat;

/// Well-known path for inference dispatch requests.
pub const INFERENCE_DISPATCH_PATH: &str = "/bunny/inference/dispatch";
/// Well-known path for inference dispatch responses.
pub const INFERENCE_RESPONSE_PATH: &str = "/bunny/inference/response";
/// Well-known path for worker heartbeat messages.
pub const HEARTBEAT_PATH: &str = "/bunny/worker/heartbeat";

/// An inference dispatch request — sent from coordinator to GPU worker.
///
/// Maps to the flow: Slack/OpenClaw → Coordinator → GPU worker dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    /// Unique request ID for correlation.
    pub request_id: String,
    /// Model to use for inference (e.g., "llama3.1:8b").
    pub model: String,
    /// The prompt or input text.
    pub prompt: String,
    /// Caller identity (which agent initiated this).
    pub caller: AgentId,
    /// Temperature for sampling (0.0 - 2.0).
    pub temperature: f32,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Extra parameters (system prompt, stop sequences, etc.).
    pub params: HashMap<String, String>,
    /// Unix timestamp when request was created.
    pub created_at_ms: u64,
}

impl InferenceRequest {
    /// Create a new inference request.
    pub fn new(
        model: impl Into<String>,
        prompt: impl Into<String>,
        caller: AgentId,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            request_id: format!("infer-{}", uuid::Uuid::new_v4()),
            model: model.into(),
            prompt: prompt.into(),
            caller,
            temperature: 0.7,
            max_tokens: 2048,
            params: HashMap::new(),
            created_at_ms: now,
        }
    }

    /// Serialize to bytes for signing/transport.
    pub fn into_payload(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| NetworkError::Serialization(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_payload(payload: &[u8]) -> Result<Self> {
        serde_json::from_slice(payload)
            .map_err(|e| NetworkError::Serialization(e.to_string()))
    }

    /// Sign this request for dispatch to GPU worker.
    pub fn sign_for_dispatch(
        &self,
        signer: &mut RequestSigner,
    ) -> Result<crate::proxy::envelope_sign::SignedRequest> {
        Ok(signer.sign(INFERENCE_DISPATCH_PATH.to_string(), self.into_payload()?))
    }

    /// Add a parameter.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = temp;
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, tokens: u32) -> Self {
        self.max_tokens = tokens;
        self
    }
}

/// An inference dispatch response — sent from GPU worker back to coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Matches the originating InferenceRequest.
    pub request_id: String,
    /// Model that was used.
    pub model: String,
    /// Whether inference succeeded.
    pub success: bool,
    /// The generated text output.
    pub output: String,
    /// Error message if failed.
    pub error: Option<String>,
    /// Number of tokens generated.
    pub tokens_generated: u32,
    /// Inference time in milliseconds.
    pub inference_time_ms: u64,
    /// Worker node name that executed the request.
    pub worker_node: String,
    /// Extra metadata (token counts, stop reason, etc.).
    pub metadata: HashMap<String, String>,
}

impl InferenceResponse {
    /// Create a successful inference response.
    pub fn success(
        request_id: impl Into<String>,
        model: impl Into<String>,
        output: impl Into<String>,
        tokens: u32,
        time_ms: u64,
        worker: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            model: model.into(),
            success: true,
            output: output.into(),
            error: None,
            tokens_generated: tokens,
            inference_time_ms: time_ms,
            worker_node: worker.into(),
            metadata: HashMap::new(),
        }
    }

    /// Create a failed inference response.
    pub fn failure(
        request_id: impl Into<String>,
        model: impl Into<String>,
        error: impl Into<String>,
        worker: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            model: model.into(),
            success: false,
            output: String::new(),
            error: Some(error.into()),
            tokens_generated: 0,
            inference_time_ms: 0,
            worker_node: worker.into(),
            metadata: HashMap::new(),
        }
    }

    /// Serialize to bytes for signing/transport.
    pub fn into_payload(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| NetworkError::Serialization(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_payload(payload: &[u8]) -> Result<Self> {
        serde_json::from_slice(payload)
            .map_err(|e| NetworkError::Serialization(e.to_string()))
    }

    /// Sign this response for return to coordinator.
    pub fn sign_for_response(
        &self,
        signer: &mut RequestSigner,
    ) -> Result<crate::proxy::envelope_sign::SignedRequest> {
        Ok(signer.sign(INFERENCE_RESPONSE_PATH.to_string(), self.into_payload()?))
    }
}

// ── WS2: Seal/Open helpers for inference dispatch ─────────────────────

/// Seal an inference request into an encrypted envelope for transport.
///
/// Flow: InferenceRequest → sign → ProxyEnvelope → seal → SealedMessage
pub async fn seal_inference_request(
    channel: &EncryptedChannel,
    signer: &mut RequestSigner,
    request: &InferenceRequest,
) -> Result<SealedMessage> {
    let signed = request.sign_for_dispatch(signer)?;
    let envelope_bytes = ProxyEnvelope::SignedRequest(signed).to_bytes()?;
    channel.seal(&envelope_bytes).await
}

/// Open and verify an inference request from a received sealed message.
///
/// Flow: SealedMessage → open → ProxyEnvelope → verify → InferenceRequest
pub async fn open_inference_request(
    channel: &EncryptedChannel,
    verifier: &mut RequestVerifier,
    peer: &NodeIdentity,
    sealed: &SealedMessage,
) -> Result<InferenceRequest> {
    let plaintext = match sealed {
        SealedMessage::Standard(env) => channel.open_standard(env).await?,
        SealedMessage::Cloaked(env) => channel.open_cloaked(env).await?,
    };
    let envelope = ProxyEnvelope::from_bytes(&plaintext)?;
    match envelope {
        ProxyEnvelope::SignedRequest(req) => {
            verifier.verify(&req, peer)?;
            if req.target != INFERENCE_DISPATCH_PATH {
                return Err(NetworkError::Protocol(format!(
                    "unexpected inference path: expected {}, got {}",
                    INFERENCE_DISPATCH_PATH, req.target
                )));
            }
            InferenceRequest::from_payload(&req.payload)
        }
    }
}

/// Seal an inference response into an encrypted envelope for transport.
///
/// Flow: InferenceResponse → sign → ProxyEnvelope → seal → SealedMessage
pub async fn seal_inference_response(
    channel: &EncryptedChannel,
    signer: &mut RequestSigner,
    response: &InferenceResponse,
) -> Result<SealedMessage> {
    let signed = response.sign_for_response(signer)?;
    let envelope_bytes = ProxyEnvelope::SignedRequest(signed).to_bytes()?;
    channel.seal(&envelope_bytes).await
}

/// Open and verify an inference response from a received sealed message.
///
/// Flow: SealedMessage → open → ProxyEnvelope → verify → InferenceResponse
pub async fn open_inference_response(
    channel: &EncryptedChannel,
    verifier: &mut RequestVerifier,
    peer: &NodeIdentity,
    sealed: &SealedMessage,
) -> Result<InferenceResponse> {
    let plaintext = match sealed {
        SealedMessage::Standard(env) => channel.open_standard(env).await?,
        SealedMessage::Cloaked(env) => channel.open_cloaked(env).await?,
    };
    let envelope = ProxyEnvelope::from_bytes(&plaintext)?;
    match envelope {
        ProxyEnvelope::SignedRequest(req) => {
            verifier.verify(&req, peer)?;
            if req.target != INFERENCE_RESPONSE_PATH {
                return Err(NetworkError::Protocol(format!(
                    "unexpected inference response path: expected {}, got {}",
                    INFERENCE_RESPONSE_PATH, req.target
                )));
            }
            InferenceResponse::from_payload(&req.payload)
        }
    }
}

// ── Heartbeat dispatch over encrypted channel ─────────────────────────

/// Seal a worker heartbeat into an encrypted envelope.
pub async fn seal_heartbeat(
    channel: &EncryptedChannel,
    signer: &mut RequestSigner,
    heartbeat: &WorkerHeartbeat,
) -> Result<SealedMessage> {
    let payload = heartbeat.to_json();
    let signed = signer.sign(HEARTBEAT_PATH.to_string(), payload);
    let envelope_bytes = ProxyEnvelope::SignedRequest(signed).to_bytes()?;
    channel.seal(&envelope_bytes).await
}

/// Open and verify a worker heartbeat from a received sealed message.
pub async fn open_heartbeat(
    channel: &EncryptedChannel,
    verifier: &mut RequestVerifier,
    peer: &NodeIdentity,
    sealed: &SealedMessage,
) -> Result<WorkerHeartbeat> {
    let plaintext = match sealed {
        SealedMessage::Standard(env) => channel.open_standard(env).await?,
        SealedMessage::Cloaked(env) => channel.open_cloaked(env).await?,
    };
    let envelope = ProxyEnvelope::from_bytes(&plaintext)?;
    match envelope {
        ProxyEnvelope::SignedRequest(req) => {
            verifier.verify(&req, peer)?;
            if req.target != HEARTBEAT_PATH {
                return Err(NetworkError::Protocol(format!(
                    "unexpected heartbeat path: expected {}, got {}",
                    HEARTBEAT_PATH, req.target
                )));
            }
            WorkerHeartbeat::from_json(&req.payload).ok_or_else(|| {
                NetworkError::Serialization("invalid heartbeat payload".into())
            })
        }
    }
}

// ── WS4: GPU channel handshake helpers ────────────────────────────────

/// Establish an encrypted channel between coordinator and GPU worker.
///
/// The coordinator initiates, the GPU worker accepts.
/// Returns both sides of the channel for testing.
pub async fn establish_gpu_channel(
    coord_channel: &EncryptedChannel,
    gpu_channel: &EncryptedChannel,
) -> Result<()> {
    // Coordinator initiates handshake
    let init = coord_channel.initiate().await?;
    // GPU worker accepts
    let accept = gpu_channel.accept(&init).await?;
    // Coordinator completes
    coord_channel.complete(&accept).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::channel::ChannelConfig;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::transport::NodeTransport;
    use bunny_crypto::types::AgentId;
    use std::sync::Arc;

    use crate::proxy::gpu_worker::{LoadedModel, RuntimeHealthInfo, WorkerState};

    fn make_node(role: NodeRole) -> SwarmNode {
        SwarmNode::generate(
            role,
            NodeCapabilities {
                models: vec![],
                has_gpu: role == NodeRole::Worker,
                max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            },
        )
    }

    /// Set up coordinator ↔ GPU worker encrypted channels.
    async fn setup_gpu_channels() -> (
        EncryptedChannel,
        EncryptedChannel,
        SwarmNode,
        SwarmNode,
    ) {
        let coord_node = make_node(NodeRole::Orchestrator);
        let gpu_node = make_node(NodeRole::Worker);

        let coord_transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Orchestrator, NodeCapabilities {
                models: vec![], has_gpu: false, max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        let gpu_transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, NodeCapabilities {
                models: vec!["llama3.1:8b".into()], has_gpu: true, max_sessions: 50,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));

        let coord_ch = EncryptedChannel::new(coord_transport, ChannelConfig::performance());
        let gpu_ch = EncryptedChannel::new(gpu_transport, ChannelConfig::performance());

        // Full handshake
        establish_gpu_channel(&coord_ch, &gpu_ch).await.unwrap();

        (coord_ch, gpu_ch, coord_node, gpu_node)
    }

    fn sample_inference_request() -> InferenceRequest {
        InferenceRequest::new(
            "llama3.1:8b",
            "What is the meaning of life?",
            AgentId::new(),
        )
        .with_temperature(0.7)
        .with_max_tokens(1024)
        .with_param("system_prompt", "You are a helpful assistant.")
    }

    // ── InferenceRequest serialization ────────────────────────────────

    #[test]
    fn inference_request_roundtrip() {
        let req = sample_inference_request();
        let payload = req.into_payload().unwrap();
        let restored = InferenceRequest::from_payload(&payload).unwrap();

        assert_eq!(restored.model, "llama3.1:8b");
        assert_eq!(restored.prompt, "What is the meaning of life?");
        assert_eq!(restored.temperature, 0.7);
        assert_eq!(restored.max_tokens, 1024);
        assert_eq!(restored.params.get("system_prompt").unwrap(), "You are a helpful assistant.");
    }

    #[test]
    fn inference_response_success_roundtrip() {
        let resp = InferenceResponse::success(
            "infer-001",
            "llama3.1:8b",
            "The meaning of life is 42.",
            12,
            850,
            "swarm-gpu",
        );

        let payload = resp.into_payload().unwrap();
        let restored = InferenceResponse::from_payload(&payload).unwrap();

        assert_eq!(restored.request_id, "infer-001");
        assert!(restored.success);
        assert_eq!(restored.output, "The meaning of life is 42.");
        assert_eq!(restored.tokens_generated, 12);
        assert_eq!(restored.inference_time_ms, 850);
        assert_eq!(restored.worker_node, "swarm-gpu");
        assert!(restored.error.is_none());
    }

    #[test]
    fn inference_response_failure_roundtrip() {
        let resp = InferenceResponse::failure(
            "infer-002",
            "llama3.1:8b",
            "Ollama returned 503: model not loaded",
            "swarm-gpu",
        );

        let payload = resp.into_payload().unwrap();
        let restored = InferenceResponse::from_payload(&payload).unwrap();

        assert!(!restored.success);
        assert!(restored.output.is_empty());
        assert_eq!(
            restored.error.as_deref(),
            Some("Ollama returned 503: model not loaded")
        );
    }

    // ── WS4: GPU channel handshake ───────────────────────────────────

    #[tokio::test]
    async fn gpu_channel_handshake() {
        let (coord_ch, gpu_ch, _coord_node, _gpu_node) = setup_gpu_channels().await;

        use crate::proxy::channel::ChannelState;
        assert_eq!(coord_ch.state().await, ChannelState::Established);
        assert_eq!(gpu_ch.state().await, ChannelState::Established);

        // Both sides have peer identity
        assert!(coord_ch.peer().await.is_some());
        assert!(gpu_ch.peer().await.is_some());
    }

    // ── WS2: Seal + Open inference request ──────────────────────────

    #[tokio::test]
    async fn seal_and_open_inference_request() {
        let (coord_ch, gpu_ch, coord_node, _gpu_node) = setup_gpu_channels().await;

        let coord_identity = coord_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(coord_node);
        let mut verifier = RequestVerifier::new(60_000);

        let req = sample_inference_request();

        // Coordinator seals inference request
        let sealed = seal_inference_request(&coord_ch, &mut signer, &req)
            .await
            .unwrap();

        // GPU worker opens and verifies
        let verified = open_inference_request(
            &gpu_ch,
            &mut verifier,
            &coord_identity,
            &sealed,
        )
        .await
        .unwrap();

        assert_eq!(verified.model, "llama3.1:8b");
        assert_eq!(verified.prompt, "What is the meaning of life?");
        assert_eq!(verified.max_tokens, 1024);
    }

    // ── WS2: Seal + Open inference response ─────────────────────────

    #[tokio::test]
    async fn seal_and_open_inference_response() {
        let (coord_ch, gpu_ch, _coord_node, gpu_node) = setup_gpu_channels().await;

        let gpu_identity = gpu_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(gpu_node);
        let mut verifier = RequestVerifier::new(60_000);

        let resp = InferenceResponse::success(
            "infer-001",
            "llama3.1:8b",
            "The answer is 42.",
            8,
            650,
            "swarm-gpu",
        );

        // GPU worker seals response
        let sealed = seal_inference_response(&gpu_ch, &mut signer, &resp)
            .await
            .unwrap();

        // Coordinator opens and verifies
        let verified = open_inference_response(
            &coord_ch,
            &mut verifier,
            &gpu_identity,
            &sealed,
        )
        .await
        .unwrap();

        assert_eq!(verified.request_id, "infer-001");
        assert!(verified.success);
        assert_eq!(verified.output, "The answer is 42.");
        assert_eq!(verified.tokens_generated, 8);
        assert_eq!(verified.worker_node, "swarm-gpu");
    }

    // ── Full dispatch roundtrip ─────────────────────────────────────

    #[tokio::test]
    async fn full_inference_dispatch_roundtrip() {
        let (coord_ch, gpu_ch, coord_node, gpu_node) = setup_gpu_channels().await;

        let coord_identity = coord_node.signed_announcement().identity.clone();
        let gpu_identity = gpu_node.signed_announcement().identity.clone();

        let mut coord_signer = RequestSigner::new(coord_node);
        let mut gpu_signer = RequestSigner::new(gpu_node);
        let mut coord_verifier = RequestVerifier::new(60_000);
        let mut gpu_verifier = RequestVerifier::new(60_000);

        // 1. Coordinator dispatches inference request to GPU
        let req = InferenceRequest::new(
            "llama3.1:8b",
            "Summarize quantum computing.",
            AgentId::new(),
        )
        .with_param("channel", "slack")
        .with_param("user_id", "U12345");

        let sealed_req = seal_inference_request(&coord_ch, &mut coord_signer, &req)
            .await
            .unwrap();

        // 2. GPU worker receives and verifies request
        let received_req = open_inference_request(
            &gpu_ch,
            &mut gpu_verifier,
            &coord_identity,
            &sealed_req,
        )
        .await
        .unwrap();

        assert_eq!(received_req.model, "llama3.1:8b");
        assert_eq!(received_req.params.get("channel").unwrap(), "slack");

        // 3. GPU worker processes and sends response
        let resp = InferenceResponse::success(
            &received_req.request_id,
            &received_req.model,
            "Quantum computing uses qubits for parallel computation.",
            14,
            1200,
            "swarm-gpu",
        );

        let sealed_resp = seal_inference_response(&gpu_ch, &mut gpu_signer, &resp)
            .await
            .unwrap();

        // 4. Coordinator receives and verifies response
        let received_resp = open_inference_response(
            &coord_ch,
            &mut coord_verifier,
            &gpu_identity,
            &sealed_resp,
        )
        .await
        .unwrap();

        assert_eq!(received_resp.request_id, received_req.request_id);
        assert!(received_resp.success);
        assert_eq!(received_resp.tokens_generated, 14);
        assert_eq!(received_resp.inference_time_ms, 1200);
    }

    // ── Wrong identity rejected ─────────────────────────────────────

    #[tokio::test]
    async fn inference_wrong_identity_rejected() {
        let (coord_ch, gpu_ch, coord_node, _gpu_node) = setup_gpu_channels().await;

        let wrong_node = make_node(NodeRole::Worker);
        let wrong_identity = wrong_node.signed_announcement().identity.clone();

        let mut signer = RequestSigner::new(coord_node);
        let mut verifier = RequestVerifier::new(60_000);

        let req = sample_inference_request();
        let sealed = seal_inference_request(&coord_ch, &mut signer, &req)
            .await
            .unwrap();

        // Try to verify with wrong identity
        let result = open_inference_request(
            &gpu_ch,
            &mut verifier,
            &wrong_identity,
            &sealed,
        )
        .await;
        assert!(result.is_err());
    }

    // ── Replay attack rejected ──────────────────────────────────────

    #[tokio::test]
    async fn inference_replay_rejected() {
        let (coord_ch, gpu_ch, coord_node, _gpu_node) = setup_gpu_channels().await;

        let coord_identity = coord_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(coord_node);
        let mut verifier = RequestVerifier::new(60_000);

        // Sign two requests — seq 0 then seq 1
        let req_a = sample_inference_request();
        let req_b = InferenceRequest::new(
            "llama3.1:8b",
            "Second request",
            AgentId::new(),
        );

        let sealed_a = seal_inference_request(&coord_ch, &mut signer, &req_a)
            .await
            .unwrap(); // seq=0
        let sealed_b = seal_inference_request(&coord_ch, &mut signer, &req_b)
            .await
            .unwrap(); // seq=1

        // Verify seq=1 first
        assert!(open_inference_request(
            &gpu_ch, &mut verifier, &coord_identity, &sealed_b
        ).await.is_ok());

        // Replay seq=0 — should fail
        assert!(open_inference_request(
            &gpu_ch, &mut verifier, &coord_identity, &sealed_a
        ).await.is_err());
    }

    // ── Heartbeat seal + open ───────────────────────────────────────

    #[tokio::test]
    async fn seal_and_open_heartbeat() {
        let (coord_ch, gpu_ch, _coord_node, gpu_node) = setup_gpu_channels().await;

        let gpu_identity = gpu_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(gpu_node);
        let mut verifier = RequestVerifier::new(60_000);

        let runtime = RuntimeHealthInfo::ollama_healthy(vec![
            LoadedModel {
                name: "llama3.1:8b".into(),
                ready: true,
                size_bytes: Some(4_500_000_000),
            },
        ]);
        let heartbeat = WorkerHeartbeat::new("swarm-gpu", Some(runtime));

        // GPU seals heartbeat
        let sealed = seal_heartbeat(&gpu_ch, &mut signer, &heartbeat)
            .await
            .unwrap();

        // Coordinator opens and verifies
        let verified = open_heartbeat(
            &coord_ch,
            &mut verifier,
            &gpu_identity,
            &sealed,
        )
        .await
        .unwrap();

        assert_eq!(verified.node_name, "swarm-gpu");
        assert_eq!(verified.state, WorkerState::Healthy);
        assert!(verified.runtime.as_ref().unwrap().healthy);
        assert!(verified.runtime.as_ref().unwrap().has_model_ready("llama3.1:8b"));
    }

    // ── Heartbeat wrong identity ────────────────────────────────────

    #[tokio::test]
    async fn heartbeat_wrong_identity_rejected() {
        let (coord_ch, gpu_ch, _coord_node, gpu_node) = setup_gpu_channels().await;

        let wrong_node = make_node(NodeRole::Worker);
        let wrong_identity = wrong_node.signed_announcement().identity.clone();

        let mut signer = RequestSigner::new(gpu_node);
        let mut verifier = RequestVerifier::new(60_000);

        let heartbeat = WorkerHeartbeat::new("swarm-gpu", None);
        let sealed = seal_heartbeat(&gpu_ch, &mut signer, &heartbeat)
            .await
            .unwrap();

        let result = open_heartbeat(
            &coord_ch,
            &mut verifier,
            &wrong_identity,
            &sealed,
        )
        .await;
        assert!(result.is_err());
    }

    // ── Failed inference roundtrip ──────────────────────────────────

    #[tokio::test]
    async fn failed_inference_roundtrip() {
        let (coord_ch, gpu_ch, coord_node, gpu_node) = setup_gpu_channels().await;

        let coord_identity = coord_node.signed_announcement().identity.clone();
        let gpu_identity = gpu_node.signed_announcement().identity.clone();

        let mut coord_signer = RequestSigner::new(coord_node);
        let mut gpu_signer = RequestSigner::new(gpu_node);
        let mut coord_verifier = RequestVerifier::new(60_000);
        let mut gpu_verifier = RequestVerifier::new(60_000);

        // Coordinator dispatches
        let req = InferenceRequest::new("llama3.1:8b", "test", AgentId::new());
        let sealed_req = seal_inference_request(&coord_ch, &mut coord_signer, &req)
            .await
            .unwrap();

        let received_req = open_inference_request(
            &gpu_ch, &mut gpu_verifier, &coord_identity, &sealed_req,
        ).await.unwrap();

        // GPU returns failure
        let resp = InferenceResponse::failure(
            &received_req.request_id,
            &received_req.model,
            "CUDA out of memory",
            "swarm-gpu",
        );

        let sealed_resp = seal_inference_response(&gpu_ch, &mut gpu_signer, &resp)
            .await
            .unwrap();

        let received_resp = open_inference_response(
            &coord_ch, &mut coord_verifier, &gpu_identity, &sealed_resp,
        ).await.unwrap();

        assert!(!received_resp.success);
        assert_eq!(received_resp.error.as_deref(), Some("CUDA out of memory"));
        assert_eq!(received_resp.tokens_generated, 0);
    }

    // ── Well-known paths ────────────────────────────────────────────

    #[test]
    fn well_known_inference_paths() {
        assert_eq!(INFERENCE_DISPATCH_PATH, "/bunny/inference/dispatch");
        assert_eq!(INFERENCE_RESPONSE_PATH, "/bunny/inference/response");
        assert_eq!(HEARTBEAT_PATH, "/bunny/worker/heartbeat");
    }

    // ── Invalid payload rejected ────────────────────────────────────

    #[test]
    fn invalid_inference_payload_rejected() {
        assert!(InferenceRequest::from_payload(b"not valid json").is_err());
        assert!(InferenceResponse::from_payload(b"{broken").is_err());
    }
}
