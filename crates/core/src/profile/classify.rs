//! Judges a profiler entry with the same rules and thresholds as the explain classifier.

use super::ProfileEntry;
use crate::explain::{VerdictKind, is_inefficient};

const COLLSCAN: &str = "COLLSCAN";

/// Classifies a profile entry. `has_filter` is whether the query had a non-empty filter (the
/// profile only holds the command, so the caller derives it from the shape).
///
/// Precedence mirrors release A: a collection scan that read nothing is `EmptyCollection`;
/// an unfiltered, unsorted scan is `NoFilter`; a blocking sort is `SortNotCovered`; any other
/// scan is `CollectionScan`; an index plan that examined far more than it returned is
/// `InefficientIndex`; everything else is `IndexUsed`. An op with no plan summary (insert,
/// most commands) did not search for anything, so it is not a miss: it is the neutral
/// `NoFilter`.
pub fn classify_entry(entry: &ProfileEntry, has_filter: bool) -> VerdictKind {
    let plan = entry.plan_summary.as_str();
    if plan.is_empty() {
        return VerdictKind::NoFilter;
    }
    let scan = plan.starts_with(COLLSCAN);
    if scan && entry.docs_examined == 0 {
        VerdictKind::EmptyCollection
    } else if scan && !has_filter && !entry.has_sort_stage {
        VerdictKind::NoFilter
    } else if entry.has_sort_stage {
        VerdictKind::SortNotCovered
    } else if scan {
        VerdictKind::CollectionScan
    } else if is_inefficient(entry.keys_examined, entry.docs_examined, entry.returned) {
        VerdictKind::InefficientIndex
    } else {
        VerdictKind::IndexUsed
    }
}
