//! OpenClaw tool-call envelopes for authenticated inter-node tool dispatch.
//!
//! Represents OpenClaw-style tool execution requests/results, wraps them
//! in signed envelopes, and supports authenticated response return.
//!
//! OpenClaw request flow:
//! ```text
//! tool_name + args_json + request_id + caller + priority + metadata
//!   → ToolCall → SignedRequest (Ed25519)
//!     → EncryptedChannel.seal() → TCP wire → remote sidecar
//!       → ToolResult → SignedRequest → EncryptedChannel → caller
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use bunny_crypto::identity::NodeIdentity;
use bunny_crypto::types::AgentId;

use crate::error::{NetworkError, Result};
use crate::proxy::envelope_sign::{RequestSigner, RequestVerifier, SignedRequest};

/// Priority level for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolCallPriority {
    BestEffort,
    Standard,
    Critical,
}

/// An OpenClaw tool execution request.
///
/// Maps directly to the Python-side `ToolExecutor.execute_tool()` parameters:
/// - `tool_name` → which registered tool to run
/// - `args_json` → JSON-encoded parameters
/// - `request_id` → correlation ID
/// - `caller` → authenticated sender
/// - `priority` → dispatch urgency
/// - `metadata` → extra context (channel, user, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub args_json: String,
    pub request_id: String,
    pub caller: AgentId,
    pub priority: ToolCallPriority,
    pub metadata: HashMap<String, String>,
}

/// An OpenClaw tool execution result.
///
/// Maps directly to the Python-side `ExecutionResult` dataclass:
/// - `request_id` → matches the originating ToolCall
/// - `success` → whether the tool completed without error
/// - `output_json` → JSON-encoded tool output
/// - `error` → error message if failed
/// - `metadata` → execution metadata (timing, node, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub request_id: String,
    pub tool_name: String,
    pub success: bool,
    pub output_json: String,
    pub error: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl ToolCall {
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

    /// Sign this tool call for a target path, producing a SignedRequest.
    pub fn sign_for_path(
        &self,
        signer: &mut RequestSigner,
        path: &str,
    ) -> Result<SignedRequest> {
        Ok(signer.sign(path.to_string(), self.into_payload()?))
    }
}

impl ToolResult {
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

    /// Sign this tool result for a target path, producing a SignedRequest.
    pub fn sign_for_path(
        &self,
        signer: &mut RequestSigner,
        path: &str,
    ) -> Result<SignedRequest> {
        Ok(signer.sign(path.to_string(), self.into_payload()?))
    }
}

/// Verify and extract a ToolCall from a SignedRequest.
///
/// Checks Ed25519 signature, timestamp freshness, anti-replay,
/// and validates the target path matches the expected tool call path.
pub fn verify_tool_call(
    verifier: &mut RequestVerifier,
    req: &SignedRequest,
    peer: &NodeIdentity,
    expected_path: &str,
) -> Result<ToolCall> {
    verifier.verify(req, peer)?;
    if req.target != expected_path {
        return Err(NetworkError::Protocol(format!(
            "unexpected tool call path: expected {}, got {}",
            expected_path, req.target
        )));
    }
    ToolCall::from_payload(&req.payload)
}

/// Verify and extract a ToolResult from a SignedRequest.
///
/// Checks Ed25519 signature, timestamp freshness, anti-replay,
/// and validates the target path matches the expected tool result path.
pub fn verify_tool_result(
    verifier: &mut RequestVerifier,
    req: &SignedRequest,
    peer: &NodeIdentity,
    expected_path: &str,
) -> Result<ToolResult> {
    verifier.verify(req, peer)?;
    if req.target != expected_path {
        return Err(NetworkError::Protocol(format!(
            "unexpected tool result path: expected {}, got {}",
            expected_path, req.target
        )));
    }
    ToolResult::from_payload(&req.payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};

    fn make_node(role: NodeRole) -> SwarmNode {
        SwarmNode::generate(
            role,
            NodeCapabilities {
                models: vec![],
                has_gpu: false,
                max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            },
        )
    }

    fn sample_tool_call() -> ToolCall {
        ToolCall {
            tool_name: "send_email".into(),
            args_json: r#"{"to_email":"bob@example.com","name":"Bob"}"#.into(),
            request_id: "req-001".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Standard,
            metadata: HashMap::from([
                ("channel".into(), "slack".into()),
                ("user_id".into(), "U12345".into()),
            ]),
        }
    }

    fn sample_tool_result(request_id: &str) -> ToolResult {
        ToolResult {
            request_id: request_id.into(),
            tool_name: "send_email".into(),
            success: true,
            output_json: r#"{"message_id":"abc123","sent":true}"#.into(),
            error: None,
            metadata: HashMap::from([
                ("executor_node".into(), "swarm-mainframe".into()),
                ("execution_time_ms".into(), "1234".into()),
            ]),
        }
    }

    #[test]
    fn serialize_deserialize_tool_call() {
        let call = sample_tool_call();
        let payload = call.into_payload().unwrap();
        let restored = ToolCall::from_payload(&payload).unwrap();

        assert_eq!(restored.tool_name, "send_email");
        assert_eq!(restored.request_id, "req-001");
        assert_eq!(restored.priority, ToolCallPriority::Standard);
        assert_eq!(restored.metadata.get("channel").unwrap(), "slack");
    }

    #[test]
    fn serialize_deserialize_tool_result() {
        let result = sample_tool_result("req-001");
        let payload = result.into_payload().unwrap();
        let restored = ToolResult::from_payload(&payload).unwrap();

        assert_eq!(restored.request_id, "req-001");
        assert!(restored.success);
        assert!(restored.error.is_none());
        assert_eq!(restored.metadata.get("executor_node").unwrap(), "swarm-mainframe");
    }

    #[test]
    fn sign_and_verify_tool_call() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);
        let mut verifier = RequestVerifier::new(60_000);

        let call = sample_tool_call();
        let signed = call
            .sign_for_path(&mut signer, "/openclaw/tool/execute")
            .unwrap();

        let verified = verify_tool_call(
            &mut verifier,
            &signed,
            &identity,
            "/openclaw/tool/execute",
        )
        .unwrap();

        assert_eq!(verified.tool_name, "send_email");
        assert_eq!(verified.request_id, "req-001");
    }

    #[test]
    fn sign_and_verify_tool_result() {
        let node = make_node(NodeRole::Worker);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);
        let mut verifier = RequestVerifier::new(60_000);

        let result = sample_tool_result("req-001");
        let signed = result
            .sign_for_path(&mut signer, "/openclaw/tool/result")
            .unwrap();

        let verified = verify_tool_result(
            &mut verifier,
            &signed,
            &identity,
            "/openclaw/tool/result",
        )
        .unwrap();

        assert_eq!(verified.request_id, "req-001");
        assert!(verified.success);
    }

    #[test]
    fn reject_wrong_path() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);
        let mut verifier = RequestVerifier::new(60_000);

        let call = sample_tool_call();
        let signed = call
            .sign_for_path(&mut signer, "/openclaw/tool/execute")
            .unwrap();

        let err = verify_tool_call(
            &mut verifier,
            &signed,
            &identity,
            "/wrong/path",
        );
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("unexpected tool call path"));
    }

    #[test]
    fn reject_wrong_identity() {
        let node_a = make_node(NodeRole::Gateway);
        let node_b = make_node(NodeRole::Worker);
        let identity_b = node_b.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node_a);
        let mut verifier = RequestVerifier::new(60_000);

        let call = sample_tool_call();
        let signed = call
            .sign_for_path(&mut signer, "/openclaw/tool/execute")
            .unwrap();

        let err = verify_tool_call(
            &mut verifier,
            &signed,
            &identity_b,
            "/openclaw/tool/execute",
        );
        assert!(err.is_err());
    }

    #[test]
    fn reject_replay() {
        let node = make_node(NodeRole::Gateway);
        let identity = node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(node);
        let mut verifier = RequestVerifier::new(60_000);

        // Sign two calls — seq 0, then seq 1
        let call_a = sample_tool_call();
        let call_b = ToolCall {
            request_id: "req-002".into(),
            ..sample_tool_call()
        };

        let signed_a = call_a
            .sign_for_path(&mut signer, "/openclaw/tool/execute")
            .unwrap(); // seq=0
        let signed_b = call_b
            .sign_for_path(&mut signer, "/openclaw/tool/execute")
            .unwrap(); // seq=1

        // Verify seq=1 first
        assert!(verify_tool_call(
            &mut verifier, &signed_b, &identity, "/openclaw/tool/execute"
        ).is_ok());

        // Replay seq=0 — should fail (anti-replay)
        assert!(verify_tool_call(
            &mut verifier, &signed_a, &identity, "/openclaw/tool/execute"
        ).is_err());
    }

    #[test]
    fn priority_serialization() {
        for priority in [
            ToolCallPriority::BestEffort,
            ToolCallPriority::Standard,
            ToolCallPriority::Critical,
        ] {
            let call = ToolCall {
                priority: priority.clone(),
                ..sample_tool_call()
            };
            let payload = call.into_payload().unwrap();
            let restored = ToolCall::from_payload(&payload).unwrap();
            assert_eq!(restored.priority, priority);
        }
    }

    #[test]
    fn invalid_payload_rejected() {
        assert!(ToolCall::from_payload(b"not valid json").is_err());
        assert!(ToolResult::from_payload(b"{broken").is_err());
    }

    #[test]
    fn failed_result_roundtrip() {
        let result = ToolResult {
            request_id: "req-fail".into(),
            tool_name: "ghl_contact".into(),
            success: false,
            output_json: String::new(),
            error: Some("GHL API returned 429".into()),
            metadata: HashMap::new(),
        };
        let payload = result.into_payload().unwrap();
        let restored = ToolResult::from_payload(&payload).unwrap();

        assert!(!restored.success);
        assert_eq!(restored.error.as_deref(), Some("GHL API returned 429"));
    }
}
