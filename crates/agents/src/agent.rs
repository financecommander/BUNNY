use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use bunny_triton::TernaryEngine;

use crate::config::{AgentConfig, AgentRole, ComputeBackend, ModelScale};
use crate::error::{AgentError, Result};

/// Input to an agent's process function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInput {
    /// Raw input data (model tensor input or structured payload).
    pub data: Vec<f32>,
    /// Optional metadata for context.
    pub metadata: std::collections::HashMap<String, String>,
}

/// Output from an agent's process function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutput {
    /// Raw output data (model logits or structured result).
    pub data: Vec<f32>,
    /// Predicted class index (argmax of output).
    pub prediction: usize,
    /// Inference time in microseconds.
    pub inference_time_us: u64,
    /// Which compute backend was used.
    pub backend: ComputeBackend,
    /// Optional structured result (JSON-serializable verdict, action, etc.).
    pub result: Option<String>,
}

/// Agent trait — the core abstraction for all BUNNY agents.
pub trait Agent: Send + Sync {
    /// Unique agent name.
    fn name(&self) -> &str;

    /// Agent role.
    fn role(&self) -> AgentRole;

    /// System prompt for LLM-backed decisions.
    fn system_prompt(&self) -> &str;

    /// Process an input and produce an output.
    fn process(&self, input: &AgentInput) -> Result<AgentOutput>;
}

/// Ternary-inference-backed agent.
///
/// Wraps a `TernaryEngine` and implements the `Agent` trait. The engine runs
/// 2-bit packed matmul inference with zero-skipping — no floating-point
/// multiplies, just additions and subtractions.
pub struct TernaryAgent {
    config: AgentConfig,
    engine: TernaryEngine,
    scale: ModelScale,
}

impl TernaryAgent {
    /// Create a new ternary agent from a config and engine.
    pub fn new(config: AgentConfig, engine: TernaryEngine) -> Self {
        let params = engine.model().total_params();
        let scale = ModelScale::from_param_count(params);

        info!(
            name = %config.name,
            params = params,
            scale = ?scale,
            backend = ?config.compute_backend.unwrap_or_else(|| scale.recommended_backend()),
            "created ternary agent"
        );

        Self {
            config,
            engine,
            scale,
        }
    }

    /// The model scale tier (cell/micro/pico).
    pub fn scale(&self) -> ModelScale {
        self.scale
    }

    /// The effective compute backend (explicit override or auto from scale).
    pub fn effective_backend(&self) -> ComputeBackend {
        self.config
            .compute_backend
            .unwrap_or_else(|| self.scale.recommended_backend())
    }

    /// Get a reference to the underlying engine.
    pub fn engine(&self) -> &TernaryEngine {
        &self.engine
    }
}

impl Agent for TernaryAgent {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn role(&self) -> AgentRole {
        self.config.role
    }

    fn system_prompt(&self) -> &str {
        &self.config.system_prompt
    }

    fn process(&self, input: &AgentInput) -> Result<AgentOutput> {
        let start = std::time::Instant::now();

        debug!(
            agent = %self.config.name,
            input_dim = input.data.len(),
            backend = ?self.effective_backend(),
            "running inference"
        );

        let output = self
            .engine
            .forward(&input.data)
            .map_err(|e| AgentError::Inference(e.to_string()))?;

        let prediction = output
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);

        let elapsed = start.elapsed();

        debug!(
            agent = %self.config.name,
            prediction = prediction,
            time_us = elapsed.as_micros(),
            "inference complete"
        );

        Ok(AgentOutput {
            data: output,
            prediction,
            inference_time_us: elapsed.as_micros() as u64,
            backend: self.effective_backend(),
            result: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_triton::activation::Activation;
    use bunny_triton::model::TernaryModelBuilder;
    use bunny_triton::packing::TernaryValue;

    fn make_test_agent() -> TernaryAgent {
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
        let config = AgentConfig::new("test_threat_hunter", AgentRole::ThreatHunter)
            .with_prompt(crate::prompts::THREAT_HUNTER_PROMPT);

        TernaryAgent::new(config, engine)
    }

    #[test]
    fn agent_trait_basics() {
        let agent = make_test_agent();
        assert_eq!(agent.name(), "test_threat_hunter");
        assert_eq!(agent.role(), AgentRole::ThreatHunter);
        assert!(!agent.system_prompt().is_empty());
    }

    #[test]
    fn agent_scale_cell() {
        let agent = make_test_agent();
        // Tiny test model — cell scale.
        assert_eq!(agent.scale(), ModelScale::Cell);
        assert_eq!(agent.effective_backend(), ComputeBackend::DenseCpu);
    }

    #[test]
    fn agent_process() {
        let agent = make_test_agent();
        let input = AgentInput {
            data: vec![1.0, 2.0, 3.0, 4.0],
            metadata: std::collections::HashMap::new(),
        };
        let output = agent.process(&input).unwrap();

        // Same expected values as TernaryEngine tests:
        // output[0] = -4.0, output[1] = 6.0
        assert_eq!(output.prediction, 1);
        assert_eq!(output.data.len(), 2);
        assert!((output.data[0] - (-4.0)).abs() < f32::EPSILON);
        assert!((output.data[1] - 6.0).abs() < f32::EPSILON);
        assert!(output.inference_time_us < 1_000_000); // < 1 second
    }

    #[test]
    fn agent_wrong_input() {
        let agent = make_test_agent();
        let input = AgentInput {
            data: vec![1.0, 2.0],
            metadata: std::collections::HashMap::new(),
        };
        assert!(agent.process(&input).is_err());
    }

    #[test]
    fn model_scale_classification() {
        assert_eq!(ModelScale::from_param_count(2_200_000), ModelScale::Cell);
        assert_eq!(ModelScale::from_param_count(8_400_000), ModelScale::Micro);
        assert_eq!(ModelScale::from_param_count(40_000_000), ModelScale::Pico);

        assert_eq!(
            ModelScale::Cell.recommended_backend(),
            ComputeBackend::DenseCpu
        );
        assert_eq!(
            ModelScale::Micro.recommended_backend(),
            ComputeBackend::CudaPacked
        );
        assert_eq!(
            ModelScale::Pico.recommended_backend(),
            ComputeBackend::CudaPacked
        );
    }
}
