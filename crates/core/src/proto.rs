//! Protocol buffer message types for BUNNY agent services.
//!
//! These types use `prost::Message` for protobuf-compatible serialization.
//! The gRPC service definition is in `proto/agent.proto`.

use prost::Message;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct InferenceRequest {
    #[prost(string, tag = "1")]
    pub model_name: String,
    #[prost(bytes = "vec", tag = "2")]
    pub input_data: Vec<u8>,
    #[prost(string, tag = "3")]
    pub session_id: String,
}

#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct InferenceResponse {
    #[prost(bytes = "vec", tag = "1")]
    pub output_data: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub inference_time_ms: u64,
    #[prost(string, tag = "3")]
    pub model_name: String,
}

#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct StatusRequest {
    #[prost(string, tag = "1")]
    pub agent_id: String,
}

#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct StatusResponse {
    #[prost(string, tag = "1")]
    pub agent_id: String,
    #[prost(string, tag = "2")]
    pub status: String,
    #[prost(string, repeated, tag = "3")]
    pub loaded_models: Vec<String>,
    #[prost(uint64, tag = "4")]
    pub uptime_ms: u64,
}

#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct LoadModelRequest {
    #[prost(string, tag = "1")]
    pub model_name: String,
    #[prost(bytes = "vec", tag = "2")]
    pub model_data: Vec<u8>,
    #[prost(string, tag = "3")]
    pub format: String,
}

#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct LoadModelResponse {
    #[prost(bool, tag = "1")]
    pub success: bool,
    #[prost(string, tag = "2")]
    pub message: String,
}
