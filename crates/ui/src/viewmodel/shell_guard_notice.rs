//! The Shell tab's index-miss notice: what it says about each query of the script that would
//! scan a collection or sort in memory, as i18n keys and the facts to fill them. Pure; the view
//! draws it. A finding names the collection as the script wrote it and the problem, and shows
//! no more of the query than the verdict's reasons do.

use crate::viewmodel::guard_notice::{WarnModel, finding_model};
use curlybrace_core::guard::GuardReport;

/// One query the check flagged.
#[derive(Debug, Clone, PartialEq)]
pub struct FindingModel {
    pub db: String,
    pub collection: String,
    /// The call's method as the script wrote it (never any of its filter).
    pub method: String,
    pub warn: WarnModel,
}

/// The whole notice.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexMissModel {
    pub findings: Vec<FindingModel>,
    /// The calls that could not be checked; the line is left out when it is 0.
    pub unchecked: usize,
}

/// The notice for `report`; `unchecked` is the total to show (the report's own count is
/// already part of it).
pub fn index_miss_model(report: &GuardReport, unchecked: usize) -> IndexMissModel {
    let findings = report
        .findings
        .iter()
        .map(|finding| FindingModel {
            db: finding.ns.db.clone(),
            collection: finding.ns.collection.clone(),
            method: finding.method.clone(),
            warn: finding_model(finding),
        })
        .collect();
    IndexMissModel {
        findings,
        unchecked,
    }
}

/// The key of the notice's title: it counts the flagged queries.
pub fn title_key(count: usize) -> &'static str {
    match count {
        1 => "shell.guard.title_one",
        _ => "shell.guard.title_other",
    }
}

/// The key of the "N calls could not be checked" line.
pub fn unchecked_key(count: usize) -> &'static str {
    match count {
        1 => "shell.guard.unchecked_one",
        _ => "shell.guard.unchecked_other",
    }
}

#[cfg(test)]
#[path = "shell_guard_notice_tests.rs"]
mod tests;
