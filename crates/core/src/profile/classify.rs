//! Judges a profiler entry with the same rules and thresholds as the explain classifier.

use super::ProfileEntry;
use crate::explain::{FieldOrderFinding, VerdictKind, is_inefficient, summary_keys};
use crate::shape::{Shape, field_order_for_shape, operation_kind};

const COLLSCAN: &str = "COLLSCAN";
const EOF: &str = "EOF";

/// Whether `stage` is one of the stage names in a plan summary such as
/// `IXSCAN { a: 1 }, COLLSCAN { direction: 1 }` (names are separated by punctuation or spaces).
fn has_stage(plan: &str, stage: &str) -> bool {
    plan.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| token == stage)
}

/// Whether the one index the plan summary names has a range key before an exact-match key of
/// the query's `shape`. False whenever the summary does not parse into a single plain index scan.
fn range_before_equality(entry: &ProfileEntry, shape: &Shape) -> bool {
    summary_keys(&entry.plan_summary)
        .and_then(|keys| field_order_for_shape(shape, &keys))
        .is_some_and(|finding| matches!(finding, FieldOrderFinding::RangeBeforeEquality { .. }))
}

/// Classifies a profile entry. `shape` is the value-free shape of its command (the profile only
/// holds the command; its filter and sort tell the equality, sort and range fields).
///
/// Precedence mirrors release A: a collection scan that read nothing is `EmptyCollection`;
/// an unfiltered, unsorted scan is `NoFilter`; a blocking sort is `SortNotCovered`; any other
/// scan is `CollectionScan`; an index whose keys put a range before an exact match is
/// `FieldOrder` (judged from the plan summary alone, and only when it parses); an index plan
/// that examined far more than it returned is
/// `InefficientIndex`; everything else is `IndexUsed`. An op with no plan summary (insert,
/// most commands) did not search for anything, so it is not a miss: it is the neutral
/// `NoFilter`. A continuation (`getMore`) is never judged by its own plan (its command has no
/// filter); it is the neutral `NoFilter` too. A summary naming `COLLSCAN` in any stage is a
/// scan; `EOF` is an `EmptyCollection`; `MULTI_ITERATOR` and other summaries are judged as
/// index plans, so `IndexUsed` unless inefficient. Only an operation that matches documents
/// (find, update, remove, findAndModify) can be inefficient: a distinct, count or aggregate
/// examines many keys to return few values whatever the index, so its index plan is `IndexUsed`.
pub fn classify_entry(entry: &ProfileEntry, shape: &Shape) -> VerdictKind {
    let plan = entry.plan_summary.as_str();
    if plan.is_empty() || entry.is_continuation() {
        return VerdictKind::NoFilter;
    }
    let has_filter = shape.has_filter();
    let scan = has_stage(plan, COLLSCAN);
    if has_stage(plan, EOF) || (scan && entry.docs_examined == 0) {
        VerdictKind::EmptyCollection
    } else if scan && !has_filter && !entry.has_sort_stage {
        VerdictKind::NoFilter
    } else if entry.has_sort_stage {
        VerdictKind::SortNotCovered
    } else if scan {
        VerdictKind::CollectionScan
    } else if range_before_equality(entry, shape) {
        VerdictKind::FieldOrder
    } else if operation_kind(&entry.op, &entry.command).matches_documents()
        && is_inefficient(entry.keys_examined, entry.docs_examined, entry.returned)
    {
        VerdictKind::InefficientIndex
    } else {
        VerdictKind::IndexUsed
    }
}
