use serde::{Deserialize, Serialize};

use crate::cipher::SwarmCipher;
use crate::envelope::SwarmEnvelope;
use crate::error::{CryptoError, Result};
use crate::types::{AgentId, SessionId};

/// Ternary payload type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TernaryPayloadType {
    /// 2-bit packed ternary tensor data (inference input/output).
    Tensor = 0x01,
    /// Ternary model weight shard.
    ModelShard = 0x02,
    /// KV-cache snapshot for session continuity.
    KvCache = 0x03,
    /// Model checkpoint for federated learning.
    Checkpoint = 0x04,
    /// Compressed control message (model routing, scheduling).
    Control = 0x05,
    /// Inference request (prompt + model selection).
    InferenceRequest = 0x06,
    /// Inference result (logits, verdicts).
    InferenceResult = 0x07,
}

impl TernaryPayloadType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0x01 => Ok(Self::Tensor),
            0x02 => Ok(Self::ModelShard),
            0x03 => Ok(Self::KvCache),
            0x04 => Ok(Self::Checkpoint),
            0x05 => Ok(Self::Control),
            0x06 => Ok(Self::InferenceRequest),
            0x07 => Ok(Self::InferenceResult),
            other => Err(CryptoError::InvalidEnvelope(format!(
                "unknown ternary payload type: {other:#x}"
            ))),
        }
    }
}

/// Header for ternary-aware payloads before encryption.
///
/// Wire format (inside the encrypted envelope payload):
/// ```text
/// ┌──────────┬──────────┬──────────┬──────────┬──────────┬──────────┐
/// │ type     │ model_id │ shard_ix │ total    │ dim_len  │ dims...  │
/// │ 1 byte   │ 32 bytes │ 2 bytes  │ 2 bytes  │ 1 byte   │ N*4 B   │
/// └──────────┴──────────┴──────────┴──────────┴──────────┴──────────┘
/// followed by raw payload bytes
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TernaryHeader {
    pub payload_type: TernaryPayloadType,
    /// SHA-256 hash of the model name (for routing without leaking names).
    pub model_hash: [u8; 32],
    /// Shard index (0 for non-sharded payloads).
    pub shard_index: u16,
    /// Total number of shards (1 for non-sharded payloads).
    pub total_shards: u16,
    /// Tensor dimensions (empty for non-tensor payloads).
    pub dimensions: Vec<u32>,
}

/// A ternary packet: header + data, ready for encryption.
#[derive(Debug)]
pub struct TernaryPacket {
    pub header: TernaryHeader,
    pub data: Vec<u8>,
}

impl TernaryPacket {
    /// Create a tensor packet with 2-bit packed ternary data.
    pub fn tensor(model_name: &str, dimensions: Vec<u32>, packed_data: Vec<u8>) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::Tensor,
                model_hash: Self::hash_model_name(model_name),
                shard_index: 0,
                total_shards: 1,
                dimensions,
            },
            data: packed_data,
        }
    }

    /// Create a model shard packet.
    pub fn model_shard(
        model_name: &str,
        shard_index: u16,
        total_shards: u16,
        shard_data: Vec<u8>,
    ) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::ModelShard,
                model_hash: Self::hash_model_name(model_name),
                shard_index,
                total_shards,
                dimensions: vec![],
            },
            data: shard_data,
        }
    }

    /// Create a KV-cache snapshot packet.
    pub fn kv_cache(model_name: &str, cache_data: Vec<u8>) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::KvCache,
                model_hash: Self::hash_model_name(model_name),
                shard_index: 0,
                total_shards: 1,
                dimensions: vec![],
            },
            data: cache_data,
        }
    }

    /// Create a checkpoint packet for federated learning.
    pub fn checkpoint(model_name: &str, checkpoint_data: Vec<u8>) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::Checkpoint,
                model_hash: Self::hash_model_name(model_name),
                shard_index: 0,
                total_shards: 1,
                dimensions: vec![],
            },
            data: checkpoint_data,
        }
    }

    /// Create an inference request packet.
    pub fn inference_request(model_name: &str, request_data: Vec<u8>) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::InferenceRequest,
                model_hash: Self::hash_model_name(model_name),
                shard_index: 0,
                total_shards: 1,
                dimensions: vec![],
            },
            data: request_data,
        }
    }

    /// Create an inference result packet.
    pub fn inference_result(
        model_name: &str,
        dimensions: Vec<u32>,
        result_data: Vec<u8>,
    ) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::InferenceResult,
                model_hash: Self::hash_model_name(model_name),
                shard_index: 0,
                total_shards: 1,
                dimensions,
            },
            data: result_data,
        }
    }

    /// Create a control message packet.
    pub fn control(message: Vec<u8>) -> Self {
        Self {
            header: TernaryHeader {
                payload_type: TernaryPayloadType::Control,
                model_hash: [0u8; 32],
                shard_index: 0,
                total_shards: 1,
                dimensions: vec![],
            },
            data: message,
        }
    }

    /// Serialize the packet (header + data) into a byte buffer.
    pub fn to_bytes(&self) -> Vec<u8> {
        let dim_bytes = self.header.dimensions.len() * 4;
        let header_size = 1 + 32 + 2 + 2 + 1 + dim_bytes;
        let mut buf = Vec::with_capacity(header_size + self.data.len());

        buf.push(self.header.payload_type as u8);
        buf.extend_from_slice(&self.header.model_hash);
        buf.extend_from_slice(&self.header.shard_index.to_be_bytes());
        buf.extend_from_slice(&self.header.total_shards.to_be_bytes());
        buf.push(self.header.dimensions.len() as u8);
        for dim in &self.header.dimensions {
            buf.extend_from_slice(&dim.to_be_bytes());
        }
        buf.extend_from_slice(&self.data);

        buf
    }

    /// Deserialize a packet from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 38 {
            return Err(CryptoError::InvalidEnvelope(
                "ternary packet too short".into(),
            ));
        }

        let payload_type = TernaryPayloadType::from_u8(data[0])?;
        let mut model_hash = [0u8; 32];
        model_hash.copy_from_slice(&data[1..33]);
        let shard_index = u16::from_be_bytes([data[33], data[34]]);
        let total_shards = u16::from_be_bytes([data[35], data[36]]);
        let dim_count = data[37] as usize;

        let dim_end = 38 + dim_count * 4;
        if data.len() < dim_end {
            return Err(CryptoError::InvalidEnvelope(
                "ternary packet truncated dimensions".into(),
            ));
        }

        let mut dimensions = Vec::with_capacity(dim_count);
        for i in 0..dim_count {
            let offset = 38 + i * 4;
            let dim = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            dimensions.push(dim);
        }

        let payload_data = data[dim_end..].to_vec();

        Ok(Self {
            header: TernaryHeader {
                payload_type,
                model_hash,
                shard_index,
                total_shards,
                dimensions,
            },
            data: payload_data,
        })
    }

    /// Seal this ternary packet into an encrypted SwarmEnvelope.
    pub fn seal(
        &self,
        cipher: &SwarmCipher,
        sender: &AgentId,
        session_id: &SessionId,
        sequence: u64,
    ) -> Result<SwarmEnvelope> {
        let payload_bytes = self.to_bytes();
        SwarmEnvelope::seal(cipher, sender, session_id, sequence, &payload_bytes)
    }

    /// Open an encrypted SwarmEnvelope and parse the ternary packet inside.
    pub fn open(envelope: &SwarmEnvelope, cipher: &SwarmCipher) -> Result<Self> {
        let plaintext = envelope.open(cipher)?;
        Self::from_bytes(&plaintext)
    }

    pub fn hash_model_name(name: &str) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::{CipherSuite, SwarmCipher};
    use crate::types::SymmetricKey;

    fn test_cipher() -> SwarmCipher {
        SwarmCipher::with_suite(&SymmetricKey::from_bytes([0x42; 32]), CipherSuite::Aes256Gcm)
    }

    #[test]
    fn tensor_packet_roundtrip() {
        let packet = TernaryPacket::tensor(
            "threat_classifier",
            vec![1, 128],
            vec![0xAA; 64], // fake 2-bit packed data
        );

        let bytes = packet.to_bytes();
        let restored = TernaryPacket::from_bytes(&bytes).unwrap();

        assert_eq!(restored.header.payload_type, TernaryPayloadType::Tensor);
        assert_eq!(restored.header.dimensions, vec![1, 128]);
        assert_eq!(restored.data, vec![0xAA; 64]);
        assert_eq!(restored.header.model_hash, packet.header.model_hash);
    }

    #[test]
    fn model_shard_packet() {
        let packet = TernaryPacket::model_shard("ternary_llm", 2, 8, vec![0xBB; 1024]);

        let bytes = packet.to_bytes();
        let restored = TernaryPacket::from_bytes(&bytes).unwrap();

        assert_eq!(restored.header.payload_type, TernaryPayloadType::ModelShard);
        assert_eq!(restored.header.shard_index, 2);
        assert_eq!(restored.header.total_shards, 8);
        assert_eq!(restored.data.len(), 1024);
    }

    #[test]
    fn encrypted_ternary_roundtrip() {
        let cipher = test_cipher();
        let sender = AgentId::new();
        let session = SessionId::new();

        let packet = TernaryPacket::inference_request(
            "packet_filter",
            b"{\"input_ids\": [1, 2, 3]}".to_vec(),
        );

        // Seal into encrypted envelope
        let envelope = packet.seal(&cipher, &sender, &session, 0).unwrap();

        // Open and parse
        let restored = TernaryPacket::open(&envelope, &cipher).unwrap();
        assert_eq!(
            restored.header.payload_type,
            TernaryPayloadType::InferenceRequest
        );
        assert_eq!(restored.data, b"{\"input_ids\": [1, 2, 3]}");
    }

    #[test]
    fn kv_cache_packet() {
        let packet = TernaryPacket::kv_cache("anomaly_detector", vec![0xCC; 512]);
        let bytes = packet.to_bytes();
        let restored = TernaryPacket::from_bytes(&bytes).unwrap();
        assert_eq!(restored.header.payload_type, TernaryPayloadType::KvCache);
    }

    #[test]
    fn checkpoint_packet() {
        let packet = TernaryPacket::checkpoint("traffic_sentinel", vec![0xDD; 256]);
        let bytes = packet.to_bytes();
        let restored = TernaryPacket::from_bytes(&bytes).unwrap();
        assert_eq!(restored.header.payload_type, TernaryPayloadType::Checkpoint);
    }

    #[test]
    fn control_message() {
        let packet = TernaryPacket::control(b"REBALANCE_SHARDS".to_vec());
        let bytes = packet.to_bytes();
        let restored = TernaryPacket::from_bytes(&bytes).unwrap();
        assert_eq!(restored.header.payload_type, TernaryPayloadType::Control);
        assert_eq!(restored.data, b"REBALANCE_SHARDS");
    }

    #[test]
    fn inference_result_with_dimensions() {
        let packet = TernaryPacket::inference_result(
            "threat_classifier",
            vec![1, 256, 32000],
            vec![0xFF; 128],
        );
        let bytes = packet.to_bytes();
        let restored = TernaryPacket::from_bytes(&bytes).unwrap();
        assert_eq!(
            restored.header.payload_type,
            TernaryPayloadType::InferenceResult
        );
        assert_eq!(restored.header.dimensions, vec![1, 256, 32000]);
    }

    #[test]
    fn too_short_packet_rejected() {
        assert!(TernaryPacket::from_bytes(&[0u8; 10]).is_err());
    }
}
