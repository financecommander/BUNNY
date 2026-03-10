use tracing::debug;

use bunny_agents::{ComputeBackend, ModelScale};
use bunny_crypto::swarm::MessagePriority;
use bunny_crypto::types::AgentId;

use crate::error::{PortalError, Result};
use crate::model_registry::ModelRegistry;

/// Routing decision for an inference request.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// Model to target.
    pub model_name: String,
    /// Scale tier.
    pub scale: ModelScale,
    /// Compute backend.
    pub backend: ComputeBackend,
    /// Priority for the task queue.
    pub priority: MessagePriority,
    /// Available workers for this model.
    pub candidate_workers: Vec<AgentId>,
}

/// Request priority hint from the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityHint {
    /// Best-effort, lowest queue priority.
    BestEffort,
    /// Standard priority (default).
    Standard,
    /// Low-latency priority — may preempt standard tasks.
    LowLatency,
    /// Critical — security/threat detection, highest priority.
    Critical,
}

impl PriorityHint {
    pub fn to_message_priority(self) -> MessagePriority {
        match self {
            Self::BestEffort => MessagePriority::Low,
            Self::Standard => MessagePriority::Normal,
            Self::LowLatency => MessagePriority::High,
            Self::Critical => MessagePriority::Critical,
        }
    }
}

/// Routes inference requests to the correct model/worker/backend.
///
/// Implements the benchmark-informed routing policy:
/// - cell (~2.2M params) → dense CPU
/// - micro (~8.4M params) → CUDA packed ternary
/// - pico (~40M params)   → CUDA packed ternary (dominant)
///
/// Also handles model validation and worker availability checks.
pub struct RequestRouter {
    registry: ModelRegistry,
}

impl RequestRouter {
    pub fn new(registry: ModelRegistry) -> Self {
        Self { registry }
    }

    /// Route an inference request.
    ///
    /// Validates the model exists, checks worker availability, and returns
    /// a routing decision with the recommended backend and priority.
    pub fn route(
        &self,
        model_name: &str,
        priority: PriorityHint,
    ) -> Result<RouteDecision> {
        let entry = self
            .registry
            .get(model_name)
            .ok_or_else(|| PortalError::ModelNotFound(model_name.to_string()))?;

        if entry.workers.is_empty() {
            return Err(PortalError::NoWorkerAvailable(model_name.to_string()));
        }

        debug!(
            model = %model_name,
            scale = ?entry.scale,
            backend = ?entry.backend,
            workers = entry.workers.len(),
            "route resolved"
        );

        Ok(RouteDecision {
            model_name: model_name.to_string(),
            scale: entry.scale,
            backend: entry.backend,
            priority: priority.to_message_priority(),
            candidate_workers: entry.workers.clone(),
        })
    }

    /// Validate that a model can accept the given input dimensions.
    pub fn validate_input(
        &self,
        model_name: &str,
        input_len: usize,
    ) -> Result<()> {
        let entry = self
            .registry
            .get(model_name)
            .ok_or_else(|| PortalError::ModelNotFound(model_name.to_string()))?;

        if input_len != entry.input_dim {
            return Err(PortalError::InvalidRequest(format!(
                "model '{}' expects input dim {}, got {}",
                model_name, entry.input_dim, input_len
            )));
        }
        Ok(())
    }

    /// Get a reference to the underlying registry.
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// List all available models with their metadata.
    pub fn available_models(&self) -> Vec<ModelInfo> {
        self.registry
            .snapshot()
            .into_iter()
            .map(|e| ModelInfo {
                name: e.name,
                params: e.total_params,
                scale: e.scale,
                backend: e.backend,
                input_dim: e.input_dim,
                output_dim: e.output_dim,
                worker_count: e.workers.len(),
                sharded: e.sharded,
                vram_bytes: e.vram_bytes,
            })
            .collect()
    }
}

/// Public-facing model metadata (no internal IDs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub params: usize,
    pub scale: ModelScale,
    pub backend: ComputeBackend,
    pub input_dim: usize,
    pub output_dim: usize,
    pub worker_count: usize,
    pub sharded: bool,
    pub vram_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::ModelEntry;
    use bunny_crypto::ternary::TernaryPacket;

    fn setup_router() -> RequestRouter {
        let registry = ModelRegistry::new();
        let worker = AgentId::new();

        registry.register_model(ModelEntry {
            name: "threat_model".into(),
            model_hash: TernaryPacket::hash_model_name("threat_model"),
            total_params: 8_400_000,
            scale: ModelScale::Micro,
            backend: ComputeBackend::CudaPacked,
            num_layers: 4,
            input_dim: 192,
            output_dim: 5,
            workers: vec![worker],
            sharded: false,
            vram_bytes: ModelEntry::estimate_vram(8_400_000),
        });

        RequestRouter::new(registry)
    }

    #[test]
    fn route_existing_model() {
        let router = setup_router();
        let decision = router.route("threat_model", PriorityHint::Standard).unwrap();

        assert_eq!(decision.model_name, "threat_model");
        assert_eq!(decision.scale, ModelScale::Micro);
        assert_eq!(decision.backend, ComputeBackend::CudaPacked);
        assert_eq!(decision.priority, MessagePriority::Normal);
        assert_eq!(decision.candidate_workers.len(), 1);
    }

    #[test]
    fn route_missing_model() {
        let router = setup_router();
        assert!(router.route("nonexistent", PriorityHint::Standard).is_err());
    }

    #[test]
    fn validate_input_dim() {
        let router = setup_router();
        assert!(router.validate_input("threat_model", 192).is_ok());
        assert!(router.validate_input("threat_model", 100).is_err());
    }

    #[test]
    fn priority_mapping() {
        assert_eq!(PriorityHint::BestEffort.to_message_priority(), MessagePriority::Low);
        assert_eq!(PriorityHint::Standard.to_message_priority(), MessagePriority::Normal);
        assert_eq!(PriorityHint::LowLatency.to_message_priority(), MessagePriority::High);
        assert_eq!(PriorityHint::Critical.to_message_priority(), MessagePriority::Critical);
    }

    #[test]
    fn available_models_list() {
        let router = setup_router();
        let models = router.available_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "threat_model");
        assert_eq!(models[0].worker_count, 1);
    }
}
