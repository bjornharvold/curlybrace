//! The latest run as the overview shows it: one row per pair entry that has run, each under
//! the fresh `PairId` it ran with, and the one run in flight. Pure; every step returns a new
//! overview, and events for another run or pair change nothing.
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::protocol::{PairId, PairOutcome, RunId};
use curlybrace_core::error::CoreError;

use super::pairs::EntryId;

#[derive(Clone, Debug, PartialEq)]
pub enum PairStatus {
    Pending,
    Running {
        scanned_source: u64,
        scanned_target: u64,
        differences: u64,
    },
    Completed(Summary),
    Cancelled(Summary),
    Failed(CoreError),
    Discarded,
}

impl PairStatus {
    fn is_live(&self) -> bool {
        matches!(self, Self::Pending | Self::Running { .. })
    }

    /// Finished. Only a completed pair holds results in the runtime; Discard for a cancelled
    /// or failed one is harmless (idempotent) and keeps the reducer's bookkeeping uniform.
    fn is_settled(&self) -> bool {
        matches!(
            self,
            Self::Completed(_) | Self::Cancelled(_) | Self::Failed(_)
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunRow {
    pub entry: EntryId,
    pub pair: PairId,
    pub run: RunId,
    pub status: PairStatus,
    /// Last (scanned source, scanned target, differences) reported while running; kept when
    /// the pair ends without a summary (failed).
    pub last_progress: (u64, u64, u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveRun {
    pub run: RunId,
    pub cancel_requested: bool,
    /// The failure banner was raised for a pair of this run (once per run).
    pub failure_noted: bool,
    /// How many pairs the run started (a snapshot: rows change as pairs end).
    pub pairs: usize,
}

/// Sums over the completed pairs, plus how many pairs ended each way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub only_in_source: u64,
    pub only_in_target: u64,
    pub different: u64,
    pub identical: u64,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOverview {
    pub rows: Vec<RunRow>,
    pub active: Option<ActiveRun>,
}

impl RunOverview {
    /// Begins `run` over `pairs`: each entry's row is replaced by a Pending one, rows of other
    /// entries stay. Returns the pairs the replaced rows still held, for the reducer to discard.
    pub fn started(self, run: RunId, pairs: &[(EntryId, PairId)]) -> (Self, Vec<PairId>) {
        let in_run = |entry: EntryId| pairs.iter().any(|(e, _)| *e == entry);
        let superseded = self
            .rows
            .iter()
            .filter(|r| in_run(r.entry) && r.status != PairStatus::Discarded)
            .map(|r| r.pair)
            .collect();
        let fresh = |entry, pair| RunRow {
            entry,
            pair,
            run,
            status: PairStatus::Pending,
            last_progress: (0, 0, 0),
        };
        let mut replaced = Vec::new();
        let mut rows: Vec<RunRow> = self
            .rows
            .into_iter()
            .map(|row| match pairs.iter().find(|(e, _)| *e == row.entry) {
                Some((entry, pair)) => {
                    replaced.push(*entry);
                    fresh(*entry, *pair)
                }
                None => row,
            })
            .collect();
        rows.extend(
            pairs
                .iter()
                .filter(|(e, _)| !replaced.contains(e))
                .map(|(entry, pair)| fresh(*entry, *pair)),
        );
        let next = Self {
            rows,
            active: Some(ActiveRun {
                run,
                cancel_requested: false,
                failure_noted: false,
                pairs: pairs.len(),
            }),
        };
        (next, superseded)
    }

    /// Progress of the active run's pair; anything else is stale.
    pub fn progressed(self, run: RunId, pair: PairId, (src, tgt, found): (u64, u64, u64)) -> Self {
        if self.active.map(|a| a.run) != Some(run) {
            return self;
        }
        let status = PairStatus::Running {
            scanned_source: src,
            scanned_target: tgt,
            differences: found,
        };
        self.with_live_row(run, pair, |row| RunRow {
            status,
            last_progress: (src, tgt, found),
            ..row
        })
    }

    /// The pair's terminal; ignored unless `run` is the active run and the pair its live row.
    pub fn pair_done(self, run: RunId, pair: PairId, outcome: PairOutcome) -> Self {
        if self.active.map(|a| a.run) != Some(run) {
            return self;
        }
        let status = match outcome {
            PairOutcome::Completed(s) => PairStatus::Completed(s),
            PairOutcome::Cancelled(s) => PairStatus::Cancelled(s),
            PairOutcome::Failed(e) => PairStatus::Failed(e),
        };
        self.with_live_row(run, pair, |row| RunRow { status, ..row })
    }

    /// The run's terminal. No pair of it stays live: one the runtime never reported ends as
    /// cancelled when Cancel was sent, otherwise as failed with the runtime's own
    /// `Database("internal error")` (a pair lost without an outcome), so such a row can
    /// always be removed or re-run.
    pub fn run_done(self, run: RunId) -> Self {
        let Some(active) = self.active.filter(|a| a.run == run) else {
            return self;
        };
        let lost = match active.cancel_requested {
            true => PairStatus::Cancelled(Summary::default()),
            false => PairStatus::Failed(CoreError::Database("internal error".into())),
        };
        let rows = self
            .rows
            .into_iter()
            .map(|row| match row.run == run && row.status.is_live() {
                true => RunRow {
                    status: lost.clone(),
                    ..row
                },
                false => row,
            })
            .collect();
        Self { rows, active: None }
    }

    /// The runtime refused the whole start of the active run: its rows that never began end
    /// as failed and the run is over. Rows of other runs, and their results, stay.
    pub fn start_refused(self, error: &CoreError) -> Self {
        let Some(active) = self.active else {
            return self;
        };
        let rows = self
            .rows
            .into_iter()
            .map(
                |row| match row.run == active.run && row.status == PairStatus::Pending {
                    true => RunRow {
                        status: PairStatus::Failed(error.clone()),
                        ..row
                    },
                    false => row,
                },
            )
            .collect();
        Self { rows, active: None }
    }

    /// Cancel was sent for the active run; a second call changes nothing.
    pub fn cancelling(self) -> Self {
        Self {
            active: self.active.map(|a| ActiveRun {
                cancel_requested: true,
                ..a
            }),
            ..self
        }
    }

    /// The pair's results were thrown away. Live rows cannot be discarded.
    pub fn discarded(self, pair: PairId) -> Self {
        let rows = self
            .rows
            .into_iter()
            .map(|row| match row.pair == pair && row.status.is_settled() {
                true => RunRow {
                    status: PairStatus::Discarded,
                    ..row
                },
                false => row,
            })
            .collect();
        Self { rows, ..self }
    }

    /// Drops the entry's row; returns its pair when the runtime may still hold its results.
    pub fn removed_entry(self, entry: EntryId) -> (Self, Option<PairId>) {
        let held = self
            .rows
            .iter()
            .find(|r| r.entry == entry && r.status.is_settled())
            .map(|r| r.pair);
        let rows = self.rows.into_iter().filter(|r| r.entry != entry).collect();
        (Self { rows, ..self }, held)
    }

    /// The entry has a row that has not ended yet (queued or running in the active run).
    pub fn entry_is_live(&self, entry: EntryId) -> bool {
        self.row_for(entry).is_some_and(|r| r.status.is_live())
    }

    /// Marks the active run's failure banner as raised; the flag says whether this was the
    /// first time (only then is the banner raised). Another run has no flag: `false`.
    pub fn failure_noted(self, run: RunId) -> (Self, bool) {
        match self.active {
            Some(a) if a.run == run && !a.failure_noted => {
                let active = Some(ActiveRun {
                    failure_noted: true,
                    ..a
                });
                (Self { active, ..self }, true)
            }
            _ => (self, false),
        }
    }

    pub fn row_for(&self, entry: EntryId) -> Option<&RunRow> {
        self.rows.iter().find(|r| r.entry == entry)
    }

    pub fn totals(&self) -> Totals {
        self.rows
            .iter()
            .fold(Totals::default(), |t, row| match &row.status {
                PairStatus::Completed(s) => Totals {
                    only_in_source: t.only_in_source + s.only_in_source,
                    only_in_target: t.only_in_target + s.only_in_target,
                    different: t.different + s.different,
                    identical: t.identical + s.equal,
                    completed: t.completed + 1,
                    ..t
                },
                PairStatus::Failed(_) => Totals {
                    failed: t.failed + 1,
                    ..t
                },
                PairStatus::Cancelled(_) => Totals {
                    cancelled: t.cancelled + 1,
                    ..t
                },
                _ => t,
            })
    }

    /// Some pair has finished and still holds results (or a failure) that Discard clears.
    pub fn any_settled(&self) -> bool {
        self.rows.iter().any(|r| r.status.is_settled())
    }

    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }

    pub fn can_cancel(&self) -> bool {
        self.active.is_some_and(|a| !a.cancel_requested)
    }

    /// Rewrites the live row of `run` under `pair`, if there is one.
    fn with_live_row(
        self,
        run: RunId,
        pair: PairId,
        change: impl FnOnce(RunRow) -> RunRow,
    ) -> Self {
        let mut change = Some(change);
        let rows = self
            .rows
            .into_iter()
            .map(|row| {
                let hit = row.run == run && row.pair == pair && row.status.is_live();
                match change.take_if(|_| hit) {
                    Some(change) => change(row),
                    None => row,
                }
            })
            .collect();
        Self { rows, ..self }
    }
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
