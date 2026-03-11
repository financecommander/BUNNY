//! OpenClaw tool registry — maps tool definitions into a Rust-side registry.
//!
//! Routes by name and capability, exposes existence/metadata checks
//! before dispatch. Thread-safe via DashMap for concurrent access.
//!
//! Maps to OpenClaw's `ToolDefinition` / `ToolRegistry` on the Python side.

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::error::{NetworkError, Result};

/// A tool definition — mirrors OpenClaw's `ToolDefinition` dataclass.
///
/// Declared by each node to advertise what tools it can execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name (e.g., "send_email", "ghl_contact").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Capabilities required to execute this tool
    /// (e.g., ["crm_write", "email_send"]).
    pub required_capabilities: Vec<String>,
    /// Tags for discovery/filtering (e.g., ["crm", "outbound"]).
    pub tags: Vec<String>,
    /// Default timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Thread-safe tool registry backed by DashMap.
///
/// Provides concurrent registration, lookup, and capability-based discovery.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Arc<DashMap<String, ToolDefinition>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. Fails if already registered (use `upsert` to overwrite).
    pub fn register(&self, tool: ToolDefinition) -> Result<()> {
        if self.tools.contains_key(&tool.name) {
            return Err(NetworkError::Protocol(format!(
                "tool already registered: {}",
                tool.name
            )));
        }
        self.tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    /// Register or update a tool (no duplicate check).
    pub fn upsert(&self, tool: ToolDefinition) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Get a tool definition by name.
    pub fn get(&self, name: &str) -> Option<ToolDefinition> {
        self.tools.get(name).map(|v| v.clone())
    }

    /// Check if a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// List all registered tools.
    pub fn list(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|v| v.clone()).collect()
    }

    /// Find tools that require a specific capability.
    pub fn find_by_capability(&self, capability: &str) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .filter(|tool| {
                tool.required_capabilities
                    .iter()
                    .any(|c| c == capability)
            })
            .map(|tool| tool.clone())
            .collect()
    }

    /// Find tools matching a tag.
    pub fn find_by_tag(&self, tag: &str) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .filter(|tool| tool.tags.iter().any(|t| t == tag))
            .map(|tool| tool.clone())
            .collect()
    }

    /// Export all tools as a HashMap.
    pub fn export_map(&self) -> HashMap<String, ToolDefinition> {
        self.tools
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Remove a tool by name.
    pub fn remove(&self, name: &str) -> Option<ToolDefinition> {
        self.tools.remove(name).map(|(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email_tool() -> ToolDefinition {
        ToolDefinition {
            name: "send_email".into(),
            description: "Send email via SendGrid".into(),
            required_capabilities: vec!["email_send".into(), "crm_write".into()],
            tags: vec!["crm".into(), "outbound".into()],
            timeout_ms: 30_000,
        }
    }

    fn ghl_tool() -> ToolDefinition {
        ToolDefinition {
            name: "ghl_contact".into(),
            description: "Create GoHighLevel CRM contact".into(),
            required_capabilities: vec!["crm_write".into()],
            tags: vec!["crm".into()],
            timeout_ms: 30_000,
        }
    }

    fn inference_tool() -> ToolDefinition {
        ToolDefinition {
            name: "inference".into(),
            description: "Run model inference via Ollama".into(),
            required_capabilities: vec!["gpu_compute".into()],
            tags: vec!["inference".into(), "ai".into()],
            timeout_ms: 120_000,
        }
    }

    fn tensor_tool() -> ToolDefinition {
        ToolDefinition {
            name: "tensor_ops".into(),
            description: "Ternary tensor operations".into(),
            required_capabilities: vec!["compute".into()],
            tags: vec!["compute".into(), "math".into()],
            timeout_ms: 10_000,
        }
    }

    #[test]
    fn register_tool() {
        let registry = ToolRegistry::new();
        assert!(registry.register(email_tool()).is_ok());
        assert!(registry.contains("send_email"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn duplicate_rejection() {
        let registry = ToolRegistry::new();
        assert!(registry.register(email_tool()).is_ok());
        assert!(registry.register(email_tool()).is_err());
    }

    #[test]
    fn upsert_overwrites() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());

        let mut updated = email_tool();
        updated.description = "Updated description".into();
        registry.upsert(updated);

        let tool = registry.get("send_email").unwrap();
        assert_eq!(tool.description, "Updated description");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn lookup_by_name() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());
        registry.upsert(ghl_tool());

        let found = registry.get("send_email");
        assert!(found.is_some());
        assert_eq!(found.unwrap().description, "Send email via SendGrid");

        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn lookup_by_capability() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());
        registry.upsert(ghl_tool());
        registry.upsert(inference_tool());

        // Both email and ghl require crm_write
        let crm_tools = registry.find_by_capability("crm_write");
        assert_eq!(crm_tools.len(), 2);

        // Only inference requires gpu_compute
        let gpu_tools = registry.find_by_capability("gpu_compute");
        assert_eq!(gpu_tools.len(), 1);
        assert_eq!(gpu_tools[0].name, "inference");

        // Nothing requires "nonexistent"
        assert!(registry.find_by_capability("nonexistent").is_empty());
    }

    #[test]
    fn lookup_by_tag() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());
        registry.upsert(ghl_tool());
        registry.upsert(inference_tool());
        registry.upsert(tensor_tool());

        let crm = registry.find_by_tag("crm");
        assert_eq!(crm.len(), 2);

        let ai = registry.find_by_tag("ai");
        assert_eq!(ai.len(), 1);

        let compute = registry.find_by_tag("compute");
        assert_eq!(compute.len(), 1);
    }

    #[test]
    fn list_all() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());
        registry.upsert(ghl_tool());
        registry.upsert(inference_tool());

        let all = registry.list();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn export_map() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());
        registry.upsert(ghl_tool());

        let map = registry.export_map();
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("send_email"));
        assert!(map.contains_key("ghl_contact"));
    }

    #[test]
    fn remove_tool() {
        let registry = ToolRegistry::new();
        registry.upsert(email_tool());
        assert_eq!(registry.len(), 1);

        let removed = registry.remove("send_email");
        assert!(removed.is_some());
        assert_eq!(registry.len(), 0);
        assert!(!registry.contains("send_email"));

        // Remove nonexistent
        assert!(registry.remove("nonexistent").is_none());
    }

    #[test]
    fn empty_registry() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.list().is_empty());
    }

    #[test]
    fn serialization_roundtrip() {
        let tool = email_tool();
        let json = serde_json::to_string(&tool).unwrap();
        let restored: ToolDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.name, "send_email");
        assert_eq!(restored.required_capabilities.len(), 2);
        assert_eq!(restored.tags.len(), 2);
        assert_eq!(restored.timeout_ms, 30_000);
    }

    #[test]
    fn concurrent_safe() {
        // ToolRegistry uses DashMap — verify Clone + Send + Sync
        let registry = ToolRegistry::new();
        let r2 = registry.clone();

        registry.upsert(email_tool());
        assert!(r2.contains("send_email")); // shared via Arc<DashMap>
    }
}
