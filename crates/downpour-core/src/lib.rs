//! Downpour's download engine.
//!
//! The engine is intentionally independent of Tauri so it can be embedded in
//! the desktop app today and a standalone daemon later.
//!
//! M1 pipeline: **probe -> plan -> transfer -> checkpoint -> finalize**, one
//! stream per download. The writer-thread and checkpoint rules are final; the
//! segmented planner is wired in M2.

pub mod cancel;
pub mod engine;
pub mod error;
pub mod filename;
pub mod paths;
pub mod planner;
pub mod probe;
pub mod ratelimit;
pub mod schedule;
mod segmented;
pub mod writer;

pub use cancel::CancelToken;
pub use engine::{Engine, EngineConfig, ProgressSample, ProgressSink, RetryPolicy, RunOutcome};
pub use error::{EngineError, EngineResult};
pub use planner::{TransferPlan, plan_transfer};
pub use probe::{ProbeResult, probe};

/// Engine crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
