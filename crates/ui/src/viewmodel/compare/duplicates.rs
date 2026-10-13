//! The "find duplicate keys" panel: what the last request left behind, and its rows.
use curlybrace_core::compare::protocol::DuplicateReport;

use super::results::key_label;

/// One duplicate key group, ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateRow {
    pub key_text: String,
    pub count: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum DuplicatesView {
    #[default]
    Idle,
    /// A request is in flight; `stale` when the form was edited under it (its answer is dropped).
    Loading { stale: bool },
    /// Both sides answered. Empty reports are possible: the panel says nothing was found.
    Loaded {
        source: DuplicateReport,
        target: DuplicateReport,
    },
    /// The request failed; the banner carries the error and the panel offers to try again.
    Failed,
}

impl DuplicatesView {
    /// Offered only after the run failed on a duplicate key, and never while a request is out.
    pub fn can_find(&self, run_failed_on_duplicate_key: bool) -> bool {
        run_failed_on_duplicate_key && !self.is_loading()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    /// A loaded lookup that found no duplicate group on either side (the data changed since the
    /// run failed, or the key is not the culprit).
    pub fn is_empty_result(&self) -> bool {
        matches!(self, Self::Loaded { source, target }
            if source.groups.is_empty() && target.groups.is_empty())
    }

    /// The groups of a report as rows, most frequent first (the order the report has).
    pub fn rows(report: &DuplicateReport) -> Vec<DuplicateRow> {
        report
            .groups
            .iter()
            .map(|g| DuplicateRow {
                key_text: key_label(&g.key),
                count: g.count,
            })
            .collect()
    }

    /// The form was edited or the run restarted or discarded: a request in flight is now stale
    /// (its answer is dropped), anything else is out of date.
    pub fn superseded(&self) -> Self {
        match self {
            Self::Loading { .. } => Self::Loading { stale: true },
            _ => Self::Idle,
        }
    }
}

#[cfg(test)]
#[path = "duplicates_tests.rs"]
mod tests;
