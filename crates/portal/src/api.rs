use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[allow(unused_imports)]
use bunny_crypto::swarm::MessagePriority;
#[allow(unused_imports)]
use bunny_crypto::ternary::TernaryPacket;

#[allow(unused_imports)]
use crate::error::PortalError;
use crate::model_registry::ModelRegistry;
use crate::router::{ModelInfo, PriorityHint, RequestRouter};
use crate::session_manager::SessionManager;
use crate::telemetry::PortalTelemetry;

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub router: Arc<RequestRouter>,
    pub sessions: SessionManager,
    pub telemetry: PortalTelemetry,
}

impl AppState {
    pub fn new(registry: ModelRegistry) -> Self {
        Self {
            router: Arc::new(RequestRouter::new(registry)),
            sessions: SessionManager::new(1_800_000), // 30 min idle timeout
            telemetry: PortalTelemetry::new(),
        }
    }
}

// --- Request/Response DTOs ---

#[derive(Deserialize)]
pub struct InferenceRequest {
    /// Model name to run inference on.
    pub model: String,
    /// Input data as f32 array.
    pub input: Vec<f32>,
    /// Optional priority hint.
    pub priority: Option<String>,
    /// Optional session token (for session continuity).
    pub session_token: Option<uuid::Uuid>,
}

#[derive(Serialize)]
pub struct InferenceResponse {
    /// Task ID for tracking.
    pub task_id: uuid::Uuid,
    /// Output logits.
    pub output: Vec<f32>,
    /// Predicted class (argmax).
    pub prediction: usize,
    /// Inference time (microseconds).
    pub inference_time_us: u64,
    /// Model scale tier that processed this request.
    pub scale: String,
    /// Compute backend used.
    pub backend: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub token: uuid::Uuid,
    pub message: String,
}

#[derive(Serialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_ms: u64,
    pub active_sessions: u64,
    pub loaded_models: u64,
    pub active_workers: u64,
    pub pending_tasks: u64,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// --- Handlers ---

/// POST /api/v1/session — Create a new client session.
pub async fn create_session(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (token, _agent_id) = state.sessions.create_session();
    state.telemetry.inc_sessions_created();
    state
        .telemetry
        .set_active_sessions(state.sessions.active_count() as u64);

    (
        StatusCode::CREATED,
        Json(SessionResponse {
            token,
            message: "session created".into(),
        }),
    )
}

/// GET /api/v1/models — List available models.
pub async fn list_models(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let models = state.router.available_models();
    Json(ModelsResponse { models })
}

/// POST /api/v1/infer — Submit an inference request.
pub async fn infer(
    State(state): State<AppState>,
    Json(req): Json<InferenceRequest>,
) -> impl IntoResponse {
    state.telemetry.inc_requests();
    let start = std::time::Instant::now();

    // Parse priority hint.
    let priority = match req.priority.as_deref() {
        Some("critical") => PriorityHint::Critical,
        Some("low_latency") => PriorityHint::LowLatency,
        Some("best_effort") => PriorityHint::BestEffort,
        _ => PriorityHint::Standard,
    };

    // Route the request.
    let route_start = std::time::Instant::now();
    let decision = match state.router.route(&req.model, priority) {
        Ok(d) => d,
        Err(e) => {
            state.telemetry.inc_failed();
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            );
        }
    };
    state
        .telemetry
        .record_routing_time(route_start.elapsed().as_micros() as u64);

    // Validate input dimensions.
    if let Err(e) = state.router.validate_input(&req.model, req.input.len()) {
        state.telemetry.inc_failed();
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        );
    }

    // Build the inference payload.
    let _input_bytes: Vec<u8> = req.input.iter().flat_map(|v| v.to_le_bytes()).collect();
    let task_id = uuid::Uuid::new_v4();

    let elapsed = start.elapsed();

    // In a full deployment, this would submit to SwarmCoordinator and await
    // the InferenceReturn via QUIC. For now, return the dispatched task info.
    state.telemetry.inc_dispatches();
    state.telemetry.inc_success();

    // Record on session if provided.
    if let Some(token) = req.session_token {
        state
            .telemetry
            .record_inference_time(elapsed.as_micros() as u64);
        state.sessions.record_request(&token, elapsed.as_micros() as u64);
    }

    debug!(
        task_id = %task_id,
        model = %req.model,
        scale = ?decision.scale,
        backend = ?decision.backend,
        input_len = req.input.len(),
        "inference dispatched"
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "task_id": task_id,
            "model": req.model,
            "scale": format!("{:?}", decision.scale),
            "backend": format!("{:?}", decision.backend),
            "workers_available": decision.candidate_workers.len(),
            "priority": format!("{:?}", decision.priority),
            "status": "dispatched"
        })),
    )
}

/// GET /api/v1/health — Health check + basic telemetry.
pub async fn health(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let snap = state.telemetry.snapshot();
    Json(HealthResponse {
        status: "ok".into(),
        uptime_ms: snap.uptime_ms,
        active_sessions: snap.active_sessions,
        loaded_models: snap.loaded_models,
        active_workers: snap.active_workers,
        pending_tasks: snap.pending_tasks,
    })
}

/// GET /api/v1/telemetry — Full telemetry snapshot.
pub async fn telemetry(
    State(state): State<AppState>,
) -> impl IntoResponse {
    Json(state.telemetry.snapshot())
}

/// Build the Axum router.
pub fn build_router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/api/v1/session", axum::routing::post(create_session))
        .route("/api/v1/models", axum::routing::get(list_models))
        .route("/api/v1/infer", axum::routing::post(infer))
        .route("/api/v1/health", axum::routing::get(health))
        .route("/api/v1/telemetry", axum::routing::get(telemetry))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::ModelEntry;
    use bunny_agents::ModelScale;

    fn make_test_state() -> AppState {
        let registry = ModelRegistry::new();
        let worker = bunny_crypto::types::AgentId::new();
        registry.register_model(ModelEntry {
            name: "test_model".into(),
            model_hash: TernaryPacket::hash_model_name("test_model"),
            total_params: 8_000_000,
            scale: ModelScale::Micro,
            backend: bunny_agents::ComputeBackend::CudaPacked,
            num_layers: 4,
            input_dim: 4,
            output_dim: 2,
            workers: vec![worker],
            sharded: false,
            vram_bytes: ModelEntry::estimate_vram(8_000_000),
        });
        AppState::new(registry)
    }

    #[test]
    fn build_router_compiles() {
        let state = make_test_state();
        let _router = build_router(state);
    }

    #[test]
    fn state_clone_shares() {
        let state = make_test_state();
        let state2 = state.clone();
        state.telemetry.inc_requests();
        assert_eq!(state2.telemetry.snapshot().requests_total, 1);
    }

    #[test]
    fn priority_parsing() {
        assert_eq!(
            PriorityHint::Critical.to_message_priority(),
            MessagePriority::Critical
        );
    }
}
