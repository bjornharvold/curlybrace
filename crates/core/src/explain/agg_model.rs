//! The typed model of an aggregate explain: what the UI shows of the server's plan.
//!
//! Everything here is derived from the explain document by `parse_aggregate_explain`; nothing
//! carries user data beyond collection and field names.

/// How much the server does for an aggregate explain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplainVerbosity {
    /// Plans only; never runs the pipeline.
    QueryPlanner,
    /// Runs the pipeline (bounded by [`AGGREGATE_EXPLAIN_MAX_TIME_MS`]) and reports counters.
    ExecutionStats,
}

impl ExplainVerbosity {
    /// The word the `explain` command takes.
    pub(crate) fn as_command_str(self) -> &'static str {
        match self {
            Self::QueryPlanner => "queryPlanner",
            Self::ExecutionStats => "executionStats",
        }
    }
}

/// Server-side time limit of an aggregate explain, in milliseconds. Only `ExecutionStats`
/// runs the pipeline, so only it can reach the limit.
pub const AGGREGATE_EXPLAIN_MAX_TIME_MS: u64 = 10_000;

/// The stage that feeds the pipeline: the pushed-down `$cursor` of the old shape, or the
/// winning plan of the newer top-level `queryPlanner`.
#[derive(Clone, Debug, PartialEq)]
pub struct InputPlan {
    /// The access stage (`COLLSCAN`, `IXSCAN`, `IDHACK`, ...), else the top stage of the plan.
    pub stage_kind: String,
    /// The first index the plan reads, when it reads one.
    pub index: Option<String>,
    pub keys_examined: Option<u64>,
    pub docs_examined: Option<u64>,
    pub n_returned: Option<u64>,
}

/// One pipeline stage the server runs after the input.
#[derive(Clone, Debug, PartialEq)]
pub struct AggStage {
    /// The stage operator, `$group`, `$lookup`, ...
    pub name: String,
    /// A short derived description (field names, the joined collection); never values.
    pub detail: Option<String>,
    pub keys_examined: Option<u64>,
    pub docs_examined: Option<u64>,
    pub n_returned: Option<u64>,
    pub millis: Option<u64>,
}

/// The totals of a plan. `None` when the server did not report the figure (plan only).
#[derive(Clone, Debug, PartialEq)]
pub struct AggSummary {
    /// `ExecutionStats` when the document carried execution figures, else `QueryPlanner`.
    pub verbosity_seen: ExplainVerbosity,
    pub total_millis: Option<u64>,
    pub n_returned: Option<u64>,
    pub keys_examined: Option<u64>,
    pub docs_examined: Option<u64>,
}

/// Plain-language findings about a plan. [`AggHint::key`] names the i18n key suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggHint {
    /// The input reads the whole collection.
    CollectionScan,
    /// A blocking sort held in memory (`SORT` plan stage or a `$sort` stage).
    InMemorySort,
    /// A `$sort` or `$group` that spilled to disk.
    DiskSort,
    /// Many more documents examined than the input handed on.
    ExaminedMuchMoreThanReturned,
    /// A `$lookup` that scanned the joined collection.
    LookupCollectionScan,
}

impl AggHint {
    /// A stable snake_case code, the suffix of the `aggregate.explain.hint.*` i18n keys.
    pub fn key(self) -> &'static str {
        match self {
            Self::CollectionScan => "collection_scan",
            Self::InMemorySort => "in_memory_sort",
            Self::DiskSort => "disk_sort",
            Self::ExaminedMuchMoreThanReturned => "examined_much_more_than_returned",
            Self::LookupCollectionScan => "lookup_collection_scan",
        }
    }
}

/// The plan of an aggregate: the input, the stages after it, one plan per shard when sharded.
#[derive(Clone, Debug, PartialEq)]
pub struct AggPlan {
    pub summary: AggSummary,
    pub input: Option<InputPlan>,
    pub stages: Vec<AggStage>,
    /// `(shard name, its plan)`; empty on an unsharded server.
    pub shards: Vec<(String, AggPlan)>,
    pub hints: Vec<AggHint>,
}

/// What reading an explain document came to. An unknown shape is not an error: the Raw view
/// still shows the document.
#[derive(Clone, Debug, PartialEq)]
pub enum AggPlanOutcome {
    Plan(AggPlan),
    NotUnderstood,
}
