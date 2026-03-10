use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, info};

use bunny_crypto::swarm::{SwarmMessage, SwarmMessageType};
use bunny_crypto::ternary::TernaryPacket;
use bunny_crypto::types::AgentId;

use crate::agent::{Agent, AgentInput, AgentOutput};
use crate::error::{AgentError, Result};

/// Manages a set of named agents and dispatches inference requests to them.
///
/// The `AgentRunner` sits between the swarm transport layer and the agent
/// implementations. It receives `SwarmMessage::InferenceDispatch` payloads,
/// routes them to the correct agent based on the ternary header's model hash,
/// and returns `SwarmMessage::InferenceReturn` with the result.
pub struct AgentRunner {
    agents: HashMap<String, Arc<dyn Agent>>,
    agent_id: AgentId,
}

impl AgentRunner {
    /// Create a new runner with the given agent identity.
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agents: HashMap::new(),
            agent_id,
        }
    }

    /// Register an agent by name.
    pub fn register(&mut self, agent: Arc<dyn Agent>) {
        let name = agent.name().to_string();
        info!(agent = %name, role = ?agent.role(), "registered agent");
        self.agents.insert(name, agent);
    }

    /// Remove a registered agent.
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Agent>> {
        let removed = self.agents.remove(name);
        if removed.is_some() {
            info!(agent = %name, "unregistered agent");
        }
        removed
    }

    /// List all registered agent names.
    pub fn registered_agents(&self) -> Vec<&str> {
        self.agents.keys().map(|k| k.as_str()).collect()
    }

    /// Get a reference to a registered agent.
    pub fn get_agent(&self, name: &str) -> Option<&Arc<dyn Agent>> {
        self.agents.get(name)
    }

    /// The runner's agent identity.
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// Dispatch an inference request to a named agent.
    pub fn dispatch(&self, agent_name: &str, input: AgentInput) -> Result<AgentOutput> {
        let agent = self
            .agents
            .get(agent_name)
            .ok_or_else(|| AgentError::NotFound(agent_name.to_string()))?;

        debug!(
            agent = %agent_name,
            input_len = input.data.len(),
            "dispatching inference"
        );

        agent.process(&input)
    }

    /// Process an inbound `InferenceDispatch` swarm message.
    ///
    /// Uses `SwarmMessage::into_ternary()` to extract the `TernaryPacket`,
    /// finds the matching agent by model hash, runs inference, and returns a
    /// reply `SwarmMessage` with `InferenceReturn` type.
    pub fn handle_dispatch(&self, message: &SwarmMessage) -> Result<SwarmMessage> {
        if message.msg_type != SwarmMessageType::InferenceDispatch {
            return Err(AgentError::Dispatch(format!(
                "expected InferenceDispatch, got {:?}",
                message.msg_type
            )));
        }

        // Extract the TernaryPacket from the message payload.
        let packet = message
            .into_ternary()
            .map_err(|e| AgentError::Dispatch(e.to_string()))?;

        debug!(
            payload_type = ?packet.header.payload_type,
            shard = packet.header.shard_index,
            "received inference dispatch"
        );

        // Find agent by matching model hash against registered agents.
        let (agent_name, agent) = self
            .find_agent_by_model_hash(&packet.header.model_hash)
            .ok_or_else(|| AgentError::NotFound("no agent for model hash".into()))?;

        // Deserialize input floats from the packet data.
        let input_data = bytes_to_f32s(&packet.data);

        let input = AgentInput {
            data: input_data,
            metadata: HashMap::new(),
        };

        let output = agent.process(&input)?;

        info!(
            agent = %agent_name,
            prediction = output.prediction,
            time_us = output.inference_time_us,
            "inference complete"
        );

        // Build the response TernaryPacket.
        let result_data: Vec<u8> = output
            .data
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let result_packet = TernaryPacket::inference_result(
            &agent_name,
            vec![output.data.len() as u32],
            result_data,
        );

        let response = SwarmMessage::inference_return(message, &result_packet);
        Ok(response)
    }

    /// Find an agent whose model name hashes to the given model hash.
    fn find_agent_by_model_hash(
        &self,
        model_hash: &[u8; 32],
    ) -> Option<(String, &Arc<dyn Agent>)> {
        for (name, agent) in &self.agents {
            let name_hash = TernaryPacket::hash_model_name(name);
            if &name_hash == model_hash {
                return Some((name.clone(), agent));
            }
        }
        None
    }
}

/// Convert raw bytes to f32 slice (little-endian).
fn bytes_to_f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::TernaryAgent;
    use crate::config::{AgentConfig, AgentRole};
    use bunny_crypto::swarm::MessagePriority;
    use bunny_crypto::ternary::TernaryPayloadType;
    use bunny_triton::activation::Activation;
    use bunny_triton::model::TernaryModelBuilder;
    use bunny_triton::packing::TernaryValue;
    use bunny_triton::TernaryEngine;

    fn make_test_runner() -> AgentRunner {
        let model = TernaryModelBuilder::new("test_model")
            .add_dense_layer(
                "hidden",
                4,
                3,
                &[
                    vec![
                        TernaryValue::Pos,
                        TernaryValue::Pos,
                        TernaryValue::Zero,
                        TernaryValue::Zero,
                    ],
                    vec![
                        TernaryValue::Zero,
                        TernaryValue::Zero,
                        TernaryValue::Pos,
                        TernaryValue::Pos,
                    ],
                    vec![
                        TernaryValue::Neg,
                        TernaryValue::Pos,
                        TernaryValue::Neg,
                        TernaryValue::Pos,
                    ],
                ],
                vec![0.0; 3],
                Activation::ReLU,
            )
            .unwrap()
            .add_dense_layer(
                "output",
                3,
                2,
                &[
                    vec![TernaryValue::Pos, TernaryValue::Neg, TernaryValue::Zero],
                    vec![TernaryValue::Neg, TernaryValue::Pos, TernaryValue::Pos],
                ],
                vec![0.0; 2],
                Activation::None,
            )
            .unwrap()
            .build()
            .unwrap();

        let engine = TernaryEngine::new(model);
        let config = AgentConfig::new("test_agent", AgentRole::ThreatHunter)
            .with_prompt(crate::prompts::THREAT_HUNTER_PROMPT);

        let agent = TernaryAgent::new(config, engine);

        let mut runner = AgentRunner::new(AgentId::new());
        runner.register(Arc::new(agent));
        runner
    }

    #[test]
    fn register_and_dispatch() {
        let runner = make_test_runner();
        assert_eq!(runner.registered_agents().len(), 1);

        let input = AgentInput {
            data: vec![1.0, 2.0, 3.0, 4.0],
            metadata: HashMap::new(),
        };

        let output = runner.dispatch("test_agent", input).unwrap();
        assert_eq!(output.prediction, 1);
    }

    #[test]
    fn dispatch_unknown_agent() {
        let runner = make_test_runner();
        let input = AgentInput {
            data: vec![1.0],
            metadata: HashMap::new(),
        };
        assert!(runner.dispatch("nonexistent", input).is_err());
    }

    #[test]
    fn unregister_agent() {
        let mut runner = make_test_runner();
        assert!(runner.unregister("test_agent").is_some());
        assert!(runner.registered_agents().is_empty());
    }

    #[test]
    fn handle_inference_dispatch() {
        let runner = make_test_runner();

        // Build a TernaryPacket for dispatch.
        let input_data: Vec<u8> = [1.0f32, 2.0, 3.0, 4.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let packet = TernaryPacket::inference_request("test_agent", input_data);
        let message = SwarmMessage::inference_dispatch(&packet);

        let response = runner.handle_dispatch(&message).unwrap();
        assert_eq!(response.msg_type, SwarmMessageType::InferenceReturn);

        // Verify the response contains a valid TernaryPacket.
        let result_packet = response.into_ternary().unwrap();
        assert_eq!(
            result_packet.header.payload_type,
            TernaryPayloadType::InferenceResult
        );

        // Verify output values.
        let output = bytes_to_f32s(&result_packet.data);
        assert_eq!(output.len(), 2);
        assert!((output[0] - (-4.0)).abs() < f32::EPSILON);
        assert!((output[1] - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn wrong_message_type() {
        let runner = make_test_runner();
        let message = SwarmMessage::new(
            SwarmMessageType::Heartbeat,
            MessagePriority::Low,
            vec![],
        );
        assert!(runner.handle_dispatch(&message).is_err());
    }
}
