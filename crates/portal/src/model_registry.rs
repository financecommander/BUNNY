use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use bunny_agents::{ComputeBackend, ModelScale};
use bunny_crypto::types::AgentId;

/// Metadata about a loaded model across the swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Human-readable model name.
    pub name: String,
    /// SHA-256 hash of the model name (for routing).
    pub model_hash: [u8; 32],
    /// Total parameter count.
    pub total_params: usize,
    /// Scale classification (cell/micro/pico).
    pub scale: ModelScale,
    /// Recommended compute backend.
    pub backend: ComputeBackend,
    /// Number of layers.
    pub num_layers: usize,
    /// Input dimension.
    pub input_dim: usize,
    /// Output dimension.
    pub output_dim: usize,
    /// Workers that have this model loaded.
    pub workers: Vec<AgentId>,
    /// Whether the model is sharded across workers.
    pub sharded: bool,
    /// VRAM usage estimate in bytes (2-bit packed).
    pub vram_bytes: usize,
}

impl ModelEntry {
    /// Estimated VRAM for 2-bit packed ternary (2 bits per weight).
    pub fn estimate_vram(total_params: usize) -> usize {
        // 2 bits per weight = params / 4 bytes, plus overhead
        (total_params / 4) + 4096
    }
}

/// Thread-safe registry tracking all models loaded across the swarm.
///
/// The portal uses this to route inference requests to the correct worker.
/// Workers register their models after loading, and the registry tracks
/// which workers can serve which models.
#[derive(Clone)]
pub struct ModelRegistry {
    models: Arc<DashMap<String, ModelEntry>>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: Arc::new(DashMap::new()),
        }
    }

    /// Register a model as loaded on a specific worker.
    pub fn register_model(&self, entry: ModelEntry) {
        let name = entry.name.clone();
        info!(
            model = %name,
            params = entry.total_params,
            scale = ?entry.scale,
            workers = entry.workers.len(),
            "model registered"
        );
        self.models.insert(name, entry);
    }

    /// Add a worker to an existing model's worker list.
    pub fn add_worker(&self, model_name: &str, worker_id: AgentId) -> bool {
        if let Some(mut entry) = self.models.get_mut(model_name) {
            if !entry.workers.contains(&worker_id) {
                entry.workers.push(worker_id);
                debug!(model = %model_name, "worker added to model");
            }
            true
        } else {
            false
        }
    }

    /// Remove a worker from a model's worker list.
    pub fn remove_worker(&self, model_name: &str, worker_id: &AgentId) {
        if let Some(mut entry) = self.models.get_mut(model_name) {
            entry.workers.retain(|w| w != worker_id);
            if entry.workers.is_empty() {
                warn!(model = %model_name, "model has no active workers");
            }
        }
    }

    /// Remove a worker from ALL models (e.g., when worker dies).
    pub fn remove_worker_from_all(&self, worker_id: &AgentId) {
        for mut entry in self.models.iter_mut() {
            entry.workers.retain(|w| w != worker_id);
        }
    }

    /// Look up a model by name.
    pub fn get(&self, model_name: &str) -> Option<ModelEntry> {
        self.models.get(model_name).map(|e| e.clone())
    }

    /// Get workers that can serve a given model.
    pub fn workers_for(&self, model_name: &str) -> Vec<AgentId> {
        self.models
            .get(model_name)
            .map(|e| e.workers.clone())
            .unwrap_or_default()
    }

    /// List all registered model names.
    pub fn model_names(&self) -> Vec<String> {
        self.models.iter().map(|r| r.key().clone()).collect()
    }

    /// Total models registered.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Get all model entries (snapshot).
    pub fn snapshot(&self) -> Vec<ModelEntry> {
        self.models.iter().map(|r| r.value().clone()).collect()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a portal `ModelEntry` into a network `ModelDescriptor`
/// for inclusion in agent manifests published to other codespaces.
impl From<ModelEntry> for bunny_network::ModelDescriptor {
    fn from(entry: ModelEntry) -> Self {
        Self {
            name: entry.name,
            total_params: entry.total_params,
            scale: entry.scale,
            backend: entry.backend,
            input_dim: entry.input_dim,
            output_dim: entry.output_dim,
            num_layers: entry.num_layers,
            vram_bytes: entry.vram_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_crypto::ternary::TernaryPacket;

    fn make_entry(name: &str, params: usize, workers: Vec<AgentId>) -> ModelEntry {
        let scale = ModelScale::from_param_count(params);
        ModelEntry {
            name: name.to_string(),
            model_hash: TernaryPacket::hash_model_name(name),
            total_params: params,
            scale,
            backend: scale.recommended_backend(),
            num_layers: 4,
            input_dim: 128,
            output_dim: 10,
            workers,
            sharded: false,
            vram_bytes: ModelEntry::estimate_vram(params),
        }
    }

    #[test]
    fn register_and_lookup() {
        let registry = ModelRegistry::new();
        let worker = AgentId::new();
        let entry = make_entry("threat_model", 8_000_000, vec![worker.clone()]);
        registry.register_model(entry);

        assert_eq!(registry.model_count(), 1);
        let found = registry.get("threat_model").unwrap();
        assert_eq!(found.total_params, 8_000_000);
        assert_eq!(found.scale, ModelScale::Micro);
        assert_eq!(found.workers.len(), 1);
    }

    #[test]
    fn add_remove_workers() {
        let registry = ModelRegistry::new();
        let w1 = AgentId::new();
        let w2 = AgentId::new();

        registry.register_model(make_entry("model_a", 1_000_000, vec![w1.clone()]));
        assert!(registry.add_worker("model_a", w2.clone()));
        assert_eq!(registry.workers_for("model_a").len(), 2);

        registry.remove_worker("model_a", &w1);
        assert_eq!(registry.workers_for("model_a").len(), 1);
    }

    #[test]
    fn remove_worker_from_all() {
        let registry = ModelRegistry::new();
        let worker = AgentId::new();

        registry.register_model(make_entry("m1", 1_000_000, vec![worker.clone()]));
        registry.register_model(make_entry("m2", 2_000_000, vec![worker.clone()]));

        registry.remove_worker_from_all(&worker);
        assert!(registry.workers_for("m1").is_empty());
        assert!(registry.workers_for("m2").is_empty());
    }

    #[test]
    fn vram_estimate() {
        // 40M params → ~10MB packed
        let vram = ModelEntry::estimate_vram(40_000_000);
        assert!(vram > 9_000_000 && vram < 11_000_000);
    }

    #[test]
    fn model_entry_to_descriptor_conversion() {
        let worker = AgentId::new();
        let entry = make_entry("threat_model", 8_000_000, vec![worker]);
        let descriptor: bunny_network::ModelDescriptor = entry.into();

        assert_eq!(descriptor.name, "threat_model");
        assert_eq!(descriptor.total_params, 8_000_000);
        assert_eq!(descriptor.scale, ModelScale::Micro);
        assert_eq!(descriptor.backend, ComputeBackend::CudaPacked);
        assert_eq!(descriptor.input_dim, 128);
        assert_eq!(descriptor.output_dim, 10);
        assert_eq!(descriptor.num_layers, 4);
    }

    #[test]
    fn scale_classification() {
        let cell = make_entry("cell", 2_200_000, vec![]);
        assert_eq!(cell.scale, ModelScale::Cell);
        assert_eq!(cell.backend, ComputeBackend::DenseCpu);

        let micro = make_entry("micro", 8_400_000, vec![]);
        assert_eq!(micro.scale, ModelScale::Micro);
        assert_eq!(micro.backend, ComputeBackend::CudaPacked);

        let pico = make_entry("pico", 40_000_000, vec![]);
        assert_eq!(pico.scale, ModelScale::Pico);
        assert_eq!(pico.backend, ComputeBackend::CudaPacked);
    }
}
