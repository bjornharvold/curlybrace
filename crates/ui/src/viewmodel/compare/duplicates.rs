//! The "find duplicate keys" panel: what the last request left behind, and its rows.
use curlybrace_core::compare::protocol::{DuplicateReport, PairId};

use super::pairs::EntryId;
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
    /// A request for `entry` is in flight under `pair`, a fresh id of its own; `stale` when the
    /// setup was edited under it (its answer is dropped).
    Loading {
        entry: EntryId,
        pair: PairId,
        stale: bool,
    },
    /// Both sides answered for `entry`. Empty reports are possible: the panel says nothing was
    /// found.
    Loaded {
        entry: EntryId,
        source: DuplicateReport,
        target: DuplicateReport,
    },
    /// The request failed; the banner carries the error and the panel offers to try again.
    Failed { entry: EntryId },
}

impl DuplicatesView {
    /// Offered only after the run failed on a duplicate key, and never while a request is out.
    pub fn can_find(&self, run_failed_on_duplicate_key: bool) -> bool {
        run_failed_on_duplicate_key && !self.is_loading()
    }

    /// The entry the panel belongs to; none while it is idle.
    pub fn entry(&self) -> Option<EntryId> {
        match self {
            Self::Idle => None,
            Self::Loading { entry, .. } | Self::Loaded { entry, .. } | Self::Failed { entry } => {
                Some(*entry)
            }
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    /// A loaded lookup that found no duplicate group on either side (the data changed since the
    /// run failed, or the key is not the culprit).
    pub fn is_empty_result(&self) -> bool {
        matches!(self, Self::Loaded { source, target, .. }
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
            Self::Loading { entry, pair, .. } => Self::Loading {
                entry: *entry,
                pair: *pair,
                stale: true,
            },
            _ => Self::Idle,
        }
    }
}

#[cfg(test)]
#[path = "duplicates_tests.rs"]
mod tests;
