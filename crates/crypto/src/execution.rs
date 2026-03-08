use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::{CryptoError, Result};
use crate::identity::NodeRole;
use crate::ternary::{TernaryPacket, TernaryPayloadType};
use crate::transport::NodeTransport;
use crate::types::{AgentId, SessionId};

/// Security classification for ternary payloads.
///
/// Determines the minimum trust level and audit requirements
/// for processing each payload at the execution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityClass {
    /// Model weights/shards/checkpoints — highest sensitivity.
    /// Only Gateway and Orchestrator roles may handle.
    ModelWeight,
    /// Inference inputs (prompts, raw tensors) — high sensitivity.
    /// Requires authenticated session, logged.
    InferenceInput,
    /// Inference outputs (logits, verdicts) — high sensitivity.
    /// Re-encrypted before leaving execution boundary.
    InferenceOutput,
    /// KV-cache snapshots — medium sensitivity.
    /// Session state, restricted to same-session peers.
    SessionState,
    /// Control/routing messages — standard sensitivity.
    Control,
}

impl SecurityClass {
    pub fn from_payload_type(pt: TernaryPayloadType) -> Self {
        match pt {
            TernaryPayloadType::ModelShard | TernaryPayloadType::Checkpoint => Self::ModelWeight,
            TernaryPayloadType::Tensor | TernaryPayloadType::InferenceRequest => {
                Self::InferenceInput
            }
            TernaryPayloadType::InferenceResult => Self::InferenceOutput,
            TernaryPayloadType::KvCache => Self::SessionState,
            TernaryPayloadType::Control => Self::Control,
        }
    }

    /// Check whether a given node role is allowed to handle this security class.
    pub fn is_role_authorized(&self, role: NodeRole) -> bool {
        match self {
            Self::ModelWeight => matches!(role, NodeRole::Gateway | NodeRole::Orchestrator),
            Self::InferenceInput | Self::InferenceOutput => matches!(
                role,
                NodeRole::Gateway | NodeRole::Worker | NodeRole::Orchestrator
            ),
            Self::SessionState => matches!(
                role,
                NodeRole::Gateway | NodeRole::Worker | NodeRole::Orchestrator
            ),
            Self::Control => true, // all roles may send/receive control messages
        }
    }
}

/// A model routing decision produced at the execution boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    /// SHA-256 of the target model name.
    pub model_hash: [u8; 32],
    /// Worker node that should handle this request.
    pub target_worker: AgentId,
    /// Shard assignment if model is sharded.
    pub shard_assignment: Option<(u16, u16)>,
}

/// A validated, typed request after decryption at the execution boundary.
#[derive(Debug)]
pub struct ExecutionRequest {
    /// Session on which this request arrived.
    pub session_id: SessionId,
    /// Classified security level.
    pub security_class: SecurityClass,
    /// Parsed ternary packet with header + data.
    pub packet: TernaryPacket,
    /// Timestamp when the request was decrypted.
    pub received_at: u64,
}

/// The result of processing an execution request, ready for re-encryption.
pub struct ExecutionResponse {
    /// The response packet to encrypt and return.
    pub packet: TernaryPacket,
}

/// The Triton execution boundary.
///
/// This is the security perimeter where encrypted swarm traffic is decrypted,
/// validated against security class rules, routed to the inference runtime,
/// and results are re-encrypted before leaving the boundary.
///
/// Flow:
/// ```text
/// encrypted envelope → [ExecutionBoundary] → decrypt → parse TernaryPacket
///   → classify security → validate role authorization → route to model
///   → execute inference → wrap result as TernaryPacket → encrypt → return
/// ```
pub struct ExecutionBoundary {
    transport: NodeTransport,
}

impl ExecutionBoundary {
    pub fn new(transport: NodeTransport) -> Self {
        Self { transport }
    }

    /// Decrypt an incoming envelope and parse it as a ternary packet.
    ///
    /// Validates: session exists, AEAD authentication, ternary format,
    /// and security class authorization for the sender's role.
    pub async fn decrypt_request(
        &self,
        session_id: &SessionId,
        peer_role: NodeRole,
        encrypted_payload: &[u8],
    ) -> Result<ExecutionRequest> {
        // 1. Parse the encrypted payload as a SwarmEnvelope
        let envelope = crate::envelope::SwarmEnvelope::from_bytes(encrypted_payload)?;

        // 2. Decrypt through the transport layer (session lookup, replay, AEAD)
        let plaintext = self.transport.receive(&envelope).await?;

        // 3. Parse as TernaryPacket
        let packet = TernaryPacket::from_bytes(&plaintext)?;

        // 4. Classify security level
        let security_class = SecurityClass::from_payload_type(packet.header.payload_type);

        // 5. Validate role authorization
        if !security_class.is_role_authorized(peer_role) {
            warn!(
                role = ?peer_role,
                security_class = ?security_class,
                payload_type = ?packet.header.payload_type,
                "unauthorized payload type for role"
            );
            return Err(CryptoError::ValidationFailed(format!(
                "role {:?} not authorized for {:?} payloads",
                peer_role, security_class
            )));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        info!(
            session = ?session_id,
            payload_type = ?packet.header.payload_type,
            security_class = ?security_class,
            data_len = packet.data.len(),
            "request decrypted at execution boundary"
        );

        Ok(ExecutionRequest {
            session_id: session_id.clone(),
            security_class,
            packet,
            received_at: now,
        })
    }

    /// Encrypt a ternary response packet for return through the swarm.
    pub async fn encrypt_response(
        &self,
        session_id: &SessionId,
        response: &ExecutionResponse,
    ) -> Result<Vec<u8>> {
        let payload_bytes = response.packet.to_bytes();
        let envelope = self.transport.send(session_id, &payload_bytes).await?;

        info!(
            session = ?session_id,
            payload_type = ?response.packet.header.payload_type,
            "response encrypted at execution boundary"
        );

        Ok(envelope.to_bytes())
    }

    /// Full execution boundary flow:
    /// decrypt → classify → authorize → process → encrypt.
    ///
    /// The `process_fn` receives the decrypted `ExecutionRequest` and returns
    /// an `ExecutionResponse` (or error). The result is re-encrypted.
    pub async fn process<F, Fut>(
        &self,
        session_id: &SessionId,
        peer_role: NodeRole,
        encrypted_payload: &[u8],
        process_fn: F,
    ) -> Result<Vec<u8>>
    where
        F: FnOnce(ExecutionRequest) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<ExecutionResponse, CryptoError>>,
    {
        let request = self
            .decrypt_request(session_id, peer_role, encrypted_payload)
            .await?;

        let response = process_fn(request).await?;

        self.encrypt_response(session_id, &response).await
    }

    /// Send a ternary packet to a peer on an established session.
    pub async fn send_ternary(
        &self,
        session_id: &SessionId,
        packet: &TernaryPacket,
    ) -> Result<Vec<u8>> {
        let payload_bytes = packet.to_bytes();
        let envelope = self.transport.send(session_id, &payload_bytes).await?;
        Ok(envelope.to_bytes())
    }

    /// Receive and decrypt a ternary packet from a peer.
    pub async fn receive_ternary(
        &self,
        encrypted_payload: &[u8],
    ) -> Result<TernaryPacket> {
        let envelope = crate::envelope::SwarmEnvelope::from_bytes(encrypted_payload)?;
        let plaintext = self.transport.receive(&envelope).await?;
        TernaryPacket::from_bytes(&plaintext)
    }

    /// Route an inference request to the appropriate worker based on model hash.
    ///
    /// The `resolver` function maps a model hash to the target worker.
    pub fn route_model<F>(
        &self,
        request: &ExecutionRequest,
        resolver: F,
    ) -> Result<ModelRoute>
    where
        F: FnOnce(&[u8; 32]) -> Option<AgentId>,
    {
        let model_hash = request.packet.header.model_hash;
        let target = resolver(&model_hash).ok_or_else(|| {
            CryptoError::GatewayRoutingError(format!(
                "no worker found for model hash {:x?}",
                &model_hash[..8]
            ))
        })?;

        let shard_assignment = if request.packet.header.total_shards > 1 {
            Some((
                request.packet.header.shard_index,
                request.packet.header.total_shards,
            ))
        } else {
            None
        };

        info!(
            target = ?target,
            model_hash = ?&model_hash[..8],
            shard = ?shard_assignment,
            "model routed at execution boundary"
        );

        Ok(ModelRoute {
            model_hash,
            target_worker: target,
            shard_assignment,
        })
    }

    pub fn transport(&self) -> &NodeTransport {
        &self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::CipherSuite;
    use crate::identity::{NodeCapabilities, NodeRole, SwarmNode};
    use crate::ternary::TernaryPacket;
    use crate::transport::NodeTransport;

    fn test_caps() -> NodeCapabilities {
        NodeCapabilities {
            models: vec!["threat_classifier".into()],
            has_gpu: false,
            max_sessions: 8,
            cipher_suites: vec![CipherSuite::Aes256Gcm],
        }
    }

    /// Helper: establish a session between two nodes, return (boundary_a, boundary_b, session_id).
    async fn setup_pair() -> (ExecutionBoundary, ExecutionBoundary, SessionId) {
        let node_a = NodeTransport::new(SwarmNode::generate(NodeRole::Gateway, test_caps()));
        let node_b = NodeTransport::new(SwarmNode::generate(NodeRole::Worker, test_caps()));

        let (init, eph) = node_b.initiate_handshake();
        let accept = node_a.accept_handshake(&init).await.unwrap();
        let session_id = node_b.complete_handshake(&accept, eph).await.unwrap();

        let boundary_a = ExecutionBoundary::new(node_a);
        let boundary_b = ExecutionBoundary::new(node_b);

        (boundary_a, boundary_b, session_id)
    }

    #[tokio::test]
    async fn ternary_send_receive_through_boundary() {
        let (gateway, worker, session_id) = setup_pair().await;

        // Worker sends an inference request
        let request = TernaryPacket::inference_request(
            "threat_classifier",
            b"{\"prompt\": \"analyze traffic\"}".to_vec(),
        );
        let encrypted = worker.send_ternary(&session_id, &request).await.unwrap();

        // Gateway receives and decrypts
        let decrypted = gateway.receive_ternary(&encrypted).await.unwrap();
        assert_eq!(
            decrypted.header.payload_type,
            TernaryPayloadType::InferenceRequest
        );
        assert_eq!(decrypted.data, b"{\"prompt\": \"analyze traffic\"}");
    }

    #[tokio::test]
    async fn full_execution_boundary_flow() {
        let (gateway, worker, session_id) = setup_pair().await;

        // Worker sends tensor data
        let tensor = TernaryPacket::tensor("threat_classifier", vec![1, 128], vec![0xAA; 64]);
        let encrypted = worker.send_ternary(&session_id, &tensor).await.unwrap();

        // Gateway processes through execution boundary
        let result = gateway
            .process(
                &session_id,
                NodeRole::Worker,
                &encrypted,
                |req| async move {
                    assert_eq!(req.security_class, SecurityClass::InferenceInput);
                    assert_eq!(
                        req.packet.header.payload_type,
                        TernaryPayloadType::Tensor
                    );

                    // Simulate inference — return result
                    Ok(ExecutionResponse {
                        packet: TernaryPacket::inference_result(
                            "threat_classifier",
                            vec![1, 3],
                            vec![0x01, 0x02, 0x03], // logits
                        ),
                    })
                },
            )
            .await
            .unwrap();

        // Worker decrypts the response
        let response = worker.receive_ternary(&result).await.unwrap();
        assert_eq!(
            response.header.payload_type,
            TernaryPayloadType::InferenceResult
        );
        assert_eq!(response.header.dimensions, vec![1, 3]);
        assert_eq!(response.data, vec![0x01, 0x02, 0x03]);
    }

    #[tokio::test]
    async fn security_class_authorization() {
        // ModelWeight payloads require Gateway or Orchestrator role
        assert!(SecurityClass::ModelWeight.is_role_authorized(NodeRole::Gateway));
        assert!(SecurityClass::ModelWeight.is_role_authorized(NodeRole::Orchestrator));
        assert!(!SecurityClass::ModelWeight.is_role_authorized(NodeRole::Worker));
        assert!(!SecurityClass::ModelWeight.is_role_authorized(NodeRole::Agent));
        assert!(!SecurityClass::ModelWeight.is_role_authorized(NodeRole::Relay));

        // InferenceInput allows Gateway, Worker, Orchestrator
        assert!(SecurityClass::InferenceInput.is_role_authorized(NodeRole::Worker));
        assert!(!SecurityClass::InferenceInput.is_role_authorized(NodeRole::Relay));

        // Control allows all roles
        assert!(SecurityClass::Control.is_role_authorized(NodeRole::Relay));
        assert!(SecurityClass::Control.is_role_authorized(NodeRole::Agent));
    }

    #[tokio::test]
    async fn unauthorized_role_rejected() {
        let (gateway, worker, session_id) = setup_pair().await;

        // Worker sends a model shard (ModelWeight class)
        let shard = TernaryPacket::model_shard("secret_model", 0, 4, vec![0xFF; 256]);
        let encrypted = worker.send_ternary(&session_id, &shard).await.unwrap();

        // Gateway tries to process it claiming the sender is a Relay
        // (Relay is NOT authorized for ModelWeight payloads)
        let result = gateway
            .decrypt_request(&session_id, NodeRole::Relay, &encrypted)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CryptoError::ValidationFailed(_)));
    }

    #[tokio::test]
    async fn model_routing() {
        let (gateway, worker, session_id) = setup_pair().await;

        // Worker sends inference request
        let request = TernaryPacket::inference_request(
            "threat_classifier",
            b"route me".to_vec(),
        );
        let encrypted = worker.send_ternary(&session_id, &request).await.unwrap();

        let exec_req = gateway
            .decrypt_request(&session_id, NodeRole::Worker, &encrypted)
            .await
            .unwrap();

        // Route based on model hash
        let expected_hash = exec_req.packet.header.model_hash;
        let target_worker = AgentId::new();
        let target_clone = target_worker.clone();

        let route = gateway
            .route_model(&exec_req, |hash| {
                if *hash == expected_hash {
                    Some(target_clone.clone())
                } else {
                    None
                }
            })
            .unwrap();

        assert_eq!(route.target_worker, target_worker);
        assert_eq!(route.model_hash, expected_hash);
        assert!(route.shard_assignment.is_none());
    }

    #[tokio::test]
    async fn model_routing_unknown_model() {
        let (gateway, worker, session_id) = setup_pair().await;

        let request = TernaryPacket::inference_request(
            "unknown_model",
            b"data".to_vec(),
        );
        let encrypted = worker.send_ternary(&session_id, &request).await.unwrap();

        let exec_req = gateway
            .decrypt_request(&session_id, NodeRole::Worker, &encrypted)
            .await
            .unwrap();

        // No resolver matches — should fail
        let result = gateway.route_model(&exec_req, |_| None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CryptoError::GatewayRoutingError(_)));
    }

    #[tokio::test]
    async fn sharded_model_routing() {
        let (gateway, worker, session_id) = setup_pair().await;

        // Worker sends a model shard
        let shard = TernaryPacket::model_shard("big_model", 3, 8, vec![0xBB; 512]);
        let encrypted = worker.send_ternary(&session_id, &shard).await.unwrap();

        // Process as Orchestrator (authorized for ModelWeight)
        let exec_req = gateway
            .decrypt_request(&session_id, NodeRole::Orchestrator, &encrypted)
            .await
            .unwrap();

        assert_eq!(exec_req.security_class, SecurityClass::ModelWeight);

        let target = AgentId::new();
        let target_clone = target.clone();
        let route = gateway
            .route_model(&exec_req, |_| Some(target_clone.clone()))
            .unwrap();

        assert_eq!(route.shard_assignment, Some((3, 8)));
    }

    #[tokio::test]
    async fn control_message_through_boundary() {
        let (gateway, worker, session_id) = setup_pair().await;

        let ctrl = TernaryPacket::control(b"REBALANCE_SHARDS".to_vec());
        let encrypted = worker.send_ternary(&session_id, &ctrl).await.unwrap();

        // Any role can send control messages
        let exec_req = gateway
            .decrypt_request(&session_id, NodeRole::Relay, &encrypted)
            .await
            .unwrap();

        assert_eq!(exec_req.security_class, SecurityClass::Control);
        assert_eq!(exec_req.packet.data, b"REBALANCE_SHARDS");
    }
}
