//! View-models of the Compare workspace. Pure and egui-free.
pub mod detail;
pub mod duplicates;
pub mod preflight;
pub mod results;
pub mod setup;

use curlybrace_core::compare::protocol::{PairSpec, Preflight, RunId, SessionId};
use curlybrace_core::error::CoreError;
use uuid::Uuid;

pub use detail::{ChangeLine, DetailView, canonical_json};
pub use duplicates::{DuplicateRow, DuplicatesView};
pub use preflight::{PreflightMessage, PreflightView};
pub use results::{Counts, PAGE_SIZE, ResultsView, category_key, key_label};
pub use setup::{PAIR_ID, Pick, RulesForm, ScopeKind, SetupError, SetupForm, SideForm};

/// Where the compare session stands. It owns its own connections, so the browsing connection
/// coming and going never changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Opening(SessionId),
    Ready(SessionId),
}

impl SessionPhase {
    pub fn id(&self) -> SessionId {
        match self {
            Self::Opening(id) | Self::Ready(id) => *id,
        }
    }
}

/// The run of the single pair, as the progress panel shows it.
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

impl RunView {
    pub fn started(run: RunId) -> Self {
        Self::Running {
            run,
            finished_pairs: 0,
            scanned_source: 0,
            scanned_target: 0,
            differences: 0,
            cancelled: false,
            failure: None,
        }
    }

    /// The run in flight, if any.
    pub fn running(&self) -> Option<RunId> {
        match self {
            Self::Running { run, .. } => Some(*run),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompareState {
    pub session: Option<SessionPhase>,
    /// The connections the current session was opened for.
    pub connections: Option<(Uuid, Uuid)>,
    /// Ids are chosen here and never reused, so a late event can never match a newer session.
    pub next_session: SessionId,
    pub next_run: RunId,
    pub setup: SetupForm,
    pub preflight: PreflightView,
    /// The counts and index status behind `preflight`; present exactly while an accepted answer
    /// is current (cleared by any edit, a failed check or a closed session).
    pub last_check: Option<Preflight>,
    /// A `Check` has been sent and not answered. At most one is in flight: answers carry no
    /// request identity, so a second one could not be told from a stale one.
    pub check_in_flight: bool,
    /// The form was edited while the check was in flight; its answer will be dropped.
    pub check_stale: bool,
    pub run: RunView,
    /// Cancel was sent for the run in flight; cleared by its pair and run terminals.
    pub cancel_requested: bool,
    pub results: Option<ResultsView>,
    pub detail: Option<DetailView>,
    /// The duplicate-keys panel, offered after a run failed on a duplicate key.
    pub duplicates: DuplicatesView,
}

impl Default for CompareState {
    fn default() -> Self {
        Self {
            session: None,
            connections: None,
            next_session: 1,
            next_run: 1,
            setup: SetupForm::default(),
            preflight: PreflightView::default(),
            last_check: None,
            check_in_flight: false,
            check_stale: false,
            run: RunView::default(),
            cancel_requested: false,
            results: None,
            detail: None,
            duplicates: DuplicatesView::default(),
        }
    }
}

impl CompareState {
    /// Forgets everything tied to the current session; the id counters survive.
    pub fn session_dropped(self) -> Self {
        Self {
            next_session: self.next_session,
            next_run: self.next_run,
            ..Self::default()
        }
    }
}

impl CompareState {
    /// Whether Connect may fire for the picked pair: both picked, different from what is open,
    /// and no open in progress.
    pub fn can_open(&self, draft: (Option<Uuid>, Option<Uuid>)) -> bool {
        let opening = matches!(self.session, Some(SessionPhase::Opening(_)));
        match draft {
            (Some(source), Some(target)) => !opening && self.connections != Some((source, target)),
            _ => false,
        }
    }

    pub fn can_close(&self) -> bool {
        self.session.is_some()
    }

    /// Finished results can be thrown away (never while a run is in flight).
    pub fn can_discard(&self) -> bool {
        self.is_ready() && matches!(self.run, RunView::Finished { .. })
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.session, Some(SessionPhase::Ready(_)))
    }

    /// The finished run's pair failed on a duplicate key (the only failure the panel explains).
    pub fn run_failed_on_duplicate_key(&self) -> bool {
        matches!(
            self.run,
            RunView::Finished {
                failure: Some(CoreError::CompareDuplicateKey),
                ..
            }
        )
    }

    /// Whether Find duplicate keys may fire.
    pub fn can_find_duplicates(&self) -> bool {
        self.is_ready() && self.duplicates.can_find(self.run_failed_on_duplicate_key())
    }

    pub fn is_running(&self) -> bool {
        self.run.running().is_some()
    }

    /// Everything wrong with the setup, in display order: the form's own issues, then whether
    /// both sides are the very same collection (which needs the session's connections).
    pub fn issues(&self) -> Vec<SetupError> {
        let mut issues = self.setup.issues();
        let one_connection = self.connections.is_some_and(|(s, t)| s == t);
        if issues.is_empty() && one_connection && self.setup.same_namespace_and_filter() {
            issues.push(SetupError::SameCollection);
        }
        issues
    }

    /// The pair to compare, or the first issue.
    pub fn pair(&self) -> Result<PairSpec, SetupError> {
        match self.issues().into_iter().next() {
            Some(first) => Err(first),
            None => self.setup.pair(),
        }
    }

    /// Whether the Check button may fire: a usable form and no check or run in flight.
    pub fn can_check(&self) -> bool {
        self.can_check_given(self.issues().is_empty())
    }

    /// `can_check` for a setup whose readiness the caller already knows (computed once a frame).
    pub fn can_check_given(&self, setup_ready: bool) -> bool {
        self.is_ready() && !self.check_in_flight && !self.is_running() && setup_ready
    }

    /// Whether Start may fire: the same as `can_check`, and the latest answer is fresh and Ok.
    pub fn can_start(&self) -> bool {
        self.can_start_given(self.issues().is_empty())
    }

    /// `can_start` for a setup whose readiness the caller already knows.
    pub fn can_start_given(&self, setup_ready: bool) -> bool {
        self.can_check_given(setup_ready) && !self.check_stale && self.preflight.allows_start()
    }

    /// Whether Cancel may fire: a run is in flight and no cancel was sent for it yet.
    pub fn can_cancel(&self) -> bool {
        self.is_running() && !self.cancel_requested
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;
