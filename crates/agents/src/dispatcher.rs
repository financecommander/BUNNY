//! Jack — the Calculus team's task dispatcher agent.
//!
//! Jack interprets user requests, classifies them into one of the
//! six `bunny-calculus` task categories, and routes to SWARM for
//! execution. Jack **cannot** modify code, repos, VMs, or
//! infrastructure — he is a pure classifier and task router.

use serde::{Deserialize, Serialize};

use crate::agent::{Agent, AgentInput, AgentOutput};
use crate::config::{AgentRole, ComputeBackend};
use crate::error::Result;

// ── Task Categories ─────────────────────────────────────────────────

/// The six Calculus tool categories Jack can route to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskCategory {
    /// Gradient apply, clip, alignment, norm, STE.
    Gradient,
    /// Quantization analysis, sparsity thresholds, weight balance, entropy.
    Optimizer,
    /// Cosine similarity, Jaccard index, k-NN, sign agreement.
    Similarity,
    /// Batch stats, EMA, percentile, IQR outlier detection.
    Statistics,
    /// Symbolic expressions, policy rule evaluation.
    Symbolic,
    /// Ternary add/dot, hamming distance, L1 norm, sparsity.
    TensorOps,
}

impl TaskCategory {
    /// Category index for prediction output (0–5).
    pub fn index(self) -> usize {
        match self {
            Self::Gradient => 0,
            Self::Optimizer => 1,
            Self::Similarity => 2,
            Self::Statistics => 3,
            Self::Symbolic => 4,
            Self::TensorOps => 5,
        }
    }

    /// All variants.
    pub fn all() -> &'static [TaskCategory] {
        &[
            Self::Gradient,
            Self::Optimizer,
            Self::Similarity,
            Self::Statistics,
            Self::Symbolic,
            Self::TensorOps,
        ]
    }

    /// Well-known tools available in this category.
    pub fn tools(self) -> &'static [&'static str] {
        match self {
            Self::Gradient => &[
                "apply_gradient",
                "clip_gradient",
                "gradient_alignment",
                "gradient_norm",
                "ste_gradient",
            ],
            Self::Optimizer => &[
                "analyze_quantization",
                "find_threshold_for_sparsity",
                "weight_balance",
                "weight_entropy",
            ],
            Self::Similarity => &[
                "cosine_similarity",
                "jaccard_index",
                "knn_cosine",
                "sign_agreement",
            ],
            Self::Statistics => &[
                "batch_mean",
                "batch_stddev",
                "batch_variance",
                "iqr_outliers",
                "percentile",
                "ema",
                "running_stats",
            ],
            Self::Symbolic => &[
                "symbolic_parse",
                "policy_rule_evaluate",
            ],
            Self::TensorOps => &[
                "ternary_add_saturate",
                "ternary_dot",
                "ternary_hamming",
                "ternary_l1_norm",
                "ternary_sparsity",
            ],
        }
    }

    /// Display name for this category.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gradient => "gradient",
            Self::Optimizer => "optimizer",
            Self::Similarity => "similarity",
            Self::Statistics => "statistics",
            Self::Symbolic => "symbolic",
            Self::TensorOps => "tensor_ops",
        }
    }
}

// ── Task Classification ─────────────────────────────────────────────

/// Result of Jack classifying a user request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskClassification {
    /// Which Calculus category this request maps to.
    pub category: String,
    /// Best-match tool name within the category.
    pub tool: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
    /// Human-readable reason for the classification.
    pub reason: String,
}

// ── Keyword-based classifier ────────────────────────────────────────

/// Keyword sets for each category (used by `classify_request`).
const GRADIENT_KEYWORDS: &[&str] = &[
    "gradient", "grad", "backprop", "clip", "alignment",
    "norm", "ste", "straight-through",
];

const OPTIMIZER_KEYWORDS: &[&str] = &[
    "quantiz", "sparsity", "threshold", "weight_balance",
    "entropy", "optimi", "prune", "compress",
];

const SIMILARITY_KEYWORDS: &[&str] = &[
    "similar", "cosine", "jaccard", "knn", "k-nn", "nearest",
    "sign_agreement", "sign agreement", "distance", "embedding",
];

const STATISTICS_KEYWORDS: &[&str] = &[
    "stats", "statistic", "mean", "stddev", "variance",
    "outlier", "iqr", "percentile", "ema", "batch",
    "anomal", "distribution",
];

const SYMBOLIC_KEYWORDS: &[&str] = &[
    "symbolic", "symbol", "expression", "policy", "rule",
    "parse", "evaluate", "formula",
];

const TENSOR_OPS_KEYWORDS: &[&str] = &[
    "ternary", "tensor", "hamming", "l1_norm", "l1 norm",
    "dot", "add_saturate", "sparsity_measure",
    "dot product", "add saturate",
];

/// Score how many keywords from `keywords` appear in `request`.
fn keyword_score(request: &str, keywords: &[&str]) -> f32 {
    let lower = request.to_lowercase();
    let hits = keywords.iter().filter(|kw| lower.contains(**kw)).count();
    if keywords.is_empty() {
        return 0.0;
    }
    (hits as f32) / (keywords.len() as f32)
}

/// Classify a user request into a [`TaskClassification`].
///
/// Uses keyword matching against each category's keyword set.
/// Returns the best match, or a low-confidence fallback if nothing
/// matches well.
pub fn classify_request(request: &str) -> TaskClassification {
    let categories: &[(TaskCategory, &[&str])] = &[
        (TaskCategory::Gradient, GRADIENT_KEYWORDS),
        (TaskCategory::Optimizer, OPTIMIZER_KEYWORDS),
        (TaskCategory::Similarity, SIMILARITY_KEYWORDS),
        (TaskCategory::Statistics, STATISTICS_KEYWORDS),
        (TaskCategory::Symbolic, SYMBOLIC_KEYWORDS),
        (TaskCategory::TensorOps, TENSOR_OPS_KEYWORDS),
    ];

    let mut best_category = TaskCategory::Statistics; // fallback
    let mut best_score: f32 = 0.0;

    for &(cat, keywords) in categories {
        let score = keyword_score(request, keywords);
        if score > best_score {
            best_score = score;
            best_category = cat;
        }
    }

    // Map best-score to confidence (multiply by ~4 to scale, clamp to [0.3, 0.99]).
    let raw_confidence = (best_score * 4.0).clamp(0.3, 0.99);

    // Pick the first tool in the best category as best guess.
    let best_tool = best_category.tools().first().unwrap_or(&"unknown");

    // Try to find a more specific tool match.
    let lower = request.to_lowercase();
    let matched_tool = best_category
        .tools()
        .iter()
        .find(|t| {
            let normalized = t.replace('_', " ");
            lower.contains(&normalized) || lower.contains(**t)
        })
        .unwrap_or(best_tool);

    TaskClassification {
        category: best_category.as_str().to_string(),
        tool: matched_tool.to_string(),
        confidence: raw_confidence,
        reason: format!(
            "{} request routed to {} category",
            matched_tool, best_category.as_str()
        ),
    }
}

// ── JackDispatcher Agent ────────────────────────────────────────────

/// Jack — the Calculus team's task dispatcher.
///
/// Implements the [`Agent`] trait as a pure classifier: reads the
/// `"request"` key from input metadata, classifies it, and returns
/// a JSON `TaskClassification` in `AgentOutput::result`.
///
/// Jack has **no** `TernaryEngine` — he is not an inference agent.
/// He only reads input and produces a classification with zero side
/// effects.
pub struct JackDispatcher;

impl Agent for JackDispatcher {
    fn name(&self) -> &str {
        "jack"
    }

    fn role(&self) -> AgentRole {
        AgentRole::Dispatcher
    }

    fn system_prompt(&self) -> &str {
        crate::prompts::DISPATCHER_PROMPT
    }

    fn process(&self, input: &AgentInput) -> Result<AgentOutput> {
        // Extract the user request from metadata.
        let request = input
            .metadata
            .get("request")
            .map(|s| s.as_str())
            .unwrap_or("");

        let classification = classify_request(request);
        let category_index = TaskCategory::all()
            .iter()
            .position(|c| c.as_str() == classification.category)
            .unwrap_or(0);

        let result_json = serde_json::to_string(&classification)
            .unwrap_or_else(|_| "{}".to_string());

        // Output data: one-hot-ish confidence vector over 6 categories.
        let mut data = vec![0.0f32; 6];
        data[category_index] = classification.confidence;

        Ok(AgentOutput {
            data,
            prediction: category_index,
            inference_time_us: 0, // pure classification, no inference
            backend: ComputeBackend::DenseCpu,
            result: Some(result_json),
        })
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentInput;
    use std::collections::HashMap;

    fn make_input(request: &str) -> AgentInput {
        let mut metadata = HashMap::new();
        metadata.insert("request".to_string(), request.to_string());
        AgentInput {
            data: vec![],
            metadata,
        }
    }

    // ── Identity ───────────────────────────────────────────────────

    #[test]
    fn jack_identity() {
        let jack = JackDispatcher;
        assert_eq!(jack.name(), "jack");
        assert_eq!(jack.role(), AgentRole::Dispatcher);
        assert!(!jack.system_prompt().is_empty());
        assert!(jack.system_prompt().contains("Jack"));
        assert!(jack.system_prompt().contains("SWARM"));
    }

    // ── Category Classification ────────────────────────────────────

    #[test]
    fn classify_gradient_request() {
        let c = classify_request("compute the gradient norm of these weights");
        assert_eq!(c.category, "gradient");
        assert!(c.confidence > 0.3);
    }

    #[test]
    fn classify_optimizer_request() {
        let c = classify_request("analyze quantization thresholds for sparsity");
        assert_eq!(c.category, "optimizer");
        assert!(c.confidence > 0.3);
    }

    #[test]
    fn classify_similarity_request() {
        let c = classify_request("how similar are these two embeddings? cosine similarity");
        assert_eq!(c.category, "similarity");
        assert!(c.confidence > 0.3);
    }

    #[test]
    fn classify_statistics_request() {
        let c = classify_request("detect outliers in the batch distribution using IQR");
        assert_eq!(c.category, "statistics");
        assert!(c.confidence > 0.3);
    }

    #[test]
    fn classify_symbolic_request() {
        let c = classify_request("evaluate this policy rule expression");
        assert_eq!(c.category, "symbolic");
        assert!(c.confidence > 0.3);
    }

    #[test]
    fn classify_tensor_ops_request() {
        let c = classify_request("compute ternary dot product of these vectors");
        assert_eq!(c.category, "tensor_ops");
        assert!(c.confidence > 0.3);
    }

    #[test]
    fn classify_unknown_request() {
        let c = classify_request("hello, what's the weather like?");
        // Should still return a classification, just with low confidence.
        assert!(c.confidence <= 0.5);
        assert!(!c.category.is_empty());
    }

    // ── Agent::process roundtrip ───────────────────────────────────

    #[test]
    fn jack_process_returns_classification() {
        let jack = JackDispatcher;
        let input = make_input("compute the gradient norm");
        let output = jack.process(&input).unwrap();

        assert_eq!(output.prediction, 0); // gradient = index 0
        assert_eq!(output.data.len(), 6);
        assert!(output.data[0] > 0.0); // gradient slot has confidence
        assert!(output.result.is_some());

        let result: TaskClassification =
            serde_json::from_str(output.result.as_ref().unwrap()).unwrap();
        assert_eq!(result.category, "gradient");
    }

    #[test]
    fn jack_process_empty_request() {
        let jack = JackDispatcher;
        let input = AgentInput {
            data: vec![],
            metadata: HashMap::new(),
        };
        let output = jack.process(&input).unwrap();
        // Should still produce valid output (low-confidence fallback).
        assert_eq!(output.data.len(), 6);
        assert!(output.result.is_some());
    }

    // ── TaskCategory tools ─────────────────────────────────────────

    #[test]
    fn task_category_tools() {
        assert!(TaskCategory::Gradient.tools().contains(&"gradient_norm"));
        assert!(TaskCategory::Gradient.tools().contains(&"clip_gradient"));

        assert!(TaskCategory::Optimizer.tools().contains(&"analyze_quantization"));
        assert!(TaskCategory::Optimizer.tools().contains(&"weight_entropy"));

        assert!(TaskCategory::Similarity.tools().contains(&"cosine_similarity"));
        assert!(TaskCategory::Similarity.tools().contains(&"jaccard_index"));

        assert!(TaskCategory::Statistics.tools().contains(&"iqr_outliers"));
        assert!(TaskCategory::Statistics.tools().contains(&"percentile"));

        assert!(TaskCategory::Symbolic.tools().contains(&"policy_rule_evaluate"));

        assert!(TaskCategory::TensorOps.tools().contains(&"ternary_dot"));
        assert!(TaskCategory::TensorOps.tools().contains(&"ternary_hamming"));
    }

    #[test]
    fn task_category_all_variants() {
        assert_eq!(TaskCategory::all().len(), 6);
    }

    #[test]
    fn task_category_indices() {
        assert_eq!(TaskCategory::Gradient.index(), 0);
        assert_eq!(TaskCategory::Optimizer.index(), 1);
        assert_eq!(TaskCategory::Similarity.index(), 2);
        assert_eq!(TaskCategory::Statistics.index(), 3);
        assert_eq!(TaskCategory::Symbolic.index(), 4);
        assert_eq!(TaskCategory::TensorOps.index(), 5);
    }
}
