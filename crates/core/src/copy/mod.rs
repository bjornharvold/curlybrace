//! Copy documents between connections. Pure planning lives here; the streaming engine builds on it.

pub mod names;
pub mod policy;

pub use policy::{BatchPlan, ConflictPolicy, ExistingIds, plan_batch};
