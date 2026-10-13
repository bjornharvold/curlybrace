//! The pure parts of the differences report: what the summary says, which paths a different row
//! changed, and the limits. The runtime (`runtime::compare::report_job`) streams the rows of the
//! stored results through [`CsvWriter`](super::csv::CsvWriter) and does the reading.
//!
//! The per-pair counts come from the stored [`ResultSet`] (they are known before the first row is
//! written, so the summary needs no second pass and no header overwrite); only the identical and
//! scanned counts come from the [`Summary`] the run reported. Paths are rendered from the
//! structured segments of a fresh diff; a value never reaches this module's output. Keys, names
//! and paths are data and are never logged.

use bson::Document;

use super::csv::{PairSummary, ReportHeader};
use crate::compare::diff::{PathSegment, diff_normalized};
use crate::compare::engine::Summary;
use crate::compare::fingerprint::fingerprint_of;
use crate::compare::normalize::normalize;
use crate::compare::protocol::PairSpec;
use crate::compare::results::{Category, DiffEntry, ResultSet};
use crate::error::Result;
use crate::store::Namespace;

/// The most rows a report for the clipboard may hold; above it the person exports to a file.
pub const CLIPBOARD_MAX_ROWS: usize = 10_000;

/// Rows written between two cancel checks (and two progress events) when no document is read.
pub const ROW_BATCH: usize = 500;

/// The categories of a pair in the order the report lists them (each in key order).
pub const REPORT_CATEGORIES: [Category; 3] = [
    Category::OnlyInSource,
    Category::OnlyInTarget,
    Category::Different,
];

/// `db.collection`: the only names a report carries (never a connection name or host).
pub fn display_name(ns: &Namespace) -> String {
    format!("{}.{}", ns.db, ns.collection)
}

/// How many differences `results` holds in all categories.
pub fn row_count(results: &ResultSet) -> usize {
    REPORT_CATEGORIES
        .iter()
        .map(|c| results.count(*c))
        .fold(0, usize::saturating_add)
}

/// The summary of one pair: the category counts are the stored ones (`ResultSet::count`), the
/// identical and scanned counts are what the run reported.
pub fn pair_summary(spec: &PairSpec, results: &ResultSet, reported: Summary) -> PairSummary {
    let count = |c| results.count(c) as u64;
    PairSummary {
        source: display_name(&spec.source),
        target: display_name(&spec.target),
        summary: Summary {
            only_in_source: count(Category::OnlyInSource),
            only_in_target: count(Category::OnlyInTarget),
            different: count(Category::Different),
            ..reported
        },
    }
}

/// The header of a report over `pairs`.
pub fn report_header(label: &str, pairs: Vec<PairSummary>) -> ReportHeader {
    ReportHeader {
        label: label.to_string(),
        pairs,
    }
}

/// What re-reading the two documents of a different row found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangedPaths {
    /// The paths of the fields that differ now, as the compare would show them.
    Paths(Vec<Vec<PathSegment>>),
    /// A document no longer matches the fingerprint the compare recorded (it changed, or the
    /// row has no recorded fingerprint): the paths would describe another comparison.
    Stale,
}

/// The paths that differ between `source` and `target` under the rules of `spec`, if both are
/// still what the compare saw. Never returns a value.
pub fn changed_paths(
    spec: &PairSpec,
    entry: &DiffEntry,
    source: &Document,
    target: &Document,
) -> Result<ChangedPaths> {
    let rules = &spec.rules;
    let (Some(want_source), Some(want_target)) = (entry.source, entry.target) else {
        return Ok(ChangedPaths::Stale);
    };
    if fingerprint_of(source, rules)? != want_source
        || fingerprint_of(target, rules)? != want_target
    {
        return Ok(ChangedPaths::Stale);
    }
    let changes = diff_normalized(
        &normalize(source, rules)?,
        &normalize(target, rules)?,
        rules.ignore_array_order,
    )?;
    // A row recorded as different whose fresh diff finds nothing cannot give an honest empty
    // cell: it is counted as stale.
    if changes.is_empty() {
        return Ok(ChangedPaths::Stale);
    }
    Ok(ChangedPaths::Paths(
        changes.into_iter().map(|c| c.segments).collect(),
    ))
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
