//! Manifest exchange — publish and consume agent manifests across codespaces.
//!
//! BUNNY/OpenClaw publishes its `AgentManifest` to every registered codespace
//! so they can discover available agents and models. Remote codespaces publish
//! their manifests back so BUNNY can route inference and tasks cross-codespace.
//!
//! This closes the final loop: BUNNY ↔ all codespaces, bidirectional model access.
//!
//! ```text
//! BUNNY → publishes manifest → AI-PORTAL, Orchestra, ProbFlow, etc.
//!                 ↕
//! Each codespace → publishes manifest back → BUNNY
//!                 ↓
//! ModelResolver consumes remote manifests for unified routing
//! ```

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use bunny_network::{
    AgentManifest, Codespace,
    CodespaceRegistry, AGENT_MANIFEST_PATH,
};

use crate::model_resolver::RemoteManifestCache;
use crate::model_sync::ModelSyncer;

// ── Exchange Events ─────────────────────────────────────────────────

/// Events from manifest exchange operations.
#[derive(Debug, Clone)]
pub enum ManifestExchangeEvent {
    /// Manifest published to a codespace.
    Published {
        target_codespace: Codespace,
        target_node: String,
        model_count: usize,
        agent_count: usize,
    },
    /// Manifest received from a codespace.
    Received {
        source_codespace: Codespace,
        source_node: String,
        model_count: usize,
        agent_count: usize,
    },
    /// Codespace unreachable — publish skipped.
    Unreachable {
        target_codespace: Codespace,
        target_node: String,
    },
}

// ── Publish Request ─────────────────────────────────────────────────

/// A manifest ready to be published to a remote codespace.
///
/// The actual HTTP/gRPC transport is left to the caller — this struct
/// carries the serialized manifest + metadata needed for delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPublishRequest {
    /// Target codespace.
    pub target_codespace: Codespace,
    /// Target node name.
    pub target_node: String,
    /// The endpoint to POST to.
    pub endpoint: String,
    /// Serialized manifest (JSON bytes).
    pub manifest_json: Vec<u8>,
    /// Whether this BUNNY node has unrestricted access.
    pub unrestricted: bool,
}

// ── Manifest Exchange ───────────────────────────────────────────────

/// Bidirectional manifest exchange between BUNNY and all codespaces.
///
/// - `prepare_publish()` builds publish requests for all reachable codespaces
/// - `receive_manifest()` ingests a remote manifest into the cache
/// - Uses `ModelSyncer` to build BUNNY's own manifest from the model registry
pub struct ManifestExchange {
    /// Our node name.
    node_name: String,
    /// Our codespace identity.
    our_codespace: Codespace,
    /// Remote manifest cache (fed into ModelResolver).
    remote_cache: RemoteManifestCache,
}

impl ManifestExchange {
    /// Create a new exchange.
    pub fn new(
        node_name: impl Into<String>,
        our_codespace: Codespace,
        remote_cache: RemoteManifestCache,
    ) -> Self {
        Self {
            node_name: node_name.into(),
            our_codespace,
            remote_cache,
        }
    }

    /// Prepare publish requests for all reachable codespaces.
    ///
    /// Builds BUNNY's manifest from the model syncer and creates
    /// a `ManifestPublishRequest` for each reachable codespace.
    pub fn prepare_publish(
        &self,
        syncer: &ModelSyncer,
        registry: &CodespaceRegistry,
    ) -> (Vec<ManifestPublishRequest>, Vec<ManifestExchangeEvent>) {
        let manifest = syncer.build_manifest(&self.node_name, self.our_codespace);
        let manifest_json = manifest.to_json();

        let mut requests = Vec::new();
        let mut events = Vec::new();

        let snapshots = registry.all_snapshots();

        for snap in &snapshots {
            // Don't publish to ourselves.
            if snap.codespace == self.our_codespace && snap.name == self.node_name {
                continue;
            }

            if snap.state.is_reachable() {
                let endpoint = format!(
                    "http://{}{}",
                    snap.name, // In production: resolve sidecar addr
                    AGENT_MANIFEST_PATH,
                );

                requests.push(ManifestPublishRequest {
                    target_codespace: snap.codespace,
                    target_node: snap.name.clone(),
                    endpoint,
                    manifest_json: manifest_json.clone(),
                    unrestricted: manifest.unrestricted_access,
                });

                events.push(ManifestExchangeEvent::Published {
                    target_codespace: snap.codespace,
                    target_node: snap.name.clone(),
                    model_count: manifest.model_count(),
                    agent_count: manifest.agent_count(),
                });

                debug!(
                    target_node = %snap.name,
                    codespace = ?snap.codespace,
                    models = manifest.model_count(),
                    "prepared manifest publish"
                );
            } else {
                events.push(ManifestExchangeEvent::Unreachable {
                    target_codespace: snap.codespace,
                    target_node: snap.name.clone(),
                });
            }
        }

        info!(
            published = requests.len(),
            unreachable = events.iter().filter(|e| matches!(e, ManifestExchangeEvent::Unreachable { .. })).count(),
            "manifest publish prepared"
        );

        (requests, events)
    }

    /// Receive and cache a manifest from a remote codespace.
    pub fn receive_manifest(&self, manifest: AgentManifest) -> ManifestExchangeEvent {
        let event = ManifestExchangeEvent::Received {
            source_codespace: manifest.codespace,
            source_node: manifest.node_name.clone(),
            model_count: manifest.model_count(),
            agent_count: manifest.agent_count(),
        };

        info!(
            source = %manifest.node_name,
            codespace = ?manifest.codespace,
            models = manifest.model_count(),
            agents = manifest.agent_count(),
            unrestricted = manifest.unrestricted_access,
            "received remote manifest"
        );

        self.remote_cache.update(manifest);
        event
    }

    /// Receive a manifest from raw JSON bytes.
    pub fn receive_manifest_json(&self, data: &[u8]) -> Option<ManifestExchangeEvent> {
        let manifest = AgentManifest::from_json(data)?;
        Some(self.receive_manifest(manifest))
    }

    /// Get a reference to the remote cache.
    pub fn remote_cache(&self) -> &RemoteManifestCache {
        &self.remote_cache
    }

    /// The well-known path for manifest exchange.
    pub fn endpoint_path() -> &'static str {
        AGENT_MANIFEST_PATH
    }

    /// Build a summary of all cached remote manifests.
    pub fn remote_summary(&self) -> Vec<RemoteManifestSummary> {
        let mut result = Vec::new();
        for (codespace, node_name, descriptor) in self.remote_cache.all_models() {
            result.push(RemoteManifestSummary {
                codespace,
                node_name,
                model_name: descriptor.name,
                total_params: descriptor.total_params,
                scale: descriptor.scale,
            });
        }
        result
    }
}

/// Summary of a model available on a remote codespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteManifestSummary {
    pub codespace: Codespace,
    pub node_name: String,
    pub model_name: String,
    pub total_params: usize,
    pub scale: bunny_agents::ModelScale,
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_network::{
        AgentManifestBuilder, CodespaceRegistration,
        ModelDescriptor,
    };
    use bunny_agents::{ComputeBackend, ModelScale};
    use crate::model_registry::ModelRegistry;

    fn setup_registry_with_reachable() -> CodespaceRegistry {
        let registry = CodespaceRegistry::new();

        // Register multiple codespaces.
        let mut spork = CodespaceRegistration::new(
            "swarm-mainframe",
            Codespace::SuperDuperSpork,
            "10.142.0.4:9001".parse().unwrap(),
        );
        // Simulate heartbeat to make reachable.
        spork.record_heartbeat(None);
        registry.register(spork);

        let triton = CodespaceRegistration::new(
            "swarm-gpu",
            Codespace::Triton,
            "10.142.0.6:9003".parse().unwrap(),
        );
        // Triton stays Registered (unreachable).
        registry.register(triton);

        let mut portal = CodespaceRegistration::new(
            "fc-ai-portal",
            Codespace::AiPortal,
            "10.142.0.8:9006".parse().unwrap(),
        );
        portal.record_heartbeat(None);
        registry.register(portal);

        registry
    }

    // ── Publish ────────────────────────────────────────────────────

    #[test]
    fn prepare_publish_to_reachable_codespaces() {
        let model_registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(model_registry);
        let cs_registry = setup_registry_with_reachable();

        let remote_cache = RemoteManifestCache::new();
        let exchange = ManifestExchange::new("bunny-node", Codespace::Triton, remote_cache);

        let (requests, events) = exchange.prepare_publish(&syncer, &cs_registry);

        // swarm-mainframe (Connected) and fc-ai-portal (Connected) are reachable.
        // swarm-gpu (Registered) is NOT reachable.
        let published: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, ManifestExchangeEvent::Published { .. }))
            .collect();
        let unreachable: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, ManifestExchangeEvent::Unreachable { .. }))
            .collect();

        assert_eq!(published.len(), 2);
        assert_eq!(unreachable.len(), 1);
        assert_eq!(requests.len(), 2);

        // All requests should have the manifest endpoint path.
        for req in &requests {
            assert!(req.endpoint.contains(AGENT_MANIFEST_PATH));
            assert!(!req.manifest_json.is_empty());
        }
    }

    #[test]
    fn prepare_publish_skips_self() {
        let model_registry = ModelRegistry::new();
        let syncer = ModelSyncer::new(model_registry);

        let cs_registry = CodespaceRegistry::new();
        let mut self_reg = CodespaceRegistration::new(
            "bunny-node",
            Codespace::Triton,
            "127.0.0.1:9000".parse().unwrap(),
        );
        self_reg.record_heartbeat(None);
        cs_registry.register(self_reg);

        let remote_cache = RemoteManifestCache::new();
        let exchange = ManifestExchange::new("bunny-node", Codespace::Triton, remote_cache);

        let (requests, _events) = exchange.prepare_publish(&syncer, &cs_registry);

        // Should NOT publish to itself.
        assert!(requests.is_empty());
    }

    // ── Receive ────────────────────────────────────────────────────

    #[test]
    fn receive_manifest_caches_it() {
        let remote_cache = RemoteManifestCache::new();
        let exchange = ManifestExchange::new("bunny-node", Codespace::Triton, remote_cache);

        let mut builder = AgentManifestBuilder::new("orchestra-engine", Codespace::Orchestra)
            .with_unrestricted_access();
        builder.add_model(ModelDescriptor {
            name: "workflow-model".into(),
            total_params: 5_000_000,
            scale: ModelScale::Micro,
            backend: ComputeBackend::CudaPacked,
            input_dim: 128,
            output_dim: 10,
            num_layers: 4,
            vram_bytes: 0,
        });
        let manifest = builder.build();

        let event = exchange.receive_manifest(manifest);
        assert!(matches!(event, ManifestExchangeEvent::Received { model_count: 1, .. }));

        // Should be in the remote cache.
        assert_eq!(exchange.remote_cache().manifest_count(), 1);
        let found = exchange.remote_cache().find_model("workflow-model");
        assert!(found.is_some());
        let (cs, node, _) = found.unwrap();
        assert_eq!(cs, Codespace::Orchestra);
        assert_eq!(node, "orchestra-engine");
    }

    #[test]
    fn receive_manifest_json_roundtrip() {
        let remote_cache = RemoteManifestCache::new();
        let exchange = ManifestExchange::new("bunny-node", Codespace::Triton, remote_cache);

        let manifest = AgentManifestBuilder::new("probflow-node", Codespace::ProbFlow)
            .with_metadata("version", "1.0")
            .build();
        let json = manifest.to_json();

        let event = exchange.receive_manifest_json(&json);
        assert!(event.is_some());
        assert_eq!(exchange.remote_cache().manifest_count(), 1);
    }

    // ── Summary ────────────────────────────────────────────────────

    #[test]
    fn remote_summary() {
        let remote_cache = RemoteManifestCache::new();
        let exchange = ManifestExchange::new("bunny-node", Codespace::Triton, remote_cache);

        let mut builder = AgentManifestBuilder::new("portal", Codespace::AiPortal);
        builder.add_model(ModelDescriptor {
            name: "eval-model".into(),
            total_params: 2_000_000,
            scale: ModelScale::Cell,
            backend: ComputeBackend::DenseCpu,
            input_dim: 64,
            output_dim: 5,
            num_layers: 2,
            vram_bytes: 0,
        });
        exchange.receive_manifest(builder.build());

        let summary = exchange.remote_summary();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].model_name, "eval-model");
        assert_eq!(summary[0].codespace, Codespace::AiPortal);
    }

    #[test]
    fn endpoint_path() {
        assert_eq!(ManifestExchange::endpoint_path(), "/bunny/agent/manifest");
    }
}
