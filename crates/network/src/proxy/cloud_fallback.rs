//! Cloud fallback provider — bypass for when swarm-gpu is unavailable.
//!
//! When the GPU worker is dead (heartbeat stale, channel lost, or all
//! ports unreachable), the coordinator can route inference requests to
//! cloud APIs instead of returning "No AI backend available."
//!
//! Supported providers:
//! - **xAI / Grok** (preferred — OpenAI-compatible API at api.x.ai)
//! - **Anthropic / Claude** (via messages API)
//! - **OpenAI / GPT** (via chat completions API)
//!
//! The fallback chain: GPU worker → xAI → Anthropic → OpenAI → error
//!
//! ```text
//! Slack message → Coordinator
//!   → GPU worker DEAD?
//!     → CloudFallbackProvider.query()
//!       → xAI api.x.ai/v1/chat/completions
//!       → Anthropic api.anthropic.com/v1/messages (if xAI fails)
//!       → OpenAI api.openai.com/v1/chat/completions (if Anthropic fails)
//!     → InferenceResponse → ToolResult → Slack reply
//! ```

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::{NetworkError, Result};
use crate::proxy::inference_dispatch::{InferenceRequest, InferenceResponse};

/// Supported cloud inference providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudProvider {
    /// xAI / Grok — OpenAI-compatible (https://api.x.ai/v1)
    Xai,
    /// Anthropic / Claude — messages API
    Anthropic,
    /// OpenAI / GPT — chat completions
    OpenAI,
}

impl CloudProvider {
    /// Base URL for this provider's API.
    pub fn base_url(&self) -> &str {
        match self {
            Self::Xai => "https://api.x.ai/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::OpenAI => "https://api.openai.com/v1",
        }
    }

    /// Default model for this provider.
    pub fn default_model(&self) -> &str {
        match self {
            Self::Xai => "grok-3-mini-fast",
            Self::Anthropic => "claude-sonnet-4-5-20250514",
            Self::OpenAI => "gpt-4o-mini",
        }
    }

    /// Map a local model name to this provider's equivalent.
    pub fn map_model(&self, local_model: &str) -> String {
        // If the model is already a cloud model, keep it
        if local_model.starts_with("grok")
            || local_model.starts_with("claude")
            || local_model.starts_with("gpt")
        {
            return local_model.into();
        }

        // Map local Ollama models to cloud equivalents
        self.default_model().into()
    }

    /// Whether this provider uses OpenAI-compatible API format.
    pub fn is_openai_compatible(&self) -> bool {
        matches!(self, Self::Xai | Self::OpenAI)
    }

    /// Display name for logging.
    pub fn name(&self) -> &str {
        match self {
            Self::Xai => "xAI",
            Self::Anthropic => "Anthropic",
            Self::OpenAI => "OpenAI",
        }
    }
}

/// Configuration for a single cloud provider.
#[derive(Debug, Clone)]
pub struct CloudProviderConfig {
    /// Which provider.
    pub provider: CloudProvider,
    /// API key (e.g., "xai-..." or "sk-...").
    pub api_key: String,
    /// Override model (if None, uses provider default).
    pub model_override: Option<String>,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Whether this provider is enabled.
    pub enabled: bool,
}

impl CloudProviderConfig {
    /// Create a new config for a provider with an API key.
    pub fn new(provider: CloudProvider, api_key: impl Into<String>) -> Self {
        Self {
            provider,
            api_key: api_key.into(),
            model_override: None,
            timeout_secs: 60,
            enabled: true,
        }
    }

    /// Whether this config has a real API key (not a placeholder).
    pub fn has_valid_key(&self) -> bool {
        let key = &self.api_key;
        !key.is_empty()
            && !key.contains("your_")
            && !key.contains("_here")
            && !key.contains("placeholder")
            && key.len() > 10
    }

    /// Resolve the model to use.
    pub fn resolve_model(&self, requested: &str) -> String {
        self.model_override.clone()
            .unwrap_or_else(|| self.provider.map_model(requested))
    }
}

/// The cloud fallback provider — tries multiple cloud APIs in order.
pub struct CloudFallbackProvider {
    /// Ordered list of providers to try (first = preferred).
    providers: Vec<CloudProviderConfig>,
    /// HTTP client.
    client: reqwest::Client,
    /// Total requests served via cloud fallback.
    pub requests_served: u64,
    /// Total failures across all providers.
    pub total_failures: u64,
}

impl CloudFallbackProvider {
    /// Create a new fallback provider with the given configs.
    ///
    /// Only providers with valid API keys are included.
    pub fn new(configs: Vec<CloudProviderConfig>) -> Self {
        let active: Vec<_> = configs.into_iter()
            .filter(|c| c.enabled && c.has_valid_key())
            .collect();

        Self {
            providers: active,
            client: reqwest::Client::new(),
            requests_served: 0,
            total_failures: 0,
        }
    }

    /// Whether any cloud provider is available.
    pub fn is_available(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Number of configured providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// List configured provider names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.provider.name()).collect()
    }

    /// Query cloud providers in order, returning the first successful response.
    ///
    /// Converts an InferenceRequest into cloud API calls and wraps the
    /// response as an InferenceResponse.
    pub async fn query(&mut self, request: &InferenceRequest) -> Result<InferenceResponse> {
        if self.providers.is_empty() {
            return Err(NetworkError::ToolDispatch(
                "no cloud providers configured with valid API keys".into()
            ));
        }

        let mut last_error = String::new();

        for config in &self.providers {
            let start = Instant::now();

            let result = if config.provider.is_openai_compatible() {
                self.query_openai_compatible(config, request).await
            } else {
                self.query_anthropic(config, request).await
            };

            match result {
                Ok((output, tokens)) => {
                    self.requests_served += 1;
                    let elapsed = start.elapsed().as_millis() as u64;
                    return Ok(InferenceResponse::success(
                        &request.request_id,
                        &config.resolve_model(&request.model),
                        output,
                        tokens,
                        elapsed,
                        format!("cloud-{}", config.provider.name()),
                    ));
                }
                Err(e) => {
                    self.total_failures += 1;
                    last_error = format!("{}: {}", config.provider.name(), e);
                    // Try next provider
                    continue;
                }
            }
        }

        // All providers failed
        Ok(InferenceResponse::failure(
            &request.request_id,
            &request.model,
            format!("all cloud providers failed: {}", last_error),
            "cloud-fallback",
        ))
    }

    /// Query an OpenAI-compatible API (xAI, OpenAI).
    async fn query_openai_compatible(
        &self,
        config: &CloudProviderConfig,
        request: &InferenceRequest,
    ) -> std::result::Result<(String, u32), String> {
        let model = config.resolve_model(&request.model);
        let url = format!("{}/chat/completions", config.provider.base_url());

        let mut messages = Vec::new();

        // System prompt if present
        if let Some(sys) = request.params.get("system_prompt") {
            messages.push(serde_json::json!({
                "role": "system",
                "content": sys
            }));
        }

        messages.push(serde_json::json!({
            "role": "user",
            "content": request.prompt
        }));

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, error_body));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("parse error: {e}"))?;

        let content = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let tokens = data["usage"]["completion_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;

        Ok((content, tokens))
    }

    /// Query Anthropic's messages API.
    async fn query_anthropic(
        &self,
        config: &CloudProviderConfig,
        request: &InferenceRequest,
    ) -> std::result::Result<(String, u32), String> {
        let model = config.resolve_model(&request.model);
        let url = format!("{}/messages", config.provider.base_url());

        let mut body = serde_json::json!({
            "model": model,
            "messages": [{
                "role": "user",
                "content": request.prompt
            }],
            "max_tokens": request.max_tokens,
        });

        // System prompt for Anthropic goes in top-level "system" field
        if let Some(sys) = request.params.get("system_prompt") {
            body["system"] = serde_json::Value::String(sys.clone());
        }

        let resp = self.client
            .post(&url)
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, error_body));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| format!("parse error: {e}"))?;

        let content = data["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let tokens = data["usage"]["output_tokens"]
            .as_u64()
            .unwrap_or(0) as u32;

        Ok((content, tokens))
    }
}

/// Build a fallback provider from environment-style key-value pairs.
///
/// Reads: XAI_API_KEY, ANTHROPIC_API_KEY, OPENAI_API_KEY
pub fn build_fallback_from_env(env: &HashMap<String, String>) -> CloudFallbackProvider {
    let mut configs = Vec::new();

    // Priority order: xAI → Anthropic → OpenAI
    if let Some(key) = env.get("XAI_API_KEY") {
        configs.push(CloudProviderConfig::new(CloudProvider::Xai, key));
    }
    if let Some(key) = env.get("ANTHROPIC_API_KEY") {
        configs.push(CloudProviderConfig::new(CloudProvider::Anthropic, key));
    }
    if let Some(key) = env.get("OPENAI_API_KEY") {
        configs.push(CloudProviderConfig::new(CloudProvider::OpenAI, key));
    }

    CloudFallbackProvider::new(configs)
}

/// Convert a cloud fallback response into a ToolResult for OpenClaw.
pub fn cloud_response_to_tool_result(
    response: &InferenceResponse,
    original_request_id: &str,
) -> crate::proxy::tool_call::ToolResult {
    let mut metadata = HashMap::new();
    metadata.insert("worker_node".into(), response.worker_node.clone());
    metadata.insert("model".into(), response.model.clone());
    metadata.insert("tokens_generated".into(), response.tokens_generated.to_string());
    metadata.insert("inference_time_ms".into(), response.inference_time_ms.to_string());
    metadata.insert("fallback".into(), "cloud".into());

    for (k, v) in &response.metadata {
        metadata.insert(k.clone(), v.clone());
    }

    crate::proxy::tool_call::ToolResult {
        request_id: original_request_id.into(),
        tool_name: super::openclaw_bridge::INFERENCE_TOOL.into(),
        success: response.success,
        output_json: if response.success {
            serde_json::json!({
                "output": response.output,
                "model": response.model,
                "tokens": response.tokens_generated,
                "time_ms": response.inference_time_ms,
                "source": "cloud_fallback",
            }).to_string()
        } else {
            String::new()
        },
        error: response.error.clone(),
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunny_crypto::types::AgentId;

    fn sample_request() -> InferenceRequest {
        InferenceRequest::new("llama3.1:8b", "What is Rust?", AgentId::new())
            .with_param("system_prompt", "Be concise.")
    }

    // ── Provider config ─────────────────────────────────────────────

    #[test]
    fn provider_base_urls() {
        assert_eq!(CloudProvider::Xai.base_url(), "https://api.x.ai/v1");
        assert_eq!(CloudProvider::Anthropic.base_url(), "https://api.anthropic.com/v1");
        assert_eq!(CloudProvider::OpenAI.base_url(), "https://api.openai.com/v1");
    }

    #[test]
    fn provider_default_models() {
        assert_eq!(CloudProvider::Xai.default_model(), "grok-3-mini-fast");
        assert_eq!(CloudProvider::Anthropic.default_model(), "claude-sonnet-4-5-20250514");
        assert_eq!(CloudProvider::OpenAI.default_model(), "gpt-4o-mini");
    }

    #[test]
    fn model_mapping_preserves_cloud_models() {
        // Cloud models pass through
        assert_eq!(CloudProvider::Xai.map_model("grok-2"), "grok-2");
        assert_eq!(CloudProvider::Anthropic.map_model("claude-3-opus"), "claude-3-opus");
        assert_eq!(CloudProvider::OpenAI.map_model("gpt-4"), "gpt-4");
    }

    #[test]
    fn model_mapping_converts_local_models() {
        // Local Ollama models get mapped to provider default
        assert_eq!(CloudProvider::Xai.map_model("llama3.1:8b"), "grok-3-mini-fast");
        assert_eq!(CloudProvider::Anthropic.map_model("llama3.1:8b"), "claude-sonnet-4-5-20250514");
        assert_eq!(CloudProvider::OpenAI.map_model("qwen2.5:7b"), "gpt-4o-mini");
    }

    #[test]
    fn openai_compatible_flag() {
        assert!(CloudProvider::Xai.is_openai_compatible());
        assert!(CloudProvider::OpenAI.is_openai_compatible());
        assert!(!CloudProvider::Anthropic.is_openai_compatible());
    }

    // ── API key validation ──────────────────────────────────────────

    #[test]
    fn valid_api_key_detection() {
        let config = CloudProviderConfig::new(CloudProvider::Xai, "xai-real-api-key-12345678");
        assert!(config.has_valid_key());
    }

    #[test]
    fn placeholder_key_rejected() {
        let cases = vec![
            "your_xai_api_key_here",
            "your_anthropic_api_key_here",
            "",
            "short",
            "placeholder_key",
        ];
        for key in cases {
            let config = CloudProviderConfig::new(CloudProvider::Xai, key);
            assert!(!config.has_valid_key(), "key '{}' should be invalid", key);
        }
    }

    // ── Fallback provider construction ──────────────────────────────

    #[test]
    fn empty_provider_not_available() {
        let provider = CloudFallbackProvider::new(vec![]);
        assert!(!provider.is_available());
        assert_eq!(provider.provider_count(), 0);
    }

    #[test]
    fn placeholder_keys_filtered_out() {
        let configs = vec![
            CloudProviderConfig::new(CloudProvider::Xai, "your_xai_api_key_here"),
            CloudProviderConfig::new(CloudProvider::Anthropic, "your_anthropic_api_key_here"),
            CloudProviderConfig::new(CloudProvider::OpenAI, "your_openai_key_here"),
        ];
        let provider = CloudFallbackProvider::new(configs);
        assert!(!provider.is_available());
        assert_eq!(provider.provider_count(), 0);
    }

    #[test]
    fn valid_keys_kept() {
        let configs = vec![
            CloudProviderConfig::new(CloudProvider::Xai, "xai-abcdefghijklmnop123456"),
            CloudProviderConfig::new(CloudProvider::Anthropic, "your_anthropic_api_key_here"),
            CloudProviderConfig::new(CloudProvider::OpenAI, "sk-proj-abcdefghijklmnop123456"),
        ];
        let provider = CloudFallbackProvider::new(configs);
        assert!(provider.is_available());
        assert_eq!(provider.provider_count(), 2); // xAI + OpenAI
        assert_eq!(provider.provider_names(), vec!["xAI", "OpenAI"]);
    }

    #[test]
    fn disabled_provider_excluded() {
        let mut config = CloudProviderConfig::new(CloudProvider::Xai, "xai-real-key-1234567890");
        config.enabled = false;

        let provider = CloudFallbackProvider::new(vec![config]);
        assert!(!provider.is_available());
    }

    // ── build_fallback_from_env ─────────────────────────────────────

    #[test]
    fn build_from_env_with_valid_keys() {
        let env = HashMap::from([
            ("XAI_API_KEY".into(), "xai-real-production-key-12345".into()),
            ("ANTHROPIC_API_KEY".into(), "sk-ant-real-key-67890123456".into()),
            ("OPENAI_API_KEY".into(), "your_openai_key_here".into()), // placeholder
        ]);

        let provider = build_fallback_from_env(&env);
        assert!(provider.is_available());
        assert_eq!(provider.provider_count(), 2); // xAI + Anthropic (OpenAI filtered)
        assert_eq!(provider.provider_names(), vec!["xAI", "Anthropic"]);
    }

    #[test]
    fn build_from_env_all_placeholders() {
        let env = HashMap::from([
            ("XAI_API_KEY".into(), "your_xai_api_key_here".into()),
            ("ANTHROPIC_API_KEY".into(), "your_anthropic_api_key_here".into()),
        ]);

        let provider = build_fallback_from_env(&env);
        assert!(!provider.is_available());
    }

    #[test]
    fn build_from_env_empty() {
        let provider = build_fallback_from_env(&HashMap::new());
        assert!(!provider.is_available());
    }

    // ── No-provider query returns error ─────────────────────────────

    #[tokio::test]
    async fn query_no_providers_returns_error() {
        let mut provider = CloudFallbackProvider::new(vec![]);
        let req = sample_request();
        let result = provider.query(&req).await;
        assert!(result.is_err());
    }

    // ── Response → ToolResult conversion ────────────────────────────

    #[test]
    fn cloud_response_success_to_tool_result() {
        let resp = InferenceResponse::success(
            "infer-cloud-001",
            "grok-3-mini-fast",
            "Rust is a systems programming language.",
            12,
            450,
            "cloud-xAI",
        );

        let result = cloud_response_to_tool_result(&resp, "req-slack-001");

        assert_eq!(result.request_id, "req-slack-001");
        assert_eq!(result.tool_name, "inference");
        assert!(result.success);
        assert_eq!(result.metadata.get("fallback").unwrap(), "cloud");
        assert_eq!(result.metadata.get("worker_node").unwrap(), "cloud-xAI");

        let output: serde_json::Value = serde_json::from_str(&result.output_json).unwrap();
        assert_eq!(output["source"], "cloud_fallback");
        assert_eq!(output["model"], "grok-3-mini-fast");
    }

    #[test]
    fn cloud_response_failure_to_tool_result() {
        let resp = InferenceResponse::failure(
            "infer-cloud-002",
            "grok-3-mini-fast",
            "all cloud providers failed: xAI: HTTP 429",
            "cloud-fallback",
        );

        let result = cloud_response_to_tool_result(&resp, "req-slack-002");

        assert!(!result.success);
        assert!(result.output_json.is_empty());
        assert!(result.error.as_deref().unwrap().contains("429"));
        assert_eq!(result.metadata.get("fallback").unwrap(), "cloud");
    }

    // ── Model resolution ────────────────────────────────────────────

    #[test]
    fn config_model_override() {
        let mut config = CloudProviderConfig::new(CloudProvider::Xai, "xai-real-key-1234567890");
        config.model_override = Some("grok-2-latest".into());

        assert_eq!(config.resolve_model("llama3.1:8b"), "grok-2-latest");
        assert_eq!(config.resolve_model("anything"), "grok-2-latest");
    }

    #[test]
    fn config_default_model_resolution() {
        let config = CloudProviderConfig::new(CloudProvider::Xai, "xai-real-key-1234567890");
        assert_eq!(config.resolve_model("llama3.1:8b"), "grok-3-mini-fast");
        assert_eq!(config.resolve_model("grok-2"), "grok-2"); // cloud model passes through
    }

    // ── Provider name display ───────────────────────────────────────

    #[test]
    fn provider_names_display() {
        assert_eq!(CloudProvider::Xai.name(), "xAI");
        assert_eq!(CloudProvider::Anthropic.name(), "Anthropic");
        assert_eq!(CloudProvider::OpenAI.name(), "OpenAI");
    }

    // ── Serialization ───────────────────────────────────────────────

    #[test]
    fn cloud_provider_serialization() {
        for provider in [CloudProvider::Xai, CloudProvider::Anthropic, CloudProvider::OpenAI] {
            let json = serde_json::to_string(&provider).unwrap();
            let restored: CloudProvider = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, provider);
        }
    }
}
