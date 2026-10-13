//! Questions the views and the reducer ask of the whole Compare state.
use curlybrace_core::error::CoreError;

use super::overview::PairStatus;
use super::pairs::EntryId;
use super::{CompareState, DuplicatesView, ResultsView};

impl CompareState {
    /// A check is out (even one whose answer will be dropped).
    pub fn check_in_flight(&self) -> bool {
        self.checks.has_pending() || self.orphaned_check.is_some()
    }

    pub fn cancel_requested(&self) -> bool {
        self.overview.active.is_some_and(|a| a.cancel_requested)
    }

    /// The results on show.
    pub fn open_results(&self) -> Option<&ResultsView> {
        self.open_pair.and_then(|pair| self.results.get(&pair))
    }

    /// The entry's latest run failed on a duplicate key (the only failure the panel explains).
    pub fn failed_on_duplicate_key(&self, entry: EntryId) -> bool {
        matches!(
            self.overview.row_for(entry).map(|r| &r.status),
            Some(PairStatus::Failed(CoreError::CompareDuplicateKey))
        )
    }

    /// Re-run may fire for `entry`: it exists and is not queued or running, and neither a run
    /// nor a check is in flight (re-running edits the shared check state).
    pub fn can_rerun(&self, entry: EntryId) -> bool {
        self.pairs.get(entry).is_some()
            && !self.overview.entry_is_live(entry)
            && !self.is_running()
            && !self.check_in_flight()
            && !self.apply.is_busy()
    }

    /// The entry's duplicate-keys lookup failed: its panel offers Try again.
    pub fn duplicates_failed_for(&self, entry: EntryId) -> bool {
        matches!(self.duplicates, DuplicatesView::Failed { entry: e } if e == entry)
    }

    /// Whether Find duplicate keys may fire for `entry`: its run failed on a duplicate key, no
    /// run, check or lookup is out, and its setup is unchanged since (an edit resets the
    /// verdict, so Find waits for a fresh check).
    pub fn can_find_duplicates_for(&self, entry: EntryId) -> bool {
        self.is_ready()
            && !self.is_running()
            && !self.check_in_flight()
            && !self.apply.is_busy()
            && self.checks.allows_start(entry)
            && self
                .duplicates
                .can_find(self.failed_on_duplicate_key(entry))
    }
}
