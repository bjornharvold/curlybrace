//! The one-pair look of the Compare views until the pair list has views of its own: every
//! accessor here reads the first included entry (the focus). The reducer never uses them; it
//! works on entries and runtime pairs. Pure.
use curlybrace_core::compare::protocol::Preflight;
use curlybrace_core::error::CoreError;

use super::overview::{PairStatus, RunRow};
use super::pairs::EntryId;
use super::preflight::PreflightView;
use super::{CompareState, ResultsView};
use curlybrace_core::compare::protocol::RunId;

/// The run of the focused entry, as the progress panel shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum RunView {
    #[default]
    Idle,
    Running {
        run: RunId,
        finished_pairs: u32,
        scanned_source: u64,
        scanned_target: u64,
        differences: u64,
        cancelled: bool,
        failure: Option<CoreError>,
    },
    Finished {
        run: RunId,
        cancelled: bool,
        failure: Option<CoreError>,
        /// Kept so a run whose filters matched nothing is not mistaken for a real match.
        scanned_source: u64,
        scanned_target: u64,
    },
}

/// (scanned source, scanned target, differences) a row shows; a failed pair keeps its last
/// progress.
fn counters(row: &RunRow) -> (u64, u64, u64) {
    match &row.status {
        PairStatus::Running {
            scanned_source,
            scanned_target,
            differences,
        } => (*scanned_source, *scanned_target, *differences),
        PairStatus::Completed(s) | PairStatus::Cancelled(s) => (
            s.scanned_source,
            s.scanned_target,
            s.only_in_source + s.only_in_target + s.different,
        ),
        PairStatus::Failed(_) => row.last_progress,
        _ => (0, 0, 0),
    }
}

fn markers(status: &PairStatus) -> (bool, Option<CoreError>) {
    match status {
        PairStatus::Cancelled(_) => (true, None),
        PairStatus::Failed(e) => (false, Some(e.clone())),
        _ => (false, None),
    }
}

fn running_view(row: &RunRow) -> RunView {
    let (scanned_source, scanned_target, differences) = counters(row);
    let (cancelled, failure) = markers(&row.status);
    RunView::Running {
        run: row.run,
        finished_pairs: u32::from(!matches!(
            row.status,
            PairStatus::Pending | PairStatus::Running { .. }
        )),
        scanned_source,
        scanned_target,
        differences,
        cancelled,
        failure,
    }
}

fn finished_view(row: &RunRow) -> RunView {
    let (scanned_source, scanned_target, _) = counters(row);
    let (cancelled, failure) = markers(&row.status);
    RunView::Finished {
        run: row.run,
        cancelled,
        failure,
        scanned_source,
        scanned_target,
    }
}

impl CompareState {
    /// The entry the one-pair views show: the first included one.
    pub fn focus_entry(&self) -> Option<EntryId> {
        self.pairs.entries.iter().find(|e| e.included).map(|e| e.id)
    }

    fn focus_row(&self) -> Option<&RunRow> {
        self.overview.row_for(self.focus_entry()?)
    }

    pub fn run_view(&self) -> RunView {
        let Some(row) = self.focus_row() else {
            return RunView::Idle;
        };
        let in_flight = self.overview.active.is_some_and(|a| a.run == row.run);
        match (in_flight, &row.status) {
            (_, PairStatus::Discarded) => RunView::Idle,
            (true, _) => running_view(row),
            (false, _) => finished_view(row),
        }
    }

    /// The latest verdict of the focused entry, `Checking` while its check is queued or out.
    pub fn preflight_view(&self) -> PreflightView {
        use super::checks::EntryCheck;
        let check = self.focus_entry().and_then(|e| self.checks.entries.get(&e));
        match check {
            Some(EntryCheck::Queued | EntryCheck::Checking(_)) => PreflightView::Checking,
            Some(EntryCheck::Done(view, _)) => *view,
            _ => PreflightView::Unchecked,
        }
    }

    /// The counts and index status behind the focused entry's verdict.
    pub fn last_check(&self) -> Option<&Preflight> {
        self.checks.preflight(self.focus_entry()?)
    }

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

    /// Whether Find duplicate keys may fire for `entry`: its run failed on a duplicate key, no
    /// run, check or lookup is out, and its setup is unchanged since (an edit resets the
    /// verdict, so Find waits for a fresh check).
    pub fn can_find_duplicates_for(&self, entry: EntryId) -> bool {
        self.is_ready()
            && !self.is_running()
            && !self.check_in_flight()
            && self.checks.allows_start(entry)
            && self
                .duplicates
                .can_find(self.failed_on_duplicate_key(entry))
    }

    /// `can_find_duplicates_for` the focused entry.
    pub fn can_find_duplicates(&self) -> bool {
        self.focus_entry()
            .is_some_and(|entry| self.can_find_duplicates_for(entry))
    }
}
