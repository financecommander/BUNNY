//! Model resolver — unified model lookup across local, remote, and cloud.
//!
//! When BUNNY receives an inference request, it needs to find *where*
//! to run the model. The `ModelResolver` checks three sources in order:
//!
//! 1. **Local registry** — GPU workers with model already loaded
//! 2. **Remote manifests** — other codespaces advertising the model
//! 3. **Cloud fallback** — xAI / Anthropic / OpenAI APIs
//!
//! This gives BUNNY/swarm access to AI models across ALL codespaces.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use bunny_agents::{ComputeBackend, ModelScale};
use bunny_crypto::types::AgentId;
use bunny_network::{
    AgentManifest, CloudProvider, Codespace, ModelDescriptor,
};

use crate::model_registry::ModelRegistry;

// ── Route Target ────────────────────────────────────────────────────

/// Where a model can be served from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteTarget {
    /// Model available on local GPU workers.
    Local {
        model_name: String,
        workers: Vec<AgentId>,
        scale: ModelScale,
        backend: ComputeBackend,
    },
    /// Model available on a remote codespace.
    Remote {
        model_name: String,
        codespace: Codespace,
        node_name: String,
    },
    /// Model available via cloud API.
    Cloud {
        model_name: String,
        provider: CloudProvider,
        cloud_model: String,
    },
    /// Model not found anywhere.
    NotFound {
        model_name: String,
    },
}

impl RouteTarget {
    /// Whether this target can serve inference.
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::NotFound { .. })
    }

    /// The model name being resolved.
    pub fn model_name(&self) -> &str {
        match self {
            Self::Local { model_name, .. } => model_name,
            Self::Remote { model_name, .. } => model_name,
            Self::Cloud { model_name, .. } => model_name,
            Self::NotFound { model_name } => model_name,
        }
    }
}

// ── Remote Manifest Cache ───────────────────────────────────────────

/// Cached manifests from remote codespaces.
///
/// Updated when BUNNY receives manifest publications or when
/// it actively fetches from codespace endpoints.
pub struct RemoteManifestCache {
    /// codespace node_name → manifest
    manifests: RwLock<HashMap<String, AgentManifest>>,
}

impl RemoteManifestCache {
    pub fn new() -> Self {
        Self {
            manifests: RwLock::new(HashMap::new()),
        }
    }

    /// Store/update a manifest from a remote codespace.
    pub fn update(&self, manifest: AgentManifest) {
        let name = manifest.node_name.clone();
        info!(
            node = %name,
            codespace = ?manifest.codespace,
            models = manifest.model_count(),
            "cached remote manifest"
        );
        self.manifests.write().unwrap().insert(name, manifest);
    }

    /// Remove a cached manifest.
    pub fn remove(&self, node_name: &str) -> bool {
        self.manifests.write().unwrap().remove(node_name).is_some()
    }

    /// Find a model across all cached manifests.
    pub fn find_model(&self, model_name: &str) -> Option<(Codespace, String, ModelDescriptor)> {
        let lock = self.manifests.read().unwrap();
        for manifest in lock.values() {
            for model in &manifest.models {
                if model.name == model_name {
                    return Some((
                        manifest.codespace,
                        manifest.node_name.clone(),
                        model.clone(),
                    ));
                }
            }
        }
        None
    }

    /// List all available models across all cached manifests.
    pub fn all_models(&self) -> Vec<(Codespace, String, ModelDescriptor)> {
        let lock = self.manifests.read().unwrap();
        let mut result = Vec::new();
        for manifest in lock.values() {
            for model in &manifest.models {
                result.push((
                    manifest.codespace,
                    manifest.node_name.clone(),
                    model.clone(),
                ));
            }
        }
        result
    }

    /// Total cached manifests.
    pub fn manifest_count(&self) -> usize {
        self.manifests.read().unwrap().len()
    }
}

impl Default for RemoteManifestCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── Cloud Model Mapping ─────────────────────────────────────────────

/// Maps local model names to cloud equivalents.
///
/// When GPU workers and remote codespaces can't serve a model,
/// we fall back to cloud APIs — but need to know which cloud model
/// corresponds to the requested local model.
pub struct CloudModelMapping {
    /// local_model_name → (cloud_provider, cloud_model_name)
    mappings: HashMap<String, (CloudProvider, String)>,
}

impl CloudModelMapping {
    /// Create with default mappings for common models.
    pub fn default_mappings() -> Self {
        let mut mappings = HashMap::new();

        // Ollama → xAI (Grok)
        mappings.insert(
            "llama3.1:8b".into(),
            (CloudProvider::Xai, "grok-3-mini-fast".into()),
        );
        mappings.insert(
            "llama3.1:70b".into(),
            (CloudProvider::Xai, "grok-3-mini".into()),
        );

        // Ollama → Anthropic (Claude)
        mappings.insert(
            "qwen2.5-coder:7b".into(),
            (CloudProvider::Anthropic, "claude-sonnet-4-20250514".into()),
        );

        // Ollama → OpenAI
        mappings.insert(
            "codellama:7b".into(),
            (CloudProvider::OpenAI, "gpt-4o-mini".into()),
        );

        Self { mappings }
    }

    /// Empty mapping (no cloud fallback).
    pub fn empty() -> Self {
        Self {
            mappings: HashMap::new(),
        }
    }

    /// Add a custom mapping.
    pub fn add(
        &mut self,
        local_model: impl Into<String>,
        provider: CloudProvider,
        cloud_model: impl Into<String>,
    ) {
        self.mappings
            .insert(local_model.into(), (provider, cloud_model.into()));
    }

    /// Look up cloud equivalent for a local model.
    pub fn resolve(&self, local_model: &str) -> Option<(CloudProvider, String)> {
        self.mappings.get(local_model).cloned()
    }
}

// ── Model Resolver ──────────────────────────────────────────────────

/// Unified model resolver across local, remote, and cloud.
///
/// This is the single entry point for "where can I run model X?"
/// BUNNY/swarm uses this to give ALL codespaces access to AI models.
pub struct ModelResolver {
    /// Local model registry (from worker heartbeat sync).
    local_registry: ModelRegistry,
    /// Cached manifests from remote codespaces.
    remote_cache: RemoteManifestCache,
    /// Cloud model name mappings.
    cloud_mapping: CloudModelMapping,
}

impl ModelResolver {
    /// Create a new resolver.
    pub fn new(
        local_registry: ModelRegistry,
        remote_cache: RemoteManifestCache,
        cloud_mapping: CloudModelMapping,
    ) -> Self {
        Self {
            local_registry,
            remote_cache,
            cloud_mapping,
        }
    }

    /// Resolve a model name to the best available target.
    ///
    /// Priority: local GPU → remote codespace → cloud API → not found.
    pub fn resolve(&self, model_name: &str) -> RouteTarget {
        // 1. Check local registry (GPU workers with model loaded).
        if let Some(entry) = self.local_registry.get(model_name) {
            if !entry.workers.is_empty() {
                debug!(model = %model_name, workers = entry.workers.len(), "resolved to local GPU");
                return RouteTarget::Local {
                    model_name: model_name.to_string(),
                    workers: entry.workers,
                    scale: entry.scale,
                    backend: entry.backend,
                };
            }
        }

        // 2. Check remote codespace manifests.
        if let Some((codespace, node_name, _descriptor)) = self.remote_cache.find_model(model_name) {
            debug!(model = %model_name, codespace = ?codespace, node = %node_name, "resolved to remote codespace");
            return RouteTarget::Remote {
                model_name: model_name.to_string(),
                codespace,
                node_name,
            };
        }

        // 3. Check cloud model mappings.
        if let Some((provider, cloud_model)) = self.cloud_mapping.resolve(model_name) {
            debug!(model = %model_name, provider = ?provider, cloud_model = %cloud_model, "resolved to cloud");
            return RouteTarget::Cloud {
                model_name: model_name.to_string(),
                provider,
                cloud_model,
            };
        }

        // 4. Not found.
        debug!(model = %model_name, "model not found in any source");
        RouteTarget::NotFound {
            model_name: model_name.to_string(),
        }
    }

    /// List all models available across all sources.
    pub fn all_available_models(&self) -> Vec<RouteTarget> {
        let mut targets = Vec::new();

        // Local models.
        for entry in self.local_registry.snapshot() {
            if !entry.workers.is_empty() {
                targets.push(RouteTarget::Local {
                    model_name: entry.name.clone(),
                    workers: entry.workers.clone(),
                    scale: entry.scale,
                    backend: entry.backend,
                });
            }
        }

        // Remote models (skip duplicates already in local).
        let local_names: std::collections::HashSet<String> = self
            .local_registry
            .model_names()
            .into_iter()
            .collect();
        for (codespace, node_name, descriptor) in self.remote_cache.all_models() {
            if !local_names.contains(&descriptor.name) {
                targets.push(RouteTarget::Remote {
                    model_name: descriptor.name,
                    codespace,
                    node_name,
                });
            }
        }

        targets
    }

    /// Get a reference to the local registry.
    pub fn local_registry(&self) -> &ModelRegistry {
        &self.local_registry
    }

    /// Get a reference to the remote cache.
    pub fn remote_cache(&self) -> &RemoteManifestCache {
        &self.remote_cache
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::ModelEntry;
    use bunny_crypto::ternary::TernaryPacket;
    use bunny_network::AgentManifestBuilder;

    fn make_local_registry() -> ModelRegistry {
        let registry = ModelRegistry::new();
        let worker = AgentId::new();

        registry.register_model(ModelEntry {
            name: "llama3.1:8b".into(),
            model_hash: TernaryPacket::hash_model_name("llama3.1:8b"),
            total_params: 8_000_000_000,
            scale: ModelScale::Pico,
            backend: ComputeBackend::CudaPacked,
            num_layers: 32,
            input_dim: 4096,
            output_dim: 4096,
            workers: vec![worker],
            sharded: false,
            vram_bytes: 4_500_000_000,
        });

        registry
    }

    fn make_remote_cache() -> RemoteManifestCache {
        let cache = RemoteManifestCache::new();

        let mut builder = AgentManifestBuilder::new("orchestra-engine", Codespace::Orchestra)
            .with_unrestricted_access();
        builder.add_model(ModelDescriptor {
            name: "workflow-classifier".into(),
            total_params: 2_000_000,
            scale: ModelScale::Cell,
            backend: ComputeBackend::DenseCpu,
            input_dim: 128,
            output_dim: 10,
            num_layers: 3,
            vram_bytes: 0,
        });
        cache.update(builder.build());

        cache
    }

    // ── Resolution ─────────────────────────────────────────────────

    #[test]
    fn resolve_local_model() {
        let resolver = ModelResolver::new(
            make_local_registry(),
            RemoteManifestCache::new(),
            CloudModelMapping::empty(),
        );

        let target = resolver.resolve("llama3.1:8b");
        assert!(target.is_available());
        assert!(matches!(target, RouteTarget::Local { .. }));
        assert_eq!(target.model_name(), "llama3.1:8b");
    }

    #[test]
    fn resolve_remote_model() {
        let resolver = ModelResolver::new(
            ModelRegistry::new(),
            make_remote_cache(),
            CloudModelMapping::empty(),
        );

        let target = resolver.resolve("workflow-classifier");
        assert!(target.is_available());
        assert!(matches!(target, RouteTarget::Remote { codespace: Codespace::Orchestra, .. }));
    }

    #[test]
    fn resolve_cloud_model() {
        let resolver = ModelResolver::new(
            ModelRegistry::new(),
            RemoteManifestCache::new(),
            CloudModelMapping::default_mappings(),
        );

        let target = resolver.resolve("llama3.1:8b");
        assert!(target.is_available());
        assert!(matches!(target, RouteTarget::Cloud { provider: CloudProvider::Xai, .. }));
    }

    #[test]
    fn resolve_not_found() {
        let resolver = ModelResolver::new(
            ModelRegistry::new(),
            RemoteManifestCache::new(),
            CloudModelMapping::empty(),
        );

        let target = resolver.resolve("nonexistent");
        assert!(!target.is_available());
        assert!(matches!(target, RouteTarget::NotFound { .. }));
    }

    #[test]
    fn local_takes_priority_over_cloud() {
        let resolver = ModelResolver::new(
            make_local_registry(),
            RemoteManifestCache::new(),
            CloudModelMapping::default_mappings(),
        );

        // llama3.1:8b is in both local AND cloud — local wins.
        let target = resolver.resolve("llama3.1:8b");
        assert!(matches!(target, RouteTarget::Local { .. }));
    }

    #[test]
    fn remote_takes_priority_over_cloud() {
        let cache = RemoteManifestCache::new();
        let mut builder = AgentManifestBuilder::new("portal", Codespace::AiPortal)
            .with_unrestricted_access();
        builder.add_model(ModelDescriptor {
            name: "llama3.1:8b".into(),
            total_params: 8_000_000_000,
            scale: ModelScale::Pico,
            backend: ComputeBackend::CudaPacked,
            input_dim: 4096,
            output_dim: 4096,
            num_layers: 32,
            vram_bytes: 4_500_000_000,
        });
        cache.update(builder.build());

        let resolver = ModelResolver::new(
            ModelRegistry::new(), // empty local
            cache,
            CloudModelMapping::default_mappings(),
        );

        let target = resolver.resolve("llama3.1:8b");
        assert!(matches!(target, RouteTarget::Remote { .. }));
    }

    // ── List all ───────────────────────────────────────────────────

    #[test]
    fn all_available_models() {
        let resolver = ModelResolver::new(
            make_local_registry(),
            make_remote_cache(),
            CloudModelMapping::empty(),
        );

        let all = resolver.all_available_models();
        assert_eq!(all.len(), 2); // llama3.1:8b local + workflow-classifier remote
    }

    #[test]
    fn all_available_deduplicates() {
        let cache = RemoteManifestCache::new();
        let mut builder = AgentManifestBuilder::new("portal", Codespace::AiPortal)
            .with_unrestricted_access();
        builder.add_model(ModelDescriptor {
            name: "llama3.1:8b".into(), // same as local!
            total_params: 8_000_000_000,
            scale: ModelScale::Pico,
            backend: ComputeBackend::CudaPacked,
            input_dim: 4096,
            output_dim: 4096,
            num_layers: 32,
            vram_bytes: 4_500_000_000,
        });
        cache.update(builder.build());

        let resolver = ModelResolver::new(
            make_local_registry(),
            cache,
            CloudModelMapping::empty(),
        );

        let all = resolver.all_available_models();
        // Should NOT duplicate llama3.1:8b — local takes precedence.
        assert_eq!(all.len(), 1);
        assert!(matches!(&all[0], RouteTarget::Local { .. }));
    }

    // ── Cache ──────────────────────────────────────────────────────

    #[test]
    fn remote_cache_lifecycle() {
        let cache = RemoteManifestCache::new();
        assert_eq!(cache.manifest_count(), 0);

        let manifest = AgentManifestBuilder::new("test", Codespace::Triton).build();
        cache.update(manifest);
        assert_eq!(cache.manifest_count(), 1);

        assert!(cache.remove("test"));
        assert_eq!(cache.manifest_count(), 0);
    }

    // ── Cloud Mapping ──────────────────────────────────────────────

    #[test]
    fn cloud_mapping_default() {
        let mapping = CloudModelMapping::default_mappings();

        let (provider, model) = mapping.resolve("llama3.1:8b").unwrap();
        assert_eq!(provider, CloudProvider::Xai);
        assert_eq!(model, "grok-3-mini-fast");

        assert!(mapping.resolve("nonexistent").is_none());
    }

    #[test]
    fn cloud_mapping_custom() {
        let mut mapping = CloudModelMapping::empty();
        mapping.add("my-model", CloudProvider::Anthropic, "claude-sonnet-4-20250514");

        let (provider, model) = mapping.resolve("my-model").unwrap();
        assert_eq!(provider, CloudProvider::Anthropic);
        assert_eq!(model, "claude-sonnet-4-20250514");
    }
}
