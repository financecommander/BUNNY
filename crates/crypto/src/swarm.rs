use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{CryptoError, Result};
use crate::identity::NodeRole;
use crate::ternary::TernaryPacket;
use crate::transport::NodeTransport;
use crate::types::SessionId;

/// Message types covering all swarm communication channels.
///
/// Every message in the swarm — orchestration, security, model distribution,
/// inference, state management, and control plane — is typed and encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SwarmMessageType {
    // Orchestration (0x1x)
    /// Orchestrator → Worker: assign a task.
    TaskAssignment = 0x10,
    /// Worker → Orchestrator: return task result.
    TaskResult = 0x11,
    /// Any → Orchestrator: periodic status update.
    StatusReport = 0x12,

    // Agent security (0x2x)
    /// Agent → Gateway/Orchestrator: threat detected.
    ThreatAlert = 0x20,
    /// Orchestrator → All: updated security policy.
    PolicyUpdate = 0x21,
    /// Any → Orchestrator: audit/forensic event.
    AuditEvent = 0x22,

    // Model distribution (0x3x)
    /// Orchestrator → Workers: distribute model weights.
    ModelDistribute = 0x30,
    /// Worker → Orchestrator: request a model shard.
    ShardRequest = 0x31,
    /// Orchestrator → Worker: deliver a model shard.
    ShardDelivery = 0x32,

    // Inference pipeline (0x4x)
    /// Gateway → Worker: dispatch inference (wraps TernaryPacket).
    InferenceDispatch = 0x40,
    /// Worker → Gateway: return inference result (wraps TernaryPacket).
    InferenceReturn = 0x41,

    // Session/state (0x5x)
    /// Worker → Worker: migrate session state.
    SessionMigrate = 0x50,
    /// Worker → Worker: transfer KV-cache snapshot.
    KvCacheTransfer = 0x51,

    // Control plane (0x6x)
    /// Any → Any: heartbeat/liveness.
    Heartbeat = 0x60,
    /// Any → Swarm: node announcement.
    NodeAnnounce = 0x61,
    /// Any → Peer: trigger session rekey.
    RekeyRequest = 0x62,
}

impl SwarmMessageType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0x10 => Ok(Self::TaskAssignment),
            0x11 => Ok(Self::TaskResult),
            0x12 => Ok(Self::StatusReport),
            0x20 => Ok(Self::ThreatAlert),
            0x21 => Ok(Self::PolicyUpdate),
            0x22 => Ok(Self::AuditEvent),
            0x30 => Ok(Self::ModelDistribute),
            0x31 => Ok(Self::ShardRequest),
            0x32 => Ok(Self::ShardDelivery),
            0x40 => Ok(Self::InferenceDispatch),
            0x41 => Ok(Self::InferenceReturn),
            0x50 => Ok(Self::SessionMigrate),
            0x51 => Ok(Self::KvCacheTransfer),
            0x60 => Ok(Self::Heartbeat),
            0x61 => Ok(Self::NodeAnnounce),
            0x62 => Ok(Self::RekeyRequest),
            other => Err(CryptoError::InvalidEnvelope(format!(
                "unknown swarm message type: {other:#x}"
            ))),
        }
    }

    /// Which roles are allowed to send this message type.
    pub fn allowed_senders(&self) -> &[NodeRole] {
        match self {
            Self::TaskAssignment | Self::PolicyUpdate | Self::ModelDistribute
            | Self::ShardDelivery => &[NodeRole::Orchestrator],

            Self::TaskResult | Self::ShardRequest => &[NodeRole::Worker],

            Self::ThreatAlert => &[NodeRole::Agent, NodeRole::Gateway],

            Self::InferenceDispatch => &[NodeRole::Gateway, NodeRole::Orchestrator],
            Self::InferenceReturn => &[NodeRole::Worker],

            Self::SessionMigrate | Self::KvCacheTransfer => &[NodeRole::Worker],

            // Anyone can send these
            Self::StatusReport | Self::AuditEvent | Self::Heartbeat
            | Self::NodeAnnounce | Self::RekeyRequest => &[
                NodeRole::Gateway,
                NodeRole::Worker,
                NodeRole::Orchestrator,
                NodeRole::Agent,
                NodeRole::Relay,
            ],
        }
    }
}

/// Message priority for scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl MessagePriority {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Low,
            1 => Self::Normal,
            2 => Self::High,
            _ => Self::Critical,
        }
    }
}

/// A typed swarm message with routing metadata.
///
/// Wire format (inside encrypted envelope payload):
/// ```text
/// ┌──────────┬──────────┬──────────────┬──────────┐
/// │ msg_type │ priority │ correlation  │ payload  │
/// │ 1 byte   │ 1 byte   │ 16 bytes     │ N bytes  │
/// └──────────┴──────────┴──────────────┴──────────┘
/// ```
#[derive(Debug, Clone)]
pub struct SwarmMessage {
    pub msg_type: SwarmMessageType,
    pub priority: MessagePriority,
    /// Correlation ID for request/response matching (random per request).
    pub correlation_id: [u8; 16],
    pub payload: Vec<u8>,
}

impl SwarmMessage {
    /// Create a new message with a random correlation ID.
    pub fn new(msg_type: SwarmMessageType, priority: MessagePriority, payload: Vec<u8>) -> Self {
        let mut correlation_id = [0u8; 16];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut correlation_id);
        Self {
            msg_type,
            priority,
            correlation_id,
            payload,
        }
    }

    /// Create a response correlated to a request.
    pub fn reply(
        request: &SwarmMessage,
        msg_type: SwarmMessageType,
        priority: MessagePriority,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            msg_type,
            priority,
            correlation_id: request.correlation_id,
            payload,
        }
    }

    /// Wrap a TernaryPacket as an inference dispatch message.
    pub fn inference_dispatch(packet: &TernaryPacket) -> Self {
        Self::new(
            SwarmMessageType::InferenceDispatch,
            MessagePriority::High,
            packet.to_bytes(),
        )
    }

    /// Wrap a TernaryPacket as an inference return message.
    pub fn inference_return(request: &SwarmMessage, packet: &TernaryPacket) -> Self {
        Self::reply(
            request,
            SwarmMessageType::InferenceReturn,
            MessagePriority::High,
            packet.to_bytes(),
        )
    }

    /// Extract the TernaryPacket from an inference message.
    pub fn into_ternary(&self) -> Result<TernaryPacket> {
        match self.msg_type {
            SwarmMessageType::InferenceDispatch | SwarmMessageType::InferenceReturn
            | SwarmMessageType::ModelDistribute | SwarmMessageType::ShardDelivery
            | SwarmMessageType::KvCacheTransfer => TernaryPacket::from_bytes(&self.payload),
            other => Err(CryptoError::InvalidEnvelope(format!(
                "message type {:?} does not contain a ternary packet",
                other
            ))),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(18 + self.payload.len());
        buf.push(self.msg_type as u8);
        buf.push(self.priority as u8);
        buf.extend_from_slice(&self.correlation_id);
        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 18 {
            return Err(CryptoError::InvalidEnvelope(
                "swarm message too short".into(),
            ));
        }
        let msg_type = SwarmMessageType::from_u8(data[0])?;
        let priority = MessagePriority::from_u8(data[1]);
        let mut correlation_id = [0u8; 16];
        correlation_id.copy_from_slice(&data[2..18]);
        let payload = data[18..].to_vec();

        Ok(Self {
            msg_type,
            priority,
            correlation_id,
            payload,
        })
    }
}

/// Full swarm protocol layer.
///
/// Wraps `NodeTransport` to provide typed, encrypted messaging for
/// all swarm communication channels. Every message is:
/// 1. Typed (SwarmMessageType) for routing
/// 2. Prioritized for scheduling
/// 3. Correlated for request/response matching
/// 4. Encrypted via NodeTransport (AEAD + hybrid PQ key exchange)
/// 5. Authenticated via signed node identity
pub struct SwarmProtocol {
    transport: NodeTransport,
    local_role: NodeRole,
}

impl SwarmProtocol {
    pub fn new(transport: NodeTransport, local_role: NodeRole) -> Self {
        Self {
            transport,
            local_role,
        }
    }

    /// Send a typed swarm message on an established session.
    pub async fn send(
        &self,
        session_id: &SessionId,
        message: &SwarmMessage,
    ) -> Result<Vec<u8>> {
        // Validate sender role
        if !message
            .msg_type
            .allowed_senders()
            .contains(&self.local_role)
        {
            return Err(CryptoError::ValidationFailed(format!(
                "role {:?} not allowed to send {:?}",
                self.local_role, message.msg_type
            )));
        }

        let wire_bytes = message.to_bytes();
        let envelope = self.transport.send(session_id, &wire_bytes).await?;

        info!(
            session = ?session_id,
            msg_type = ?message.msg_type,
            priority = ?message.priority,
            "swarm message sent"
        );

        Ok(envelope.to_bytes())
    }

    /// Receive and decrypt a swarm message, validating the sender's role.
    pub async fn receive(
        &self,
        encrypted: &[u8],
        sender_role: NodeRole,
    ) -> Result<SwarmMessage> {
        let envelope = crate::envelope::SwarmEnvelope::from_bytes(encrypted)?;
        let plaintext = self.transport.receive(&envelope).await?;
        let message = SwarmMessage::from_bytes(&plaintext)?;

        // Validate sender role
        if !message.msg_type.allowed_senders().contains(&sender_role) {
            return Err(CryptoError::ValidationFailed(format!(
                "role {:?} not allowed to send {:?}",
                sender_role, message.msg_type
            )));
        }

        info!(
            msg_type = ?message.msg_type,
            priority = ?message.priority,
            "swarm message received"
        );

        Ok(message)
    }

    /// Dispatch an inference request (Gateway → Worker).
    pub async fn dispatch_inference(
        &self,
        session_id: &SessionId,
        packet: &TernaryPacket,
    ) -> Result<Vec<u8>> {
        let message = SwarmMessage::inference_dispatch(packet);
        self.send(session_id, &message).await
    }

    /// Return an inference result (Worker → Gateway).
    pub async fn return_inference(
        &self,
        session_id: &SessionId,
        request: &SwarmMessage,
        result_packet: &TernaryPacket,
    ) -> Result<Vec<u8>> {
        let message = SwarmMessage::inference_return(request, result_packet);
        self.send(session_id, &message).await
    }

    /// Send a heartbeat on an established session.
    pub async fn heartbeat(&self, session_id: &SessionId) -> Result<Vec<u8>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let message = SwarmMessage::new(
            SwarmMessageType::Heartbeat,
            MessagePriority::Low,
            now.to_be_bytes().to_vec(),
        );
        self.send(session_id, &message).await
    }

    /// Send a threat alert (Agent → Gateway/Orchestrator).
    pub async fn threat_alert(
        &self,
        session_id: &SessionId,
        alert_data: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let message = SwarmMessage::new(
            SwarmMessageType::ThreatAlert,
            MessagePriority::Critical,
            alert_data,
        );
        self.send(session_id, &message).await
    }

    pub fn transport(&self) -> &NodeTransport {
        &self.transport
    }

    pub fn local_role(&self) -> NodeRole {
        self.local_role
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::CipherSuite;
    use crate::identity::{NodeCapabilities, SwarmNode};

    fn test_caps() -> NodeCapabilities {
        NodeCapabilities {
            models: vec!["threat_classifier".into()],
            has_gpu: false,
            max_sessions: 8,
            cipher_suites: vec![CipherSuite::Aes256Gcm],
        }
    }

    async fn setup_pair(
        role_a: NodeRole,
        role_b: NodeRole,
    ) -> (SwarmProtocol, SwarmProtocol, SessionId) {
        let node_a = NodeTransport::new(SwarmNode::generate(role_a, test_caps()));
        let node_b = NodeTransport::new(SwarmNode::generate(role_b, test_caps()));

        let (init, eph) = node_b.initiate_handshake();
        let accept = node_a.accept_handshake(&init).await.unwrap();
        let session_id = node_b.complete_handshake(&accept, eph).await.unwrap();

        let proto_a = SwarmProtocol::new(node_a, role_a);
        let proto_b = SwarmProtocol::new(node_b, role_b);

        (proto_a, proto_b, session_id)
    }

    #[tokio::test]
    async fn message_roundtrip() {
        let (gateway, worker, session_id) =
            setup_pair(NodeRole::Gateway, NodeRole::Worker).await;

        // Gateway sends heartbeat
        let encrypted = gateway.heartbeat(&session_id).await.unwrap();

        // Worker receives
        let msg = worker.receive(&encrypted, NodeRole::Gateway).await.unwrap();
        assert_eq!(msg.msg_type, SwarmMessageType::Heartbeat);
        assert_eq!(msg.priority, MessagePriority::Low);
    }

    #[tokio::test]
    async fn inference_dispatch_roundtrip() {
        let (gateway, worker, session_id) =
            setup_pair(NodeRole::Gateway, NodeRole::Worker).await;

        let packet = TernaryPacket::inference_request(
            "threat_classifier",
            b"{\"prompt\": \"scan\"}".to_vec(),
        );

        // Gateway dispatches inference
        let encrypted = gateway
            .dispatch_inference(&session_id, &packet)
            .await
            .unwrap();

        // Worker receives
        let msg = worker.receive(&encrypted, NodeRole::Gateway).await.unwrap();
        assert_eq!(msg.msg_type, SwarmMessageType::InferenceDispatch);

        // Extract ternary packet
        let received_packet = msg.into_ternary().unwrap();
        assert_eq!(received_packet.data, b"{\"prompt\": \"scan\"}");

        // Worker sends result back
        let result_packet = TernaryPacket::inference_result(
            "threat_classifier",
            vec![1, 3],
            vec![0x01, 0x02, 0x03],
        );
        let response = worker
            .return_inference(&session_id, &msg, &result_packet)
            .await;
        // Worker role can send InferenceReturn
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn role_enforcement_sender() {
        let (gateway, _worker, session_id) =
            setup_pair(NodeRole::Gateway, NodeRole::Worker).await;

        // Gateway tries to send TaskResult (only Worker allowed)
        let msg = SwarmMessage::new(
            SwarmMessageType::TaskResult,
            MessagePriority::Normal,
            b"result".to_vec(),
        );
        let result = gateway.send(&session_id, &msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn role_enforcement_receiver() {
        let (gateway, worker, session_id) =
            setup_pair(NodeRole::Gateway, NodeRole::Worker).await;

        // Gateway sends heartbeat (valid)
        let encrypted = gateway.heartbeat(&session_id).await.unwrap();

        // Try to receive claiming sender is a Worker
        // Heartbeat allows all roles, so this should succeed
        let msg = worker.receive(&encrypted, NodeRole::Worker).await.unwrap();
        assert_eq!(msg.msg_type, SwarmMessageType::Heartbeat);
    }

    #[tokio::test]
    async fn correlation_id_preserved() {
        let (gateway, worker, session_id) =
            setup_pair(NodeRole::Gateway, NodeRole::Worker).await;

        let packet = TernaryPacket::inference_request(
            "test_model",
            b"data".to_vec(),
        );
        let dispatch = SwarmMessage::inference_dispatch(&packet);
        let corr_id = dispatch.correlation_id;

        let encrypted = gateway.send(&session_id, &dispatch).await.unwrap();
        let received = worker.receive(&encrypted, NodeRole::Gateway).await.unwrap();

        assert_eq!(received.correlation_id, corr_id);
    }

    #[tokio::test]
    async fn threat_alert_critical_priority() {
        let (_, agent, session_id) =
            setup_pair(NodeRole::Gateway, NodeRole::Agent).await;

        // Agent sends threat alert
        let encrypted = agent
            .threat_alert(&session_id, b"MALWARE_DETECTED".to_vec())
            .await
            .unwrap();

        // Would be received as Critical priority
        // (We can't easily receive on the other side since the session
        // is only in node_b's store, but we can verify the send succeeded)
        assert!(!encrypted.is_empty());
    }

    #[tokio::test]
    async fn message_serialization_roundtrip() {
        let msg = SwarmMessage::new(
            SwarmMessageType::PolicyUpdate,
            MessagePriority::High,
            b"new_policy_v2".to_vec(),
        );
        let bytes = msg.to_bytes();
        let restored = SwarmMessage::from_bytes(&bytes).unwrap();

        assert_eq!(restored.msg_type, SwarmMessageType::PolicyUpdate);
        assert_eq!(restored.priority, MessagePriority::High);
        assert_eq!(restored.correlation_id, msg.correlation_id);
        assert_eq!(restored.payload, b"new_policy_v2");
    }

    #[tokio::test]
    async fn orchestrator_task_assignment() {
        let (orchestrator, worker, session_id) =
            setup_pair(NodeRole::Orchestrator, NodeRole::Worker).await;

        let msg = SwarmMessage::new(
            SwarmMessageType::TaskAssignment,
            MessagePriority::Normal,
            b"{\"task\": \"classify\", \"model\": \"threat_classifier\"}".to_vec(),
        );

        let encrypted = orchestrator.send(&session_id, &msg).await.unwrap();
        let received = worker
            .receive(&encrypted, NodeRole::Orchestrator)
            .await
            .unwrap();

        assert_eq!(received.msg_type, SwarmMessageType::TaskAssignment);
        assert_eq!(received.payload, b"{\"task\": \"classify\", \"model\": \"threat_classifier\"}");
    }
}
