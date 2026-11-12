//! The index-miss guard: before a query runs, asks the server for a plan-only explain and
//! reports the queries that would scan a collection or sort in memory.

use std::time::Duration;

use tokio::time::Instant;

use crate::explain::{Reason, SuggestedIndex, VerdictKind, classify};
use crate::store::{DocumentStore, Namespace, Query};

/// The most queries one check explains; the rest count as unchecked.
pub const GUARD_MAX_TARGETS: usize = 20;

/// A scan is reported only on a collection of at least this many documents (estimated).
pub const GUARD_MIN_DOCS: u64 = 1000;

/// How long one check may run, in milliseconds. The single source of the budget.
pub const GUARD_BUDGET_MS: u64 = 10_000;

/// One query to check, with the collection it runs against.
#[derive(Clone, Debug, PartialEq)]
pub struct GuardTarget {
    pub ns: Namespace,
    pub query: Query,
}

/// A query the server would answer with a collection scan or an in-memory sort.
#[derive(Clone, Debug, PartialEq)]
pub struct GuardFinding {
    pub ns: Namespace,
    pub query: Query,
    pub kind: VerdictKind,
    pub reasons: Vec<Reason>,
    pub suggestion: Option<SuggestedIndex>,
}

/// What a check found. `unchecked` counts the targets it could not judge: over the cap, past the
/// budget, or whose explain failed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuardReport {
    pub findings: Vec<GuardFinding>,
    pub unchecked: usize,
}

/// Whether a verdict is one the guard warns about.
pub fn warns(kind: VerdictKind) -> bool {
    matches!(
        kind,
        VerdictKind::CollectionScan | VerdictKind::SortNotCovered
    )
}

/// What judging one target came to.
enum Judged {
    Finding(Box<GuardFinding>),
    Fine,
    Unchecked,
}

/// Explains each target in turn (plan only; nothing runs) and reports those that warn.
///
/// A target with no filter and no sort is skipped: there is nothing an index could serve. At most
/// [`GUARD_MAX_TARGETS`] are explained, and none is started once `budget` has elapsed; an explain still
/// running at the deadline is cut off. A scan on a collection estimated below `min_docs` is not
/// reported; with `min_docs` of `None` no count is taken. A failing count keeps the finding.
pub async fn check(
    store: &dyn DocumentStore,
    targets: Vec<GuardTarget>,
    min_docs: Option<u64>,
    budget: Duration,
) -> GuardReport {
    let started = Instant::now();
    let candidates: Vec<GuardTarget> = targets.into_iter().filter(is_checkable).collect();
    let mut report = GuardReport {
        unchecked: candidates.len().saturating_sub(GUARD_MAX_TARGETS),
        ..GuardReport::default()
    };
    let mut pending = candidates.into_iter().take(GUARD_MAX_TARGETS);
    while let Some(target) = pending.next() {
        let left = budget.saturating_sub(started.elapsed());
        if left.is_zero() {
            return GuardReport {
                unchecked: report.unchecked + 1 + pending.count(),
                ..report
            };
        }
        let judged = tokio::time::timeout(left, judge(store, target, min_docs))
            .await
            .unwrap_or(Judged::Unchecked);
        report = absorb(report, judged);
    }
    report
}

fn absorb(report: GuardReport, judged: Judged) -> GuardReport {
    match judged {
        Judged::Finding(f) => GuardReport {
            findings: report.findings.into_iter().chain([*f]).collect(),
            ..report
        },
        Judged::Fine => report,
        Judged::Unchecked => GuardReport {
            unchecked: report.unchecked + 1,
            ..report
        },
    }
}

fn is_checkable(target: &GuardTarget) -> bool {
    !target.query.filter.is_empty() || target.query.sort.is_some()
}

async fn judge(store: &dyn DocumentStore, target: GuardTarget, min_docs: Option<u64>) -> Judged {
    let Ok(explain) = store.explain_plan(&target.ns, &target.query).await else {
        return Judged::Unchecked;
    };
    let Ok(verdict) = classify(&explain.document, &target.query) else {
        return Judged::Unchecked;
    };
    if !warns(verdict.kind) || !large_enough(store, &target.ns, min_docs).await {
        return Judged::Fine;
    }
    Judged::Finding(Box::new(GuardFinding {
        ns: target.ns,
        query: target.query,
        kind: verdict.kind,
        reasons: verdict.reasons,
        suggestion: verdict.suggestion,
    }))
}

/// Whether the collection holds at least `min_docs` documents. No threshold, or a count that
/// fails, keeps the finding: a missed warning costs more than a spurious one.
async fn large_enough(store: &dyn DocumentStore, ns: &Namespace, min_docs: Option<u64>) -> bool {
    match min_docs {
        None => true,
        Some(min) => store.estimated_count(ns).await.map_or(true, |n| n >= min),
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
