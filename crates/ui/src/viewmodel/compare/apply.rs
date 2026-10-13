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
        reports: Vec<(PairId, ApplyReport)>,
        cancelled: bool,
        /// Operations the reports account for: applied + stale + not planned + failed + not run.
        ops_sent: u64,
    },
}

fn accounted(report: &ApplyReport) -> u64 {
    report.applied + report.stale + report.not_planned + report.failed + report.not_run
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
                rows,
                cancel_requested,
                ..
            } if current == run => {
                let reports: Vec<(PairId, ApplyReport)> = rows
                    .into_iter()
                    .filter_map(|row| row.report.map(|r| (row.pair, r)))
                    .collect();
                let ops_sent = reports.iter().map(|(_, r)| accounted(r)).sum();
                Self::Finished {
                    reports,
                    cancelled: cancel_requested,
                    ops_sent,
                }
            }
            other => other,
        }
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
