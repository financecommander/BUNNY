pub mod agent;
pub mod config;
pub mod error;
pub mod prompts;
pub mod runner;

pub use agent::{Agent, AgentInput, AgentOutput, TernaryAgent};
pub use config::{AgentConfig, AgentRole, ComputeBackend, ModelScale};
pub use error::{AgentError, Result};
pub use runner::AgentRunner;
