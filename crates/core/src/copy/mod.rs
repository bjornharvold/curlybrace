//! Copy documents between connections. Pure planning lives here; the streaming engine builds on it.

pub mod engine;
pub mod names;
pub mod policy;
pub mod preview;
pub mod protocol;

pub use engine::{CopyFailure, CopyJob, CopyOutcome, CopyProgress, CopyReport, copy_collection};
pub use policy::{BatchPlan, ConflictPolicy, ExistingIds, plan_batch};
pub use preview::{PreviewCounts, preview_collection};
pub use protocol::{CopyCommand, CopyEvent, CopyItem, CopyRequest, CopyScope};
