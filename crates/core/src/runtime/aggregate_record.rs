//! Recording a pipeline explained in the Aggregate tab as a typed shape.
//!
//! Only the person's own Explain press leads here (a preview is never explained), so this adds no
//! server load. The shape is the one the profiler would make of the same aggregate, so a typed row
//! and a profiled row of one pipeline share a hash. Nothing but the shape and a verdict is kept.

use std::sync::Arc;

use bson::{Bson, Document, doc};

use super::SessionId;
use super::typed_record::record_typed_shape;
use crate::explain::{
    AggPlanOutcome, FieldOrderFinding, INDEX_SCAN_STAGES, InputPlan, VerdictKind,
    parse_aggregate_explain,
};
use crate::history::HistoryDb;
use crate::pipeline::PipelineOptions;
use crate::shape::{Shape, field_order_for_shape, from_profile_command};
use crate::store::Namespace;

const COLLSCAN: &str = "COLLSCAN";
const EOF: &str = "EOF";
const SORT: &str = "SORT";

fn has_stage(input: &InputPlan, stage: &str) -> bool {
    input.plan_stages.iter().any(|s| s == stage)
}

/// Whether the input plan's one index has a range key before an exact-match key of the shape's
/// filter. A plan with several index scans, or no key pattern, is not judged.
fn range_before_equality(input: &InputPlan, shape: &Shape) -> bool {
    let scans = input
        .plan_stages
        .iter()
        .filter(|s| INDEX_SCAN_STAGES.contains(&s.as_str()))
        .count();
    input
        .key_pattern
        .as_ref()
        .filter(|_| scans == 1)
        .and_then(|keys| field_order_for_shape(shape, keys))
        .is_some_and(|finding| matches!(finding, FieldOrderFinding::RangeBeforeEquality { .. }))
}

/// The verdict of a pipeline's input plan, judged like a profiled aggregate: `EmptyCollection`
/// (a scan that read nothing, or `EOF`), `NoFilter` (an unfiltered, unsorted scan),
/// `SortNotCovered` (a `SORT` in the input plan), `CollectionScan`, `FieldOrder` (a range key
/// before an exact-match key of the one index used), else `IndexUsed`. Never `InefficientIndex`:
/// an aggregate examines many keys to return few values whatever the index. `None` for a sharded
/// plan, no input plan or a document that is not understood.
pub(super) fn pipeline_verdict(plan: &AggPlanOutcome, shape: &Shape) -> Option<VerdictKind> {
    let AggPlanOutcome::Plan(plan) = plan else {
        return None;
    };
    if !plan.shards.is_empty() {
        return None;
    }
    let input = plan.input.as_ref()?;
    let scan = has_stage(input, COLLSCAN);
    // The shape holds a `$sort` only when it follows the `$match` directly. Another SORT in the
    // plan (a later `$sort`, a pushed-down stage) is not the shape's, so it is not judged.
    let sorted = has_stage(input, SORT) && shape.sort().is_some();
    Some(
        if has_stage(input, EOF) || (scan && input.docs_examined == Some(0)) {
            VerdictKind::EmptyCollection
        } else if scan && !shape.has_filter() && !sorted {
            VerdictKind::NoFilter
        } else if sorted {
            VerdictKind::SortNotCovered
        } else if scan {
            VerdictKind::CollectionScan
        } else if range_before_equality(input, shape) {
            VerdictKind::FieldOrder
        } else {
            VerdictKind::IndexUsed
        },
    )
}

/// The shape the profiler would record for this pipeline: its leading `$match` and the `$sort`
/// right after it. `None` when the pipeline does not start with a `$match`.
fn pipeline_shape(ns: &Namespace, stages: &[Document]) -> Option<Shape> {
    // Only the first two stages can be part of a shape: copy no more than that.
    let pipeline: Vec<Bson> = stages.iter().take(2).cloned().map(Bson::Document).collect();
    from_profile_command(
        "command",
        &doc! { "aggregate": ns.collection.as_str(), "pipeline": pipeline },
    )
}

/// The shape to record for an explained pipeline, built before the stages are handed to the
/// store. `None` under a hint or a collation (they describe those options, not the shape) and
/// for a pipeline with no leading `$match`.
pub(super) fn shape_to_record(
    ns: &Namespace,
    stages: &[Document],
    options: &PipelineOptions,
) -> Option<Shape> {
    if options.hint.is_some() || options.collation.is_some() {
        return None;
    }
    pipeline_shape(ns, stages)
}

/// Records `shape` with the verdict of the explained plan, in the background. Nothing is
/// recorded when the plan cannot be judged.
pub(super) fn record_pipeline(
    history: &Arc<HistoryDb>,
    session: SessionId,
    ns: &Namespace,
    shape: &Shape,
    explain: &Document,
    now: &dyn Fn() -> i64,
) {
    let Some(kind) = pipeline_verdict(&parse_aggregate_explain(explain), shape) else {
        return;
    };
    record_typed_shape(history, session, ns, shape, kind, now);
}
