//! The Shell tab's index-miss guard in the view-model: a Run held back while its queries are
//! planned (`ShellStatus::Checking`), and the `IndexMiss` notice when the plan would scan. Pure:
//! every updater returns a new value. The app decides when to check and what to send.
use super::shell::{PendingCheck, ShellNotice, ShellState, ShellStatus};
use curlybrace_core::guard::GuardReport;
use std::ops::Range;

impl ShellState {
    /// Waits for guard request `request` before running `ran` (`None`: the whole script);
    /// `unchecked` counts the calls the check was not asked about. The previous notice goes, as
    /// when a run starts; the output and the last value stay until the run really starts.
    pub fn checking(&self, request: u64, ran: Option<Range<usize>>, unchecked: usize) -> Self {
        Self {
            status: ShellStatus::Checking { request },
            pending_check: Some(PendingCheck { ran, unchecked }),
            notice: None,
            ..self.clone()
        }
    }

    /// Whether this shell waits for the answer to guard request `request`.
    pub fn awaits_check(&self, request: u64) -> bool {
        self.status == (ShellStatus::Checking { request })
    }

    /// Back to idle when a check was in flight (its late answer then matches nothing); any other
    /// status is kept.
    pub fn check_dropped(&self) -> Self {
        match self.status {
            ShellStatus::Checking { .. } => Self {
                status: ShellStatus::Idle,
                pending_check: None,
                ..self.clone()
            },
            ShellStatus::Idle | ShellStatus::Running { .. } => self.clone(),
        }
    }

    /// Cancel: a check in flight goes, and so does an index notice (the run it held back is
    /// not wanted). A real run is stopped by the app, not here.
    pub fn check_cancelled(&self) -> Self {
        let next = self.check_dropped();
        match next.notice {
            Some(ShellNotice::IndexMiss { .. }) => Self {
                notice: None,
                ..next
            },
            _ => next,
        }
    }

    /// The check let the run go on: back to idle, with what it was for (the range to run and
    /// the calls it was not asked about). The caller made sure it [`awaits_check`].
    ///
    /// [`awaits_check`]: ShellState::awaits_check
    pub fn check_settled(&self) -> (Self, PendingCheck) {
        let pending = self.pending_check.clone().unwrap_or_default();
        (self.check_dropped(), pending)
    }

    /// The check found queries that would scan: back to idle with the `IndexMiss` notice, whose
    /// `unchecked` adds the calls the check was not asked about to the ones it could not judge.
    pub fn index_missed(&self, report: GuardReport) -> Self {
        let (idle, pending) = self.check_settled();
        let unchecked = pending.unchecked.saturating_add(report.unchecked);
        Self {
            notice: Some(ShellNotice::IndexMiss {
                report,
                unchecked,
                ran: pending.ran,
            }),
            ..idle
        }
    }
}
