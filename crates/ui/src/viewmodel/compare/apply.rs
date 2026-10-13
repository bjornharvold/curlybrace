//! The apply workflow as the view-model knows it: idle, reviewing the snapshot, running, and
//! finished with a report per pair. Pure; every step returns a new state and an event of
//! another run or pair changes nothing.
use curlybrace_core::compare::apply::ApplyReport;
use curlybrace_core::compare::protocol::{PairId, RunId};

use super::pairs::EntryId;
use super::review::Review;

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyRow {
    pub pair: PairId,
    pub entry: EntryId,
    pub done: u64,
    pub total: u64,
    pub report: Option<ApplyReport>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ApplyUi {
    #[default]
    Idle,
    Reviewing(Review),
    Running {
        run: RunId,
        total: u64,
        rows: Vec<ApplyRow>,
        cancel_requested: bool,
    },
    Finished {
        /// One report per reviewed pair; a pair that never reported counts all its writes as
        /// not run.
        reports: Vec<(PairId, ApplyReport)>,
        cancelled: bool,
        /// The writes the confirmed review asked for.
        requested: u64,
    },
}

/// What happened to the requested writes, over all pairs. The five counts add up to the
/// requested total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplySummary {
    pub applied: u64,
    pub stale: u64,
    pub not_planned: u64,
    pub failed: u64,
    pub not_run: u64,
}

impl ApplySummary {
    pub fn total(&self) -> u64 {
        self.applied + self.stale + self.not_planned + self.failed + self.not_run
    }
}

impl ApplyUi {
    pub fn reviewing(review: Review) -> Self {
        Self::Reviewing(review)
    }

    /// A workflow is in progress that blocks other work: the dialog is open or writes run.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Reviewing(_) | Self::Running { .. })
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub fn review(&self) -> Option<&Review> {
        match self {
            Self::Reviewing(review) => Some(review),
            _ => None,
        }
    }

    /// The typed confirmation changed; ignored unless reviewing.
    pub fn confirm_typed(self, text: &str) -> Self {
        match self {
            Self::Reviewing(review) => Self::Reviewing(review.with_confirm_text(text)),
            other => other,
        }
    }

    /// Confirm was accepted and `run` sent: the reviewed pairs become progress rows.
    pub fn started(self, run: RunId) -> Self {
        let Self::Reviewing(review) = self else {
            return self;
        };
        let rows = review
            .pairs
            .iter()
            .map(|p| ApplyRow {
                pair: p.pair,
                entry: p.entry,
                done: 0,
                total: p.counts.total(),
                report: None,
            })
            .collect();
        Self::Running {
            run,
            total: review.op_total(),
            rows,
            cancel_requested: false,
        }
    }

    /// Cancel was sent for the running apply; a second call changes nothing.
    pub fn cancelling(self) -> Self {
        match self {
            Self::Running {
                run, total, rows, ..
            } => Self::Running {
                run,
                total,
                rows,
                cancel_requested: true,
            },
            other => other,
        }
    }

    pub fn can_cancel(&self) -> bool {
        matches!(
            self,
            Self::Running {
                cancel_requested: false,
                ..
            }
        )
    }

    /// Progress of a pair of the running apply; another run or an unknown pair is ignored.
    pub fn progressed(self, run: RunId, pair: PairId, done: u64, total: u64) -> Self {
        self.with_row(run, pair, |row| ApplyRow { done, total, ..row })
    }

    /// The pair's final report; ignored unless it belongs to the running apply.
    pub fn pair_done(self, run: RunId, pair: PairId, report: ApplyReport) -> Self {
        self.with_row(run, pair, |row| ApplyRow {
            done: row.total,
            report: Some(report),
            ..row
        })
    }

    /// The apply's terminal; ignored unless it is the running apply's.
    pub fn apply_done(self, run: RunId) -> Self {
        match self {
            Self::Running {
                run: current,
                total,
                rows,
                cancel_requested,
            } if current == run => {
                let reports = rows
                    .into_iter()
                    .map(|row| {
                        let never_ran = ApplyReport {
                            not_run: row.total,
                            ..ApplyReport::default()
                        };
                        (row.pair, row.report.unwrap_or(never_ran))
                    })
                    .collect();
                Self::Finished {
                    reports,
                    cancelled: cancel_requested,
                    requested: total,
                }
            }
            other => other,
        }
    }

    /// The totals over all pairs once finished. Writes no report accounts for (a count that
    /// falls short of `requested`) are added to `not_run`, so the counts add up to `requested`.
    pub fn summary(&self) -> Option<ApplySummary> {
        let Self::Finished {
            reports, requested, ..
        } = self
        else {
            return None;
        };
        let sum = reports
            .iter()
            .fold(ApplySummary::default(), |s, (_, r)| ApplySummary {
                applied: s.applied + r.applied,
                stale: s.stale + r.stale,
                not_planned: s.not_planned + r.not_planned,
                failed: s.failed + r.failed,
                not_run: s.not_run + r.not_run,
            });
        Some(ApplySummary {
            not_run: sum.not_run + requested.saturating_sub(sum.total()),
            ..sum
        })
    }

    /// The pairs whose results are now stale: an apply that wrote anything or failed.
    pub fn touched_pairs(&self) -> Vec<PairId> {
        match self {
            Self::Finished { reports, .. } => reports
                .iter()
                .filter(|(_, r)| r.applied > 0 || r.failed > 0)
                .map(|(pair, _)| *pair)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Dialog closed or report dismissed. A running apply stays.
    pub fn dismissed(self) -> Self {
        match self {
            running @ Self::Running { .. } => running,
            _ => Self::Idle,
        }
    }

    fn with_row(self, run: RunId, pair: PairId, change: impl FnOnce(ApplyRow) -> ApplyRow) -> Self {
        let Self::Running {
            run: current,
            total,
            rows,
            cancel_requested,
        } = self
        else {
            return self;
        };
        let mut change = Some(change);
        let hit = current == run;
        let rows = rows
            .into_iter()
            .map(|row| match change.take_if(|_| hit && row.pair == pair) {
                Some(change) => change(row),
                None => row,
            })
            .collect();
        Self::Running {
            run: current,
            total,
            rows,
            cancel_requested,
        }
    }
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
