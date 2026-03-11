//! Tool dispatch over encrypted channels — the OpenClaw integration seam.
//!
//! Pushes signed tool calls through [`EncryptedChannel`], verifies on
//! receipt, and returns signed results. This is the exact boundary where
//! OpenClaw (Python) meets BUNNY (Rust) encrypted transport.
//!
//! ```text
//! OpenClaw (Python)
//!   → ToolCall → sign → seal → EncryptedChannel → remote sidecar
//!   ← ToolResult ← verify ← open ← EncryptedChannel ← remote sidecar
//! ```

use serde::{Deserialize, Serialize};

use crate::error::{NetworkError, Result};
use crate::proxy::channel::{EncryptedChannel, SealedMessage};
use crate::proxy::envelope_sign::{RequestSigner, RequestVerifier, SignedRequest};
use crate::proxy::tool_call::{verify_tool_call, verify_tool_result, ToolCall, ToolResult};

use bunny_crypto::identity::NodeIdentity;

/// Well-known path for tool execution requests.
pub const TOOL_CALL_PATH: &str = "/openclaw/tool/execute";
/// Well-known path for tool execution results.
pub const TOOL_RESULT_PATH: &str = "/openclaw/tool/result";

/// Envelope wrapping a SignedRequest for transport over EncryptedChannel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProxyEnvelope {
    SignedRequest(SignedRequest),
}

impl ProxyEnvelope {
    /// Serialize to bytes for sealing.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| NetworkError::Serialization(e.to_string()))
    }

    /// Deserialize from bytes after opening.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data)
            .map_err(|e| NetworkError::Serialization(e.to_string()))
    }
}

/// Seal a tool call into an encrypted envelope for transport.
///
/// Flow: ToolCall → sign → ProxyEnvelope → seal → SealedMessage
pub async fn seal_tool_call(
    channel: &EncryptedChannel,
    signer: &mut RequestSigner,
    call: &ToolCall,
) -> Result<SealedMessage> {
    let signed = call.sign_for_path(signer, TOOL_CALL_PATH)?;
    let envelope_bytes = ProxyEnvelope::SignedRequest(signed).to_bytes()?;
    channel.seal(&envelope_bytes).await
}

/// Open and verify a tool call from a received sealed message.
///
/// Flow: SealedMessage → open → ProxyEnvelope → verify → ToolCall
pub async fn open_tool_call(
    channel: &EncryptedChannel,
    verifier: &mut RequestVerifier,
    peer: &NodeIdentity,
    sealed: &SealedMessage,
) -> Result<ToolCall> {
    let plaintext = match sealed {
        SealedMessage::Standard(env) => channel.open_standard(env).await?,
        SealedMessage::Cloaked(env) => channel.open_cloaked(env).await?,
    };
    let envelope = ProxyEnvelope::from_bytes(&plaintext)?;
    match envelope {
        ProxyEnvelope::SignedRequest(req) => {
            verify_tool_call(verifier, &req, peer, TOOL_CALL_PATH)
        }
    }
}

/// Seal a tool result into an encrypted envelope for transport.
///
/// Flow: ToolResult → sign → ProxyEnvelope → seal → SealedMessage
pub async fn seal_tool_result(
    channel: &EncryptedChannel,
    signer: &mut RequestSigner,
    result: &ToolResult,
) -> Result<SealedMessage> {
    let signed = result.sign_for_path(signer, TOOL_RESULT_PATH)?;
    let envelope_bytes = ProxyEnvelope::SignedRequest(signed).to_bytes()?;
    channel.seal(&envelope_bytes).await
}

/// Open and verify a tool result from a received sealed message.
///
/// Flow: SealedMessage → open → ProxyEnvelope → verify → ToolResult
pub async fn open_tool_result(
    channel: &EncryptedChannel,
    verifier: &mut RequestVerifier,
    peer: &NodeIdentity,
    sealed: &SealedMessage,
) -> Result<ToolResult> {
    let plaintext = match sealed {
        SealedMessage::Standard(env) => channel.open_standard(env).await?,
        SealedMessage::Cloaked(env) => channel.open_cloaked(env).await?,
    };
    let envelope = ProxyEnvelope::from_bytes(&plaintext)?;
    match envelope {
        ProxyEnvelope::SignedRequest(req) => {
            verify_tool_result(verifier, &req, peer, TOOL_RESULT_PATH)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::channel::ChannelConfig;
    use crate::proxy::tool_call::ToolCallPriority;
    use bunny_crypto::cipher::CipherSuite;
    use bunny_crypto::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use bunny_crypto::transport::NodeTransport;
    use bunny_crypto::types::AgentId;
    use std::collections::HashMap;
    use std::sync::Arc;

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

    /// Set up a pair of established encrypted channels (Portal ↔ Coordinator).
    async fn setup_channels() -> (
        EncryptedChannel,
        EncryptedChannel,
        SwarmNode,
        SwarmNode,
    ) {
        let portal_node = make_node(NodeRole::Gateway);
        let coord_node = make_node(NodeRole::Orchestrator);

        let portal_transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Gateway, NodeCapabilities {
                models: vec![], has_gpu: false, max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));
        let coord_transport = Arc::new(NodeTransport::new(
            SwarmNode::generate(NodeRole::Orchestrator, NodeCapabilities {
                models: vec![], has_gpu: false, max_sessions: 100,
                cipher_suites: vec![CipherSuite::Aes256Gcm],
            }),
        ));

        let portal_ch = EncryptedChannel::new(portal_transport, ChannelConfig::performance());
        let coord_ch = EncryptedChannel::new(coord_transport, ChannelConfig::performance());

        // Full handshake
        let init = portal_ch.initiate().await.unwrap();
        let accept = coord_ch.accept(&init).await.unwrap();
        portal_ch.complete(&accept).await.unwrap();

        (portal_ch, coord_ch, portal_node, coord_node)
    }

    fn sample_call() -> ToolCall {
        ToolCall {
            tool_name: "send_email".into(),
            args_json: r#"{"to_email":"bob@example.com"}"#.into(),
            request_id: "req-dispatch-001".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Standard,
            metadata: HashMap::from([("channel".into(), "slack".into())]),
        }
    }

    fn sample_result(request_id: &str) -> ToolResult {
        ToolResult {
            request_id: request_id.into(),
            tool_name: "send_email".into(),
            success: true,
            output_json: r#"{"sent":true}"#.into(),
            error: None,
            metadata: HashMap::from([("node".into(), "swarm-mainframe".into())]),
        }
    }

    // ── Seal + Open Tool Call ────────────────────────────────────────

    #[tokio::test]
    async fn seal_and_open_tool_call() {
        let (portal_ch, coord_ch, portal_node, _coord_node) = setup_channels().await;

        let portal_identity = portal_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(portal_node);
        let mut verifier = RequestVerifier::new(60_000);

        let call = sample_call();

        // Portal seals a tool call
        let sealed = seal_tool_call(&portal_ch, &mut signer, &call).await.unwrap();

        // Coordinator opens and verifies it
        let verified = open_tool_call(
            &coord_ch,
            &mut verifier,
            &portal_identity,
            &sealed,
        )
        .await
        .unwrap();

        assert_eq!(verified.tool_name, "send_email");
        assert_eq!(verified.request_id, "req-dispatch-001");
        assert_eq!(verified.metadata.get("channel").unwrap(), "slack");
    }

    // ── Seal + Open Tool Result ─────────────────────────────────────

    #[tokio::test]
    async fn seal_and_open_tool_result() {
        let (portal_ch, coord_ch, _portal_node, coord_node) = setup_channels().await;

        let coord_identity = coord_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(coord_node);
        let mut verifier = RequestVerifier::new(60_000);

        let result = sample_result("req-dispatch-001");

        // Coordinator seals a result
        let sealed = seal_tool_result(&coord_ch, &mut signer, &result)
            .await
            .unwrap();

        // Portal opens and verifies it
        let verified = open_tool_result(
            &portal_ch,
            &mut verifier,
            &coord_identity,
            &sealed,
        )
        .await
        .unwrap();

        assert_eq!(verified.request_id, "req-dispatch-001");
        assert!(verified.success);
        assert_eq!(verified.metadata.get("node").unwrap(), "swarm-mainframe");
    }

    // ── Tamper Rejection ────────────────────────────────────────────

    #[tokio::test]
    async fn tampered_sealed_message_rejected() {
        let (portal_ch, coord_ch, portal_node, _coord_node) = setup_channels().await;

        let portal_identity = portal_node.signed_announcement().identity.clone();
        let mut signer = RequestSigner::new(portal_node);
        let mut verifier = RequestVerifier::new(60_000);

        let call = sample_call();
        let sealed = seal_tool_call(&portal_ch, &mut signer, &call).await.unwrap();

        // Tamper with the sealed bytes
        let mut tampered_bytes = sealed.to_bytes();
        if let Some(last) = tampered_bytes.last_mut() {
            *last ^= 0xFF;
        }

        // Try to parse tampered bytes — should fail at deserialization or decryption
        let result = SealedMessage::from_bytes(&tampered_bytes);
        if let Ok(tampered_sealed) = result {
            let open_result = open_tool_call(
                &coord_ch,
                &mut verifier,
                &portal_identity,
                &tampered_sealed,
            )
            .await;
            assert!(open_result.is_err());
        }
        // If from_bytes fails, that's also correct — tampered data rejected
    }

    // ── Wrong Identity Rejection ────────────────────────────────────

    #[tokio::test]
    async fn wrong_identity_rejected() {
        let (portal_ch, coord_ch, portal_node, _coord_node) = setup_channels().await;

        // Use a different node's identity for verification
        let wrong_node = make_node(NodeRole::Worker);
        let wrong_identity = wrong_node.signed_announcement().identity.clone();

        let mut signer = RequestSigner::new(portal_node);
        let mut verifier = RequestVerifier::new(60_000);

        let call = sample_call();
        let sealed = seal_tool_call(&portal_ch, &mut signer, &call).await.unwrap();

        // Verify with wrong identity — should fail
        let result = open_tool_call(
            &coord_ch,
            &mut verifier,
            &wrong_identity,
            &sealed,
        )
        .await;
        assert!(result.is_err());
    }

    // ── ProxyEnvelope Roundtrip ─────────────────────────────────────

    #[test]
    fn proxy_envelope_roundtrip() {
        let node = make_node(NodeRole::Gateway);
        let mut signer = RequestSigner::new(node);

        let call = sample_call();
        let signed = call.sign_for_path(&mut signer, TOOL_CALL_PATH).unwrap();

        let envelope = ProxyEnvelope::SignedRequest(signed);
        let bytes = envelope.to_bytes().unwrap();
        let restored = ProxyEnvelope::from_bytes(&bytes).unwrap();

        match restored {
            ProxyEnvelope::SignedRequest(req) => {
                assert_eq!(req.target, TOOL_CALL_PATH);
                let payload = ToolCall::from_payload(&req.payload).unwrap();
                assert_eq!(payload.tool_name, "send_email");
            }
        }
    }

    // ── Constants ───────────────────────────────────────────────────

    #[test]
    fn well_known_paths() {
        assert_eq!(TOOL_CALL_PATH, "/openclaw/tool/execute");
        assert_eq!(TOOL_RESULT_PATH, "/openclaw/tool/result");
    }

    // ── Full Round-Trip: Call → Result ───────────────────────────────

    #[tokio::test]
    async fn full_tool_call_roundtrip() {
        let (portal_ch, coord_ch, portal_node, coord_node) = setup_channels().await;

        let portal_identity = portal_node.signed_announcement().identity.clone();
        let coord_identity = coord_node.signed_announcement().identity.clone();

        let mut portal_signer = RequestSigner::new(portal_node);
        let mut coord_signer = RequestSigner::new(coord_node);
        let mut portal_verifier = RequestVerifier::new(60_000);
        let mut coord_verifier = RequestVerifier::new(60_000);

        // 1. Portal sends tool call to coordinator
        let call = ToolCall {
            tool_name: "ghl_contact".into(),
            args_json: r#"{"email":"alice@corp.com","first_name":"Alice"}"#.into(),
            request_id: "req-roundtrip".into(),
            caller: AgentId::new(),
            priority: ToolCallPriority::Critical,
            metadata: HashMap::from([("channel".into(), "messaging".into())]),
        };

        let sealed_call = seal_tool_call(&portal_ch, &mut portal_signer, &call)
            .await
            .unwrap();

        // 2. Coordinator receives and verifies
        let received_call = open_tool_call(
            &coord_ch,
            &mut coord_verifier,
            &portal_identity,
            &sealed_call,
        )
        .await
        .unwrap();

        assert_eq!(received_call.tool_name, "ghl_contact");
        assert_eq!(received_call.request_id, "req-roundtrip");

        // 3. Coordinator sends result back
        let result = ToolResult {
            request_id: received_call.request_id.clone(),
            tool_name: received_call.tool_name.clone(),
            success: true,
            output_json: r#"{"contact_id":"ghl_12345"}"#.into(),
            error: None,
            metadata: HashMap::from([
                ("executor_node".into(), "swarm-mainframe".into()),
                ("execution_time_ms".into(), "890".into()),
            ]),
        };

        let sealed_result = seal_tool_result(&coord_ch, &mut coord_signer, &result)
            .await
            .unwrap();

        // 4. Portal receives and verifies result
        let received_result = open_tool_result(
            &portal_ch,
            &mut portal_verifier,
            &coord_identity,
            &sealed_result,
        )
        .await
        .unwrap();

        assert_eq!(received_result.request_id, "req-roundtrip");
        assert!(received_result.success);
        assert!(received_result.output_json.contains("ghl_12345"));
    }
}
