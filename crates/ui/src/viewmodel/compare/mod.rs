//! View-models of the Compare workspace. Pure and egui-free.
pub mod detail;
pub mod preflight;
pub mod results;
pub mod setup;

use curlybrace_core::compare::protocol::{RunId, SessionId};
use curlybrace_core::error::CoreError;
use uuid::Uuid;

pub use detail::{ChangeLine, DetailView};
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
    /// A `Check` has been sent and not answered. At most one is in flight: answers carry no
    /// request identity, so a second one could not be told from a stale one.
    pub check_in_flight: bool,
    /// The form was edited while the check was in flight; its answer will be dropped.
    pub check_stale: bool,
    pub run: RunView,
    pub results: Option<ResultsView>,
    pub detail: Option<DetailView>,
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
            check_in_flight: false,
            check_stale: false,
            run: RunView::default(),
            results: None,
            detail: None,
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
    pub fn is_ready(&self) -> bool {
        matches!(self.session, Some(SessionPhase::Ready(_)))
    }

    pub fn is_running(&self) -> bool {
        self.run.running().is_some()
    }

    /// Whether the Check button may fire: a usable form and no check or run in flight.
    pub fn can_check(&self) -> bool {
        self.is_ready() && !self.check_in_flight && !self.is_running() && self.setup.ready()
    }

    /// Whether Start may fire: the same as `can_check`, and the latest answer is fresh and Ok.
    pub fn can_start(&self) -> bool {
        self.can_check() && !self.check_stale && self.preflight.allows_start()
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;
