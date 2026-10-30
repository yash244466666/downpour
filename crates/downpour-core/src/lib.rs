//! Downpour's segmented download engine.
//!
//! The engine is intentionally independent of Tauri so it can be embedded in
//! the desktop app today and a standalone daemon later.

pub mod filename;
pub mod planner;

pub use planner::{TransferPlan, plan_transfer};

/// Engine crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
