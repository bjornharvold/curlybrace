//! The notice under the find bar while the index-miss guard holds a Run back: what it says, as
//! i18n keys and the facts to fill them. Pure; the view draws it.

use crate::app::GuardState;
use crate::viewmodel::verdict::kind_presentation;
use curlybrace_core::explain::{Reason, VerdictKind};
use curlybrace_core::guard::GuardFinding;

/// What the notice shows for a guard state.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardNotice {
    /// The plan is being asked for: a quiet line, no buttons.
    Checking,
    /// The query would miss an index: the problem, Run anyway and Cancel.
    Warn(WarnModel),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WarnModel {
    pub icon: &'static str,
    pub title_key: &'static str,
    pub reasons: Vec<Reason>,
    /// A ready-to-copy `createIndex` command, when the check proposed an index.
    pub suggestion: Option<String>,
}

/// The notice for `state`, or `None` when there is nothing to show. Only the first finding is
/// shown: the query bar checks a single target.
pub fn guard_notice(state: &GuardState) -> Option<GuardNotice> {
    match state {
        GuardState::None => None,
        GuardState::Checking { .. } => Some(GuardNotice::Checking),
        GuardState::Warn { report, .. } => report.findings.first().map(warn_model),
    }
}

fn warn_model(finding: &GuardFinding) -> GuardNotice {
    let (icon, verdict_title) = kind_presentation(finding.kind);
    GuardNotice::Warn(WarnModel {
        icon,
        title_key: title_key(finding.kind, verdict_title),
        reasons: finding.reasons.clone(),
        suggestion: finding.suggestion.as_ref().map(|s| s.command(&finding.ns)),
    })
}

/// The sentence of the problem. Only a scan and a blocking sort are raised by the guard; any
/// other kind falls back to its verdict title rather than saying nothing.
fn title_key(kind: VerdictKind, verdict_title: &'static str) -> &'static str {
    match kind {
        VerdictKind::CollectionScan => "query.guard.title_collection_scan",
        VerdictKind::SortNotCovered => "query.guard.title_sort_not_covered",
        _ => verdict_title,
    }
}

#[cfg(test)]
#[path = "guard_notice_tests.rs"]
mod tests;
