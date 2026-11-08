//! Judges a profiler entry with the same rules and thresholds as the explain classifier.

use super::ProfileEntry;
use crate::explain::{VerdictKind, is_inefficient};
use crate::shape::operation_kind;

const COLLSCAN: &str = "COLLSCAN";
const EOF: &str = "EOF";

/// Whether `stage` is one of the stage names in a plan summary such as
/// `IXSCAN { a: 1 }, COLLSCAN { direction: 1 }` (names are separated by punctuation or spaces).
fn has_stage(plan: &str, stage: &str) -> bool {
    plan.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| token == stage)
}

/// Classifies a profile entry. `has_filter` is whether the query had a non-empty filter (the
/// profile only holds the command, so the caller derives it from the shape).
///
/// Precedence mirrors release A: a collection scan that read nothing is `EmptyCollection`;
/// an unfiltered, unsorted scan is `NoFilter`; a blocking sort is `SortNotCovered`; any other
/// scan is `CollectionScan`; an index plan that examined far more than it returned is
/// `InefficientIndex`; everything else is `IndexUsed`. An op with no plan summary (insert,
/// most commands) did not search for anything, so it is not a miss: it is the neutral
/// `NoFilter`. A continuation (`getMore`) is never judged by its own plan (its command has no
/// filter); it is the neutral `NoFilter` too. A summary naming `COLLSCAN` in any stage is a
/// scan; `EOF` is an `EmptyCollection`; `MULTI_ITERATOR` and other summaries are judged as
/// index plans, so `IndexUsed` unless inefficient. Only an operation that matches documents
/// (find, update, remove, findAndModify) can be inefficient: a distinct, count or aggregate
/// examines many keys to return few values whatever the index, so its index plan is `IndexUsed`.
pub fn classify_entry(entry: &ProfileEntry, has_filter: bool) -> VerdictKind {
    let plan = entry.plan_summary.as_str();
    if plan.is_empty() || entry.is_continuation() {
        return VerdictKind::NoFilter;
    }
    let scan = has_stage(plan, COLLSCAN);
    if has_stage(plan, EOF) || (scan && entry.docs_examined == 0) {
        VerdictKind::EmptyCollection
    } else if scan && !has_filter && !entry.has_sort_stage {
        VerdictKind::NoFilter
    } else if entry.has_sort_stage {
        VerdictKind::SortNotCovered
    } else if scan {
        VerdictKind::CollectionScan
    } else if operation_kind(&entry.op, &entry.command).matches_documents()
        && is_inefficient(entry.keys_examined, entry.docs_examined, entry.returned)
    {
        VerdictKind::InefficientIndex
    } else {
        VerdictKind::IndexUsed
    }
}
