pub mod api;
pub mod error;
pub mod model_registry;
pub mod router;
pub mod session_manager;
pub mod telemetry;

pub use api::{build_router, AppState};
pub use error::{PortalError, Result};
pub use model_registry::{ModelEntry, ModelRegistry};
pub use router::{ModelInfo, PriorityHint, RequestRouter, RouteDecision};
pub use session_manager::{ClientSession, SessionManager};
pub use telemetry::{PortalTelemetry, TelemetrySnapshot};
