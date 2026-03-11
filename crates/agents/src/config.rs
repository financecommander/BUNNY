use serde::{Deserialize, Serialize};

/// Model scale tier for routing decisions.
///
/// Based on L4 GPU benchmarks establishing ternary crossover points:
/// - Cell (~2.2M params) → dense CPU best
/// - Micro (~8.4M params) → CUDA packed ternary wins (2.28x)
/// - Pico (~40M params) → CUDA packed ternary dominates (11.5x)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelScale {
    /// < 5M params — dense CPU is faster than ternary.
    Cell,
    /// 5M–20M params — GPU ternary crossover, 2x+ speedup.
    Micro,
    /// 20M+ params — GPU ternary dominant, 10x+ speedup.
    Pico,
}

impl ModelScale {
    /// Classify a parameter count into a model scale tier.
    pub fn from_param_count(params: usize) -> Self {
        if params < 5_000_000 {
            Self::Cell
        } else if params < 20_000_000 {
            Self::Micro
        } else {
            Self::Pico
        }
    }

    /// Recommended compute backend for this scale.
    pub fn recommended_backend(&self) -> ComputeBackend {
        match self {
            Self::Cell => ComputeBackend::DenseCpu,
            Self::Micro => ComputeBackend::CudaPacked,
            Self::Pico => ComputeBackend::CudaPacked,
        }
    }
}

/// Compute backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputeBackend {
    /// Standard dense floating-point on CPU.
    DenseCpu,
    /// 2-bit packed ternary on CPU (crossover at ~40M params).
    TernaryCpu,
    /// 2-bit packed ternary on CUDA (crossover at ~8.4M params).
    CudaPacked,
}

/// Configuration for an agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique name for this agent.
    pub name: String,
    /// Agent role description.
    pub role: AgentRole,
    /// System prompt for LLM-backed decisions.
    pub system_prompt: String,
    /// Model to use for inference (if any).
    pub model_name: Option<String>,
    /// Override compute backend (auto-selects based on model scale if None).
    pub compute_backend: Option<ComputeBackend>,
    /// Maximum inference time before timeout (ms).
    pub inference_timeout_ms: u64,
    /// Temperature for stochastic decisions (0.0 = deterministic).
    pub temperature: f32,
}

impl AgentConfig {
    /// Create a new config with sensible defaults.
    pub fn new(name: impl Into<String>, role: AgentRole) -> Self {
        Self {
            name: name.into(),
            role,
            system_prompt: String::new(),
            model_name: None,
            compute_backend: None,
            inference_timeout_ms: 5000,
            temperature: 0.1,
        }
    }

    /// Set the system prompt.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Set the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model_name = Some(model.into());
        self
    }

    /// Set the compute backend explicitly.
    pub fn with_backend(mut self, backend: ComputeBackend) -> Self {
        self.compute_backend = Some(backend);
        self
    }
}

/// Predefined agent roles matching the swarm architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    ThreatHunter,
    Sandbox,
    IotFirewall,
    AiGuardian,
    Learning,
    /// Task dispatcher — classifies requests and routes to SWARM.
    /// Cannot modify code, repos, or infrastructure.
    Dispatcher,
    /// Custom agent with user-defined behavior.
    Custom,
}

impl AgentRole {
    /// Default system prompt for this role.
    pub fn default_prompt(&self) -> &'static str {
        match self {
            Self::ThreatHunter => crate::prompts::THREAT_HUNTER_PROMPT,
            Self::Sandbox => crate::prompts::SANDBOX_PROMPT,
            Self::IotFirewall => crate::prompts::IOT_FIREWALL_PROMPT,
            Self::AiGuardian => crate::prompts::AI_GUARDIAN_PROMPT,
            Self::Learning => crate::prompts::LEARNING_PROMPT,
            Self::Dispatcher => crate::prompts::DISPATCHER_PROMPT,
            Self::Custom => "",
        }
    }
}
