//! Index-miss detection: turns a server explain document into a typed [`Verdict`]
//! (`classify`) and proposes an index for the query (`suggest`).

mod agg_model;
mod agg_plan;
mod agg_stage;
mod classify;
#[cfg(test)]
mod classify_sharded_tests;
#[cfg(test)]
mod classify_tests;
pub(crate) mod suggest;
#[cfg(test)]
mod suggest_tests;
mod verdict;
#[cfg(test)]
mod verdict_tests;

pub use agg_model::{
    AGGREGATE_EXPLAIN_MAX_TIME_MS, AggHint, AggPlan, AggPlanOutcome, AggStage, AggSummary,
    ExplainVerbosity, InputPlan,
};
pub use agg_plan::parse_aggregate_explain;
pub use classify::classify;
pub(crate) use classify::{count, is_inefficient};
pub use suggest::suggest;
pub use verdict::{Reason, SuggestedIndex, Verdict, VerdictKind, js_string, worst};
