//! OpenClaw / Slack path restoration through encrypted coordinator.
//!
//! Implements Workstream 6 of the Phase 4 directive:
//! The bridge connects OpenClaw tool calls (from Slack/messaging) to
//! GPU inference dispatch through the coordinator's encrypted channels.
//!
//! Complete path:
//! ```text
//! Slack message
//!   → OpenClaw ToolCall (Python) → signed envelope → encrypt
//!     → Coordinator (swarm-mainframe)
//!       → Route to inference dispatch
//!         → InferenceRequest → signed → encrypted → GPU worker (swarm-gpu)
//!         ← InferenceResponse ← signed ← encrypted ← GPU worker
//!       ← ToolResult ← signed ← encrypted
//!     ← Coordinator → OpenClaw → Slack reply
//! ```
//!
//! This module provides the coordinator-side routing logic that converts
//! incoming tool calls into inference requests and routes them to the
//! correct GPU worker.

use std::collections::HashMap;

use bunny_crypto::identity::NodeIdentity;

use crate::error::{NetworkError, Result};
use crate::proxy::channel::{EncryptedChannel, SealedMessage};
use crate::proxy::envelope_sign::{RequestSigner, RequestVerifier};
use crate::proxy::cloud_fallback::CloudFallbackProvider;
use crate::proxy::gpu_worker::GpuWorkerRegistration;
use crate::proxy::inference_dispatch::{
    seal_inference_request, open_inference_response,
    InferenceRequest, InferenceResponse,
};
use crate::proxy::tool_call::{ToolCall, ToolResult};

/// Well-known tool name that triggers inference dispatch.
pub const INFERENCE_TOOL: &str = "inference";

/// Well-known tool names that should route through GPU worker.
pub const GPU_ROUTABLE_TOOLS: &[&str] = &[
    "inference",
    "ollama_generate",
    "ollama_chat",
    "model_inference",
    "ai_completion",
];

/// Routing decision for an incoming tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRoute {
    /// Route to GPU worker for inference.
    GpuInference {
        /// Model to use.
        model: String,
        /// Target worker name.
        worker: String,
    },
    /// GPU unavailable — fall back to cloud API.
    CloudFallback {
        /// Model requested (will be mapped to cloud equivalent).
        model: String,
    },
    /// Execute locally on coordinator (CRM, email, etc.).
    LocalExecution,
    /// No available worker AND no cloud fallback — reject.
    NoWorkerAvailable,
}

/// Classify an incoming tool call into a routing decision.
///
/// Checks if the tool name is GPU-routable and if a healthy worker is available.
pub fn classify_tool_route(
    call: &ToolCall,
    workers: &[&GpuWorkerRegistration],
) -> ToolRoute {
    // Check if this tool should route to GPU
    let is_gpu_tool = GPU_ROUTABLE_TOOLS.iter().any(|t| *t == call.tool_name);

    if !is_gpu_tool {
        return ToolRoute::LocalExecution;
    }

    // Find a healthy, dispatchable worker
    let model = extract_model_from_args(&call.args_json)
        .unwrap_or_else(|| "llama3.1:8b".into());

    for worker in workers {
        if worker.is_dispatchable() {
            // Check if the worker has the requested model
            let has_model = worker.runtime.as_ref()
                .map(|r| r.has_model_ready(&model))
                .unwrap_or(false);

            if has_model {
                return ToolRoute::GpuInference {
                    model,
                    worker: worker.name.clone(),
                };
            }
        }
    }

    // Fallback: try any healthy worker even without model check
    for worker in workers {
        if worker.is_dispatchable() {
            return ToolRoute::GpuInference {
                model,
                worker: worker.name.clone(),
            };
        }
    }

    ToolRoute::NoWorkerAvailable
}

/// Classify with cloud fallback — when GPU is unavailable, fall back to cloud.
///
/// Same as `classify_tool_route`, but returns `CloudFallback` instead of
/// `NoWorkerAvailable` when a cloud provider is configured.
pub fn classify_tool_route_with_fallback(
    call: &ToolCall,
    workers: &[&GpuWorkerRegistration],
    cloud: &CloudFallbackProvider,
) -> ToolRoute {
    let route = classify_tool_route(call, workers);

    match route {
        ToolRoute::NoWorkerAvailable if cloud.is_available() => {
            let model = extract_model_from_args(&call.args_json)
                .unwrap_or_else(|| "llama3.1:8b".into());
            ToolRoute::CloudFallback { model }
        }
        other => other,
    }
}

/// Full dispatch with cloud bypass — the main entry point for Slack → inference.
///
/// Tries GPU worker first; if unavailable, falls back to cloud API.
/// Returns a ToolResult ready for OpenClaw/Slack.
pub async fn dispatch_with_fallback(
    call: &ToolCall,
    workers: &[&GpuWorkerRegistration],
    cloud: &mut CloudFallbackProvider,
    // GPU channel + signer/verifier only needed when GPU is available
    gpu_channel: Option<(&EncryptedChannel, &mut RequestSigner)>,
) -> Result<ToolResult> {
    let route = classify_tool_route_with_fallback(call, workers, cloud);

    match route {
        ToolRoute::GpuInference { .. } => {
            // Route through encrypted GPU channel
            if let Some((channel, signer)) = gpu_channel {
                let (req, _sealed) = dispatch_tool_to_gpu(call, channel, signer).await?;
                // Note: response reception handled separately by caller
                // Return a pending result — caller will await GPU response
                Ok(ToolResult {
                    request_id: call.request_id.clone(),
                    tool_name: call.tool_name.clone(),
                    success: true,
                    output_json: serde_json::json!({
                        "status": "dispatched_to_gpu",
                        "inference_request_id": req.request_id,
                    }).to_string(),
                    error: None,
                    metadata: HashMap::from([
                        ("dispatch_target".into(), "gpu".into()),
                    ]),
                })
            } else {
                Err(NetworkError::ToolDispatch(
                    "GPU route selected but no channel available".into()
                ))
            }
        }
        ToolRoute::CloudFallback { .. } => {
            // Bypass GPU — query cloud directly
            let infer_req = tool_call_to_inference_request(call)?;
            let response = cloud.query(&infer_req).await?;
            Ok(crate::proxy::cloud_fallback::cloud_response_to_tool_result(
                &response,
                &call.request_id,
            ))
        }
        ToolRoute::LocalExecution => {
            // Not an inference tool — handled by local executor
            Err(NetworkError::ToolDispatch(
                "tool should be executed locally, not dispatched".into()
            ))
        }
        ToolRoute::NoWorkerAvailable => {
            Ok(ToolResult {
                request_id: call.request_id.clone(),
                tool_name: call.tool_name.clone(),
                success: false,
                output_json: String::new(),
                error: Some("no GPU worker available and no cloud fallback configured".into()),
                metadata: HashMap::new(),
            })
        }
    }
}

/// Extract model name from tool call args JSON.
///
/// Looks for "model" key in the JSON args.
fn extract_model_from_args(args_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(args_json)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(String::from))
}

/// Extract prompt from tool call args JSON.
///
/// Looks for "prompt", "message", or "input" keys.
fn extract_prompt_from_args(args_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    v.get("prompt")
        .or_else(|| v.get("message"))
        .or_else(|| v.get("input"))
        .and_then(|p| p.as_str())
        .map(String::from)
}

/// Convert an OpenClaw ToolCall into an InferenceRequest for GPU dispatch.
///
/// Extracts model, prompt, and parameters from the tool call args.
pub fn tool_call_to_inference_request(call: &ToolCall) -> Result<InferenceRequest> {
    let model = extract_model_from_args(&call.args_json)
        .unwrap_or_else(|| "llama3.1:8b".into());

    let prompt = extract_prompt_from_args(&call.args_json)
        .ok_or_else(|| NetworkError::ToolDispatch(
            "tool call args must contain 'prompt', 'message', or 'input' field".into()
        ))?;

    let mut req = InferenceRequest::new(model, prompt, call.caller.clone());

    // Transfer metadata
    for (k, v) in &call.metadata {
        req.params.insert(k.clone(), v.clone());
    }

    // Extract optional parameters from args
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&call.args_json) {
        if let Some(temp) = v.get("temperature").and_then(|t| t.as_f64()) {
            req.temperature = temp as f32;
        }
        if let Some(max) = v.get("max_tokens").and_then(|t| t.as_u64()) {
            req.max_tokens = max as u32;
        }
        if let Some(sys) = v.get("system_prompt").and_then(|s| s.as_str()) {
            req.params.insert("system_prompt".into(), sys.into());
        }
    }

    // Copy request_id for correlation
    req.params.insert("openclaw_request_id".into(), call.request_id.clone());

    Ok(req)
}

/// Convert an InferenceResponse back into an OpenClaw ToolResult.
///
/// Maps the GPU worker's response into the format OpenClaw/Python expects.
pub fn inference_response_to_tool_result(
    response: &InferenceResponse,
    original_request_id: &str,
) -> ToolResult {
    let mut metadata = HashMap::new();
    metadata.insert("worker_node".into(), response.worker_node.clone());
    metadata.insert("model".into(), response.model.clone());
    metadata.insert("tokens_generated".into(), response.tokens_generated.to_string());
    metadata.insert("inference_time_ms".into(), response.inference_time_ms.to_string());

    // Copy any extra metadata from inference response
    for (k, v) in &response.metadata {
        metadata.insert(k.clone(), v.clone());
    }

    ToolResult {
        request_id: original_request_id.into(),
        tool_name: INFERENCE_TOOL.into(),
        success: response.success,
        output_json: if response.success {
            serde_json::json!({
                "output": response.output,
                "model": response.model,
                "tokens": response.tokens_generated,
                "time_ms": response.inference_time_ms,
            }).to_string()
        } else {
            String::new()
        },
        error: response.error.clone(),
        metadata,
    }
}

/// Coordinator dispatch pipeline: ToolCall → InferenceRequest → seal.
///
/// Full pipeline for the coordinator side:
/// 1. Classify the tool call route
/// 2. Convert to InferenceRequest
/// 3. Seal for transport to GPU worker
pub async fn dispatch_tool_to_gpu(
    call: &ToolCall,
    gpu_channel: &EncryptedChannel,
    signer: &mut RequestSigner,
) -> Result<(InferenceRequest, SealedMessage)> {
    let req = tool_call_to_inference_request(call)?;
    let sealed = seal_inference_request(gpu_channel, signer, &req).await?;
    Ok((req, sealed))
}

/// Coordinator receive pipeline: open InferenceResponse → ToolResult.
///
/// Full pipeline for the coordinator receiving GPU worker's response:
/// 1. Open and verify the inference response
/// 2. Convert to ToolResult for return to OpenClaw
pub async fn receive_gpu_response(
    gpu_channel: &EncryptedChannel,
    verifier: &mut RequestVerifier,
    gpu_identity: &NodeIdentity,
    sealed: &SealedMessage,
    original_request_id: &str,
) -> Result<ToolResult> {
    let response = open_inference_response(
        gpu_channel,
        verifier,
        gpu_identity,
        sealed,
    ).await?;

    Ok(inference_response_to_tool_result(&response, original_request_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::channel::ChannelConfig;
    use crate::proxy::gpu_worker::{LoadedModel, RuntimeHealthInfo, RuntimeKind};
    use crate::proxy::inference_dispatch::establish_gpu_channel;
    use crate::proxy::tool_call::ToolCallPriority;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::transport::NodeTransport;
    use bunny_crypto::types::AgentId;
    use std::sync::Arc;

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

    fn make_healthy_registration() -> GpuWorkerRegistration {
        let transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, NodeCapabilities {
                models: vec!["llama3.1:8b".into()], has_gpu: true, max_sessions: 50,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );
        reg.set_channel(EncryptedChannel::new(transport, ChannelConfig::performance()));
        reg.record_heartbeat(Some(RuntimeHealthInfo::ollama_healthy(vec![
            LoadedModel { name: "llama3.1:8b".into(), ready: true, size_bytes: Some(4_500_000_000) },
        ])));
        reg
    }

    fn inference_tool_call() -> ToolCall {
        ToolCall {
            tool_name: "inference".into(),
            args_json: serde_json::json!({
                "model": "llama3.1:8b",
                "prompt": "What is quantum computing?",
                "temperature": 0.7,
                "max_tokens": 512,
                "system_prompt": "You are a helpful assistant."
            }).to_string(),
            request_id: "req-slack-001".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Standard,
            metadata: HashMap::from([
                ("channel".into(), "slack".into()),
                ("user_id".into(), "U12345".into()),
            ]),
        }
    }

    fn email_tool_call() -> ToolCall {
        ToolCall {
            tool_name: "send_email".into(),
            args_json: r#"{"to_email":"bob@example.com","name":"Bob"}"#.into(),
            request_id: "req-email-001".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Standard,
            metadata: HashMap::new(),
        }
    }

    // ── Routing ─────────────────────────────────────────────────────

    #[test]
    fn route_inference_to_gpu() {
        let reg = make_healthy_registration();
        let call = inference_tool_call();
        let route = classify_tool_route(&call, &[&reg]);

        match route {
            ToolRoute::GpuInference { model, worker } => {
                assert_eq!(model, "llama3.1:8b");
                assert_eq!(worker, "swarm-gpu");
            }
            _ => panic!("expected GpuInference route"),
        }
    }

    #[test]
    fn route_email_locally() {
        let reg = make_healthy_registration();
        let call = email_tool_call();
        let route = classify_tool_route(&call, &[&reg]);
        assert_eq!(route, ToolRoute::LocalExecution);
    }

    #[test]
    fn route_no_worker_available() {
        // No workers registered
        let call = inference_tool_call();
        let route = classify_tool_route(&call, &[]);
        assert_eq!(route, ToolRoute::NoWorkerAvailable);
    }

    #[test]
    fn route_unavailable_worker_rejected() {
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );
        reg.mark_unavailable();

        let call = inference_tool_call();
        let route = classify_tool_route(&call, &[&reg]);
        assert_eq!(route, ToolRoute::NoWorkerAvailable);
    }

    #[test]
    fn route_degraded_worker_rejected() {
        let transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Worker, NodeCapabilities {
                models: vec![], has_gpu: true, max_sessions: 50,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );
        reg.set_channel(EncryptedChannel::new(transport, ChannelConfig::performance()));
        reg.record_heartbeat(Some(RuntimeHealthInfo::unhealthy(RuntimeKind::Ollama)));

        let call = inference_tool_call();
        let route = classify_tool_route(&call, &[&reg]);
        assert_eq!(route, ToolRoute::NoWorkerAvailable);
    }

    #[test]
    fn route_ollama_tools() {
        let reg = make_healthy_registration();

        for tool_name in &["ollama_generate", "ollama_chat", "model_inference", "ai_completion"] {
            let call = ToolCall {
                tool_name: tool_name.to_string(),
                args_json: r#"{"model":"llama3.1:8b","prompt":"test"}"#.into(),
                request_id: "req-001".into(),
                caller: AgentId::new(),
                priority: ToolCallPriority::Standard,
                metadata: HashMap::new(),
            };
            let route = classify_tool_route(&call, &[&reg]);
            assert!(matches!(route, ToolRoute::GpuInference { .. }),
                "tool '{}' should route to GPU", tool_name);
        }
    }

    // ── Conversion: ToolCall → InferenceRequest ─────────────────────

    #[test]
    fn tool_call_to_inference_request_conversion() {
        let call = inference_tool_call();
        let req = tool_call_to_inference_request(&call).unwrap();

        assert_eq!(req.model, "llama3.1:8b");
        assert_eq!(req.prompt, "What is quantum computing?");
        assert_eq!(req.temperature, 0.7);
        assert_eq!(req.max_tokens, 512);
        assert_eq!(req.params.get("system_prompt").unwrap(), "You are a helpful assistant.");
        assert_eq!(req.params.get("channel").unwrap(), "slack");
        assert_eq!(req.params.get("user_id").unwrap(), "U12345");
        assert_eq!(req.params.get("openclaw_request_id").unwrap(), "req-slack-001");
    }

    #[test]
    fn tool_call_missing_prompt_rejected() {
        let call = ToolCall {
            tool_name: "inference".into(),
            args_json: r#"{"model":"llama3.1:8b"}"#.into(),
            request_id: "req-001".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Standard,
            metadata: HashMap::new(),
        };
        assert!(tool_call_to_inference_request(&call).is_err());
    }

    #[test]
    fn tool_call_default_model() {
        let call = ToolCall {
            tool_name: "inference".into(),
            args_json: r#"{"prompt":"test"}"#.into(),
            request_id: "req-001".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Standard,
            metadata: HashMap::new(),
        };
        let req = tool_call_to_inference_request(&call).unwrap();
        assert_eq!(req.model, "llama3.1:8b"); // default fallback
    }

    // ── Conversion: InferenceResponse → ToolResult ──────────────────

    #[test]
    fn inference_response_to_tool_result_success() {
        let resp = InferenceResponse::success(
            "infer-001",
            "llama3.1:8b",
            "Quantum computing uses qubits.",
            10,
            800,
            "swarm-gpu",
        );

        let result = inference_response_to_tool_result(&resp, "req-slack-001");

        assert_eq!(result.request_id, "req-slack-001");
        assert_eq!(result.tool_name, "inference");
        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(result.metadata.get("worker_node").unwrap(), "swarm-gpu");
        assert_eq!(result.metadata.get("model").unwrap(), "llama3.1:8b");

        // Verify output JSON contains expected fields
        let output: serde_json::Value = serde_json::from_str(&result.output_json).unwrap();
        assert_eq!(output["output"], "Quantum computing uses qubits.");
        assert_eq!(output["tokens"], 10);
        assert_eq!(output["time_ms"], 800);
    }

    #[test]
    fn inference_response_to_tool_result_failure() {
        let resp = InferenceResponse::failure(
            "infer-002",
            "llama3.1:8b",
            "CUDA out of memory",
            "swarm-gpu",
        );

        let result = inference_response_to_tool_result(&resp, "req-slack-002");

        assert!(!result.success);
        assert!(result.output_json.is_empty());
        assert_eq!(result.error.as_deref(), Some("CUDA out of memory"));
    }

    // ── Full Slack → GPU → Response pipeline ────────────────────────

    #[tokio::test]
    async fn full_slack_to_gpu_pipeline() {
        // Set up encrypted channels
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

        let coord_node = make_node(NodeRole::Orchestrator);
        let gpu_node = make_node(NodeRole::Worker);
        let coord_identity = coord_node.signed_announcement().identity.clone();
        let gpu_identity = gpu_node.signed_announcement().identity.clone();

        let coord_ch = EncryptedChannel::new(coord_transport, ChannelConfig::performance());
        let gpu_ch = EncryptedChannel::new(gpu_transport, ChannelConfig::performance());

        establish_gpu_channel(&coord_ch, &gpu_ch).await.unwrap();

        let mut coord_signer = RequestSigner::new(coord_node);
        let mut gpu_signer = RequestSigner::new(gpu_node);
        let mut coord_verifier = RequestVerifier::new(60_000);
        let mut gpu_verifier = RequestVerifier::new(60_000);

        // ── Step 1: Slack sends ToolCall to coordinator ──
        let tool_call = inference_tool_call();
        let original_request_id = tool_call.request_id.clone();

        // ── Step 2: Coordinator dispatches to GPU ──
        let (infer_req, sealed_req) = dispatch_tool_to_gpu(
            &tool_call,
            &coord_ch,
            &mut coord_signer,
        ).await.unwrap();

        assert_eq!(infer_req.model, "llama3.1:8b");
        assert_eq!(infer_req.prompt, "What is quantum computing?");

        // ── Step 3: GPU worker receives and processes ──
        let received_req = crate::proxy::inference_dispatch::open_inference_request(
            &gpu_ch,
            &mut gpu_verifier,
            &coord_identity,
            &sealed_req,
        ).await.unwrap();

        assert_eq!(received_req.model, "llama3.1:8b");

        // GPU processes and returns response
        let resp = InferenceResponse::success(
            &received_req.request_id,
            &received_req.model,
            "Quantum computing leverages quantum mechanics for computation.",
            15,
            1100,
            "swarm-gpu",
        );

        let sealed_resp = crate::proxy::inference_dispatch::seal_inference_response(
            &gpu_ch,
            &mut gpu_signer,
            &resp,
        ).await.unwrap();

        // ── Step 4: Coordinator receives GPU response → ToolResult ──
        let tool_result = receive_gpu_response(
            &coord_ch,
            &mut coord_verifier,
            &gpu_identity,
            &sealed_resp,
            &original_request_id,
        ).await.unwrap();

        assert_eq!(tool_result.request_id, "req-slack-001");
        assert_eq!(tool_result.tool_name, "inference");
        assert!(tool_result.success);
        assert!(tool_result.error.is_none());

        // Parse output
        let output: serde_json::Value = serde_json::from_str(&tool_result.output_json).unwrap();
        assert_eq!(output["tokens"], 15);
        assert_eq!(output["time_ms"], 1100);
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn extract_model_from_various_args() {
        assert_eq!(
            extract_model_from_args(r#"{"model":"gpt-4"}"#),
            Some("gpt-4".into()),
        );
        assert_eq!(
            extract_model_from_args(r#"{"prompt":"test"}"#),
            None,
        );
        assert_eq!(
            extract_model_from_args("not json"),
            None,
        );
    }

    #[test]
    fn extract_prompt_from_various_keys() {
        assert_eq!(
            extract_prompt_from_args(r#"{"prompt":"test1"}"#),
            Some("test1".into()),
        );
        assert_eq!(
            extract_prompt_from_args(r#"{"message":"test2"}"#),
            Some("test2".into()),
        );
        assert_eq!(
            extract_prompt_from_args(r#"{"input":"test3"}"#),
            Some("test3".into()),
        );
        assert_eq!(
            extract_prompt_from_args(r#"{"other":"test4"}"#),
            None,
        );
    }

    #[test]
    fn gpu_routable_tools_list() {
        assert!(GPU_ROUTABLE_TOOLS.contains(&"inference"));
        assert!(GPU_ROUTABLE_TOOLS.contains(&"ollama_generate"));
        assert!(GPU_ROUTABLE_TOOLS.contains(&"ollama_chat"));
        assert!(!GPU_ROUTABLE_TOOLS.contains(&"send_email"));
        assert!(!GPU_ROUTABLE_TOOLS.contains(&"ghl_contact"));
    }

    // ── Cloud Fallback Routing ──────────────────────────────────────

    fn make_cloud_provider(has_key: bool) -> CloudFallbackProvider {
        use crate::proxy::cloud_fallback::{CloudProviderConfig, CloudProvider};
        let key = if has_key {
            "xai-real-production-key-12345"
        } else {
            "your_xai_api_key_here"
        };
        CloudFallbackProvider::new(vec![
            CloudProviderConfig::new(CloudProvider::Xai, key),
        ])
    }

    #[test]
    fn cloud_fallback_when_gpu_dead() {
        // GPU unavailable + cloud configured → CloudFallback
        let call = inference_tool_call();
        let cloud = make_cloud_provider(true);
        let route = classify_tool_route_with_fallback(&call, &[], &cloud);

        match route {
            ToolRoute::CloudFallback { model } => {
                assert_eq!(model, "llama3.1:8b");
            }
            _ => panic!("expected CloudFallback, got {:?}", route),
        }
    }

    #[test]
    fn gpu_preferred_over_cloud() {
        // GPU healthy + cloud configured → still routes to GPU
        let reg = make_healthy_registration();
        let cloud = make_cloud_provider(true);
        let call = inference_tool_call();
        let route = classify_tool_route_with_fallback(&call, &[&reg], &cloud);

        assert!(matches!(route, ToolRoute::GpuInference { .. }),
            "GPU should be preferred over cloud fallback");
    }

    #[test]
    fn no_cloud_no_gpu_still_unavailable() {
        // GPU dead + no cloud key → NoWorkerAvailable
        let call = inference_tool_call();
        let cloud = make_cloud_provider(false);
        let route = classify_tool_route_with_fallback(&call, &[], &cloud);
        assert_eq!(route, ToolRoute::NoWorkerAvailable);
    }

    #[test]
    fn cloud_fallback_does_not_affect_local_tools() {
        // Email tool should still be LocalExecution even with cloud
        let call = email_tool_call();
        let cloud = make_cloud_provider(true);
        let route = classify_tool_route_with_fallback(&call, &[], &cloud);
        assert_eq!(route, ToolRoute::LocalExecution);
    }

    #[test]
    fn cloud_fallback_with_unavailable_worker() {
        // Worker exists but is Unavailable + cloud configured → CloudFallback
        let mut reg = GpuWorkerRegistration::new(
            "swarm-gpu",
            "10.142.0.6:9003".parse().unwrap(),
        );
        reg.mark_unavailable();

        let cloud = make_cloud_provider(true);
        let call = inference_tool_call();
        let route = classify_tool_route_with_fallback(&call, &[&reg], &cloud);

        assert!(matches!(route, ToolRoute::CloudFallback { .. }),
            "should fall back to cloud when worker is unavailable");
    }

    #[tokio::test]
    async fn dispatch_with_fallback_no_providers() {
        // No GPU, no cloud → ToolResult with error
        let call = inference_tool_call();
        let mut cloud = make_cloud_provider(false);

        let result = dispatch_with_fallback(
            &call,
            &[],
            &mut cloud,
            None,
        ).await.unwrap();

        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("no GPU worker"));
    }
}
