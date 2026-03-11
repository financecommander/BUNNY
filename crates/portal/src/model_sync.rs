//! Model sync — wires GPU worker heartbeats into the model registry.
//!
//! This is the critical missing link: workers broadcast heartbeats with
//! `RuntimeHealthInfo.loaded_models`, but that data never reached the
//! `ModelRegistry`. This module closes the loop so BUNNY/swarm has
//! live model inventory across all codespaces.
//!
//! Flow:
//! ```text
//! WorkerHeartbeat → ModelSyncer.sync_heartbeat()
//!   → updates ModelRegistry entries
//!   → adds/removes workers for each model
//!   → emits ModelSyncEvent for downstream consumers
//! ```

use std::collections::HashSet;

use tracing::{debug, info, warn};

use bunny_agents::ModelScale;
use bunny_crypto::ternary::TernaryPacket;
use bunny_crypto::types::AgentId;
use bunny_network::{
    AgentManifest, LoadedModel, WorkerHeartbeat,
    ModelDescriptor, AgentManifestBuilder, Codespace,
};

use crate::model_registry::{ModelEntry, ModelRegistry};

// ── Sync Events ─────────────────────────────────────────────────────

/// Events emitted during model sync for downstream consumers.
#[derive(Debug, Clone)]
pub enum ModelSyncEvent {
    /// A new model was discovered on a worker.
    ModelDiscovered {
        model_name: String,
        worker_id: AgentId,
    },
    /// An existing model gained a new worker.
    WorkerAdded {
        model_name: String,
        worker_id: AgentId,
    },
    /// A model was removed from a worker (no longer loaded).
    WorkerRemoved {
        model_name: String,
        worker_id: AgentId,
    },
    /// All models removed from a worker (worker went down).
    WorkerEvicted {
        worker_id: AgentId,
    },
}

// ── Model Syncer ────────────────────────────────────────────────────

/// Syncs GPU worker heartbeats into the model registry.
///
/// Call `sync_heartbeat()` every time a `WorkerHeartbeat` arrives.
/// Call `evict_worker()` when a worker disconnects or times out.
pub struct ModelSyncer {
    registry: ModelRegistry,
}

impl ModelSyncer {
    /// Create a new syncer backed by the given registry.
    pub fn new(registry: ModelRegistry) -> Self {
        Self { registry }
    }

    /// Get a reference to the underlying registry.
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// Process an incoming worker heartbeat and sync loaded models.
    ///
    /// Returns a list of sync events describing what changed.
    pub fn sync_heartbeat(
        &self,
        worker_id: &AgentId,
        heartbeat: &WorkerHeartbeat,
    ) -> Vec<ModelSyncEvent> {
        let mut events = Vec::new();

        let runtime = match &heartbeat.runtime {
            Some(r) => r,
            None => {
                debug!(
                    worker = %heartbeat.node_name,
                    "heartbeat has no runtime info, skipping sync"
                );
                return events;
            }
        };

        if !runtime.healthy {
            // Unhealthy worker — remove from all models.
            self.registry.remove_worker_from_all(worker_id);
            events.push(ModelSyncEvent::WorkerEvicted {
                worker_id: worker_id.clone(),
            });
            warn!(
                worker = %heartbeat.node_name,
                "unhealthy runtime, evicted from all models"
            );
            return events;
        }

        // Collect currently loaded model names from the heartbeat.
        let loaded: HashSet<String> = runtime
            .loaded_models
            .iter()
            .filter(|m| m.ready)
            .map(|m| m.name.clone())
            .collect();

        // Check existing registry — remove worker from models no longer loaded.
        for name in self.registry.model_names() {
            if !loaded.contains(&name) {
                let workers = self.registry.workers_for(&name);
                if workers.contains(worker_id) {
                    self.registry.remove_worker(&name, worker_id);
                    events.push(ModelSyncEvent::WorkerRemoved {
                        model_name: name.clone(),
                        worker_id: worker_id.clone(),
                    });
                    debug!(model = %name, worker = %heartbeat.node_name, "worker removed from model");
                }
            }
        }

        // Add/update models that are loaded.
        for model in &runtime.loaded_models {
            if !model.ready {
                continue;
            }

            if self.registry.get(&model.name).is_some() {
                // Model exists — add this worker.
                let added = self.registry.add_worker(&model.name, worker_id.clone());
                if added {
                    events.push(ModelSyncEvent::WorkerAdded {
                        model_name: model.name.clone(),
                        worker_id: worker_id.clone(),
                    });
                    debug!(model = %model.name, worker = %heartbeat.node_name, "worker added to model");
                }
            } else {
                // New model — register it.
                let entry = model_entry_from_loaded(model, worker_id);
                self.registry.register_model(entry);
                events.push(ModelSyncEvent::ModelDiscovered {
                    model_name: model.name.clone(),
                    worker_id: worker_id.clone(),
                });
                info!(model = %model.name, worker = %heartbeat.node_name, "new model discovered");
            }
        }

        events
    }

    /// Remove a worker from all models (e.g., on disconnect/timeout).
    pub fn evict_worker(&self, worker_id: &AgentId) -> ModelSyncEvent {
        self.registry.remove_worker_from_all(worker_id);
        ModelSyncEvent::WorkerEvicted {
            worker_id: worker_id.clone(),
        }
    }

    /// Build an `AgentManifest` reflecting current registry state.
    ///
    /// This is the manifest that gets published to other codespaces
    /// so they can discover what models BUNNY can serve.
    pub fn build_manifest(
        &self,
        node_name: &str,
        codespace: Codespace,
    ) -> AgentManifest {
        let mut builder = AgentManifestBuilder::new(node_name, codespace)
            .with_unrestricted_access()
            .with_metadata("synced_models", &self.registry.model_count().to_string());

        // Add all registered models as ModelDescriptors.
        for entry in self.registry.snapshot() {
            let descriptor: ModelDescriptor = entry.into();
            builder.add_model(descriptor);
        }

        // Add all tool names from the codespace.
        for tool in codespace.default_tools() {
            builder.add_tool(tool);
        }

        builder.build()
    }
}

/// Convert a `LoadedModel` from heartbeat into a `ModelEntry`.
fn model_entry_from_loaded(model: &LoadedModel, worker_id: &AgentId) -> ModelEntry {
    // Infer scale from model name patterns.
    let (params_estimate, scale) = estimate_model_params(&model.name);
    let backend = scale.recommended_backend();

    ModelEntry {
        name: model.name.clone(),
        model_hash: TernaryPacket::hash_model_name(&model.name),
        total_params: params_estimate,
        scale,
        backend,
        num_layers: 0, // unknown from heartbeat alone
        input_dim: 0,  // populated when first inference runs
        output_dim: 0,
        workers: vec![worker_id.clone()],
        sharded: false,
        vram_bytes: model.size_bytes.unwrap_or(0) as usize,
    }
}

/// Estimate parameter count from model name conventions.
///
/// Parses common patterns like "llama3.1:8b", "qwen2.5-coder:7b",
/// "threat_classifier" → param count + ModelScale.
fn estimate_model_params(name: &str) -> (usize, ModelScale) {
    let lower = name.to_lowercase();

    // Look for explicit size markers like :8b, :7b, :70b, :1b, etc.
    if let Some(pos) = lower.rfind(':') {
        let suffix = &lower[pos + 1..];
        if let Some(billions) = parse_billions(suffix) {
            let params = (billions * 1_000_000_000.0) as usize;
            return (params, ModelScale::from_param_count(params));
        }
    }

    // Look for size markers in the name itself (e.g., "8b-model").
    for part in lower.split(&['-', '_', '.'][..]) {
        if let Some(billions) = parse_billions(part) {
            let params = (billions * 1_000_000_000.0) as usize;
            return (params, ModelScale::from_param_count(params));
        }
    }

    // Unknown — assume Micro scale (middle tier).
    (10_000_000, ModelScale::Micro)
}

/// Parse a string like "8b", "7b", "70b", "1.5b" into billions.
fn parse_billions(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.ends_with('b') {
        let num_part = &s[..s.len() - 1];
        num_part.parse::<f64>().ok()
    } else {
        None
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_network::{RuntimeHealthInfo, RuntimeKind, WorkerState};

    fn make_runtime(models: Vec<(&str, bool)>) -> RuntimeHealthInfo {
        RuntimeHealthInfo {
            healthy: true,
            kind: RuntimeKind::Ollama,
            loaded_models: models
                .into_iter()
                .map(|(name, ready)| LoadedModel {
                    name: name.to_string(),
                    ready,
                    size_bytes: None,
                })
                .collect(),
            gpu_utilization: 0.25,
            gpu_memory_used: 4_000_000_000,
            gpu_memory_total: 24_000_000_000,
            checked_at_ms: 0,
        }
    }

    fn make_heartbeat(name: &str, runtime: Option<RuntimeHealthInfo>) -> WorkerHeartbeat {
        WorkerHeartbeat {
            node_name: name.to_string(),
            timestamp_ms: 0,
            runtime: runtime.clone(),
            state: match &runtime {
                Some(r) if r.healthy => WorkerState::Healthy,
                Some(_) => WorkerState::Degraded,
                None => WorkerState::Connected,
            },
        }
    }

    // ── Sync basics ────────────────────────────────────────────────

    #[test]
    fn sync_discovers_new_models() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        let runtime = make_runtime(vec![("llama3.1:8b", true), ("qwen2.5:7b", true)]);
        let hb = make_heartbeat("swarm-gpu", Some(runtime));

        let events = syncer.sync_heartbeat(&worker, &hb);

        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], ModelSyncEvent::ModelDiscovered { model_name, .. } if model_name == "llama3.1:8b"));
        assert!(matches!(&events[1], ModelSyncEvent::ModelDiscovered { model_name, .. } if model_name == "qwen2.5:7b"));

        assert_eq!(syncer.registry().model_count(), 2);
        assert!(syncer.registry().get("llama3.1:8b").is_some());
        assert!(syncer.registry().get("qwen2.5:7b").is_some());
    }

    #[test]
    fn sync_adds_worker_to_existing_model() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let w1 = AgentId::new();
        let w2 = AgentId::new();

        // First worker registers model.
        let runtime = make_runtime(vec![("llama3.1:8b", true)]);
        syncer.sync_heartbeat(&w1, &make_heartbeat("gpu-1", Some(runtime.clone())));

        // Second worker also has the model.
        let events = syncer.sync_heartbeat(&w2, &make_heartbeat("gpu-2", Some(runtime)));

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], ModelSyncEvent::WorkerAdded { .. }));
        assert_eq!(syncer.registry().workers_for("llama3.1:8b").len(), 2);
    }

    #[test]
    fn sync_removes_worker_when_model_unloaded() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        // Worker has two models.
        let runtime = make_runtime(vec![("llama3.1:8b", true), ("qwen2.5:7b", true)]);
        syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(runtime)));
        assert_eq!(syncer.registry().model_count(), 2);

        // Worker now only has one model.
        let runtime = make_runtime(vec![("llama3.1:8b", true)]);
        let events = syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(runtime)));

        // Should have a WorkerRemoved event for qwen.
        let removed: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, ModelSyncEvent::WorkerRemoved { .. }))
            .collect();
        assert_eq!(removed.len(), 1);
    }

    #[test]
    fn sync_evicts_unhealthy_worker() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        // Worker has a model.
        let runtime = make_runtime(vec![("llama3.1:8b", true)]);
        syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(runtime)));
        assert_eq!(syncer.registry().workers_for("llama3.1:8b").len(), 1);

        // Worker becomes unhealthy.
        let unhealthy = RuntimeHealthInfo {
            healthy: false,
            kind: RuntimeKind::Ollama,
            loaded_models: vec![],
            gpu_utilization: 0.0,
            gpu_memory_used: 0,
            gpu_memory_total: 0,
            checked_at_ms: 0,
        };
        let events = syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(unhealthy)));

        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], ModelSyncEvent::WorkerEvicted { .. }));
        assert!(syncer.registry().workers_for("llama3.1:8b").is_empty());
    }

    #[test]
    fn sync_ignores_not_ready_models() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        let runtime = make_runtime(vec![("llama3.1:8b", true), ("loading-model", false)]);
        syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(runtime)));

        assert_eq!(syncer.registry().model_count(), 1);
        assert!(syncer.registry().get("loading-model").is_none());
    }

    #[test]
    fn sync_no_runtime_info() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        let events = syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", None));
        assert!(events.is_empty());
    }

    #[test]
    fn evict_worker() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        let runtime = make_runtime(vec![("model_a", true), ("model_b", true)]);
        syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(runtime)));

        let event = syncer.evict_worker(&worker);
        assert!(matches!(event, ModelSyncEvent::WorkerEvicted { .. }));
        assert!(syncer.registry().workers_for("model_a").is_empty());
        assert!(syncer.registry().workers_for("model_b").is_empty());
    }

    // ── Param Estimation ───────────────────────────────────────────

    #[test]
    fn estimate_params_from_name() {
        let (params, scale) = estimate_model_params("llama3.1:8b");
        assert_eq!(params, 8_000_000_000);
        assert_eq!(scale, ModelScale::Pico);

        let (params, scale) = estimate_model_params("qwen2.5-coder:7b");
        assert_eq!(params, 7_000_000_000);
        assert_eq!(scale, ModelScale::Pico);

        let (params, scale) = estimate_model_params("phi3:3.8b");
        assert_eq!(params, 3_800_000_000);
        assert_eq!(scale, ModelScale::Pico);

        let (params, _) = estimate_model_params("unknown_model");
        assert_eq!(params, 10_000_000); // default
    }

    // ── Manifest Building ──────────────────────────────────────────

    #[test]
    fn build_manifest_from_registry() {
        let registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(registry);
        let worker = AgentId::new();

        let runtime = make_runtime(vec![("llama3.1:8b", true), ("qwen2.5:7b", true)]);
        syncer.sync_heartbeat(&worker, &make_heartbeat("gpu", Some(runtime)));

        let manifest = syncer.build_manifest("bunny-edge-01", Codespace::Triton);

        assert_eq!(manifest.node_name, "bunny-edge-01");
        assert!(manifest.unrestricted_access);
        assert_eq!(manifest.model_count(), 2);
        assert!(manifest.has_model("llama3.1:8b"));
        assert!(manifest.has_model("qwen2.5:7b"));
        assert!(!manifest.available_tools.is_empty());
    }
}
