//! Production write guard: a mutating command aimed at a production connection is parked here
//! until the user confirms it, then sent exactly once.
use super::reducer::Out;
use super::state::{AppState, ConnStatus, Pending};
use curlybrace_core::runtime::Command;

/// Which kind of write is parked (the warning words each one differently).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Insert,
    Replace,
    Clone,
    Delete,
}

/// Whether a mutation must wait for the user's confirmation when it targets a production
/// connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    /// Park the write on a production connection.
    Warn,
    /// The user already confirmed this exact write (the delete dialog): send it.
    Confirmed,
}

/// A write held back until the user answers the warning. It is sent at most once.
#[derive(Clone, Debug)]
pub struct ParkedWrite {
    pub command: Command,
    /// What `pending` becomes when the write is sent, exactly as an unguarded write would set it.
    pub pending: Pending,
    pub kind: WriteKind,
}

/// The user's answer to the production write warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteWarningAction {
    Confirm,
    Cancel,
}

impl AppState {
    /// Whether the active session is a saved connection flagged as production. The embedded
    /// session has no saved connection and is never production.
    pub fn active_is_production(&self) -> bool {
        self.active
            .and_then(|id| self.connection(id))
            .is_some_and(|c| c.production)
    }
}

/// Answers the warning: `Confirm` sends the parked write if it is still valid, `Cancel` drops it.
pub(super) fn on_action(state: AppState, action: WriteWarningAction) -> Out {
    let AppState {
        write_warning: parked,
        ..
    } = &state;
    let parked = parked.clone();
    let state = AppState {
        write_warning: None,
        ..state
    };
    match (action, parked) {
        (WriteWarningAction::Confirm, Some(parked)) if still_valid(&state, &parked) => {
            let next = AppState {
                pending: Some(parked.pending),
                ..state
            };
            (next, vec![parked.command])
        }
        _ => (state, Vec::new()),
    }
}

/// The parked write still targets the live active session and nothing else is being written.
fn still_valid(state: &AppState, parked: &ParkedWrite) -> bool {
    let session = state.session_id();
    parked.command.session() == Some(session)
        && state.is_open(session)
        && state.status == ConnStatus::Connected
        && state.pending.is_none()
}

#[cfg(test)]
#[path = "write_guard_tests.rs"]
mod write_guard_tests;
