//! The single-pair workspace on top of the multi-pair state: until the pair list has its own
//! views and reducer, the setup form always yields one entry, and the progress panel reads that
//! entry's row. Pure; replaced when the multi-pair views land.
use curlybrace_core::compare::protocol::{PairId, Preflight, RunId};
use curlybrace_core::error::CoreError;

use super::overview::{PairStatus, RunRow};
use super::pairs::{EntryId, PairEntry, PairList, PairOverrides};
use super::preflight::PreflightView;
use super::setup::SetupForm;
use super::{CompareState, ResultsView};

/// The id of the one entry the setup form stands for.
pub const SINGLE_ENTRY: EntryId = 1;

/// The run of the single entry, as the progress panel shows it.
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

/// The one entry the pickers describe; none until both sides are picked and usable.
fn entry_of(setup: &SetupForm) -> Option<PairEntry> {
    let (source, target) = setup.pick_pair().ok()?;
    Some(PairEntry {
        id: SINGLE_ENTRY,
        source,
        target,
        included: true,
        overrides: PairOverrides::default(),
    })
}

/// (scanned source, scanned target, differences) a row shows.
fn counters(status: &PairStatus) -> (u64, u64, u64) {
    match status {
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
    let (scanned_source, scanned_target, differences) = counters(&row.status);
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
    let (scanned_source, scanned_target, _) = counters(&row.status);
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
    /// Replaces the setup form and re-derives the one entry from its pickers.
    pub fn with_setup(self, setup: SetupForm) -> Self {
        let entries = entry_of(&setup).into_iter().collect();
        let pairs = PairList {
            entries,
            next_entry: SINGLE_ENTRY + 1,
            editing: None,
        };
        Self {
            setup,
            pairs,
            ..self
        }
    }

    fn single_row(&self) -> Option<&RunRow> {
        self.overview.row_for(SINGLE_ENTRY)
    }

    pub fn run_view(&self) -> RunView {
        let Some(row) = self.single_row() else {
            return RunView::Idle;
        };
        match (self.is_running(), &row.status) {
            (_, PairStatus::Discarded) => RunView::Idle,
            (true, _) => running_view(row),
            (false, _) => finished_view(row),
        }
    }

    /// The latest verdict of the entry, `Checking` while its check is queued or in flight.
    pub fn preflight_view(&self) -> PreflightView {
        use super::checks::EntryCheck;
        match self.checks.entries.get(&SINGLE_ENTRY) {
            Some(EntryCheck::Queued | EntryCheck::Checking(_)) => PreflightView::Checking,
            Some(EntryCheck::Done(view, _)) => *view,
            _ => PreflightView::Unchecked,
        }
    }

    /// The counts and index status behind an accepted verdict.
    pub fn last_check(&self) -> Option<&Preflight> {
        self.checks.preflight(SINGLE_ENTRY)
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

    /// The runtime pair of the single entry's latest run, while its results may still exist.
    pub fn single_pair(&self) -> Option<PairId> {
        self.single_row().map(|row| row.pair)
    }

    /// The run of the single entry failed on a duplicate key (the only failure the panel
    /// explains).
    pub fn run_failed_on_duplicate_key(&self) -> bool {
        !self.is_running()
            && matches!(
                self.single_row().map(|r| &r.status),
                Some(PairStatus::Failed(CoreError::CompareDuplicateKey))
            )
    }

    /// Whether Find duplicate keys may fire: the run failed on a duplicate key and the setup is
    /// unchanged since (an edit resets the verdict, so Find waits for a fresh check).
    pub fn can_find_duplicates(&self) -> bool {
        self.is_ready()
            && self.checks.allows_start(SINGLE_ENTRY)
            && !self.check_in_flight()
            && self.duplicates.can_find(self.run_failed_on_duplicate_key())
    }
}
