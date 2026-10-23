//! Production write guard: a mutating command aimed at a production connection is parked here
//! until the user confirms it, then sent exactly once.
use super::reducer::Out;
use super::state::{AppState, Banner, ConnStatus, Pending};
use curlybrace_core::runtime::Command;

/// Which kind of write is parked (the warning words each one differently).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Insert,
    Replace,
    Clone,
    Delete,
    CreateIndex,
    DropIndex,
    /// Hiding or unhiding an index.
    HideIndex,
    SetIndexTtl,
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
    /// The connect epoch of the session when the write was parked: a reconnect bumps it, so a
    /// write never outlives the opening it was aimed at.
    pub epoch: u64,
}

/// The user's answer to the production write warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteWarningAction {
    Confirm,
    Cancel,
}

impl AppState {
    /// Drops a parked write that belongs to the active session.
    pub fn write_warning_dropped_for_active(self) -> AppState {
        let session = self.session_id();
        self.write_warning_dropped_for(session)
    }

    /// Drops a parked write that belongs to `session` (closed, lost or replaced).
    pub fn write_warning_dropped_for(self, session: uuid::Uuid) -> AppState {
        let belongs = self
            .write_warning
            .as_ref()
            .is_some_and(|w| w.command.session() == Some(session));
        match belongs {
            true => AppState {
                write_warning: None,
                ..self
            },
            false => self,
        }
    }

    /// Whether the active session counts as production: see [`AppState::session_production`].
    /// The embedded session has no saved connection and is never production.
    pub fn active_is_production(&self) -> bool {
        self.active.is_some_and(|id| self.session_production(id))
    }

    /// Whether the open session `id` counts as production: the flag it opened with (snapshotted
    /// at connect, wherever its view lives) OR the saved connection's live flag. Ticking the flag
    /// therefore applies at once, unticking only on the next connect; a connection deleted while
    /// open is judged by its snapshot alone. Every reader of "is this session production" goes
    /// through here.
    pub fn session_production(&self, id: uuid::Uuid) -> bool {
        let snapshot = match self.active == Some(id) {
            true => self.production,
            false => self.parked.get(&id).is_some_and(|v| v.production),
        };
        snapshot || self.connection(id).is_some_and(|c| c.production)
    }

    /// Whether the active session is read-only: see [`AppState::session_read_only`]. The embedded
    /// session has no saved connection and is never read-only.
    pub fn active_is_read_only(&self) -> bool {
        self.active.is_some_and(|id| self.session_read_only(id))
    }

    /// Whether the open session `id` is read-only: the flag it opened with (snapshotted at
    /// connect, wherever its view lives) OR the saved connection's live flag. Ticking locks at
    /// once (the lock is also sent to the core), unticking only on the next connect; a connection
    /// deleted while open is judged by its snapshot alone.
    pub fn session_read_only(&self, id: uuid::Uuid) -> bool {
        let snapshot = match self.active == Some(id) {
            true => self.read_only,
            false => self.parked.get(&id).is_some_and(|v| v.read_only),
        };
        snapshot || self.connection(id).is_some_and(|c| c.read_only)
    }
}

/// Answers the warning: `Confirm` sends the parked write if it is still valid (else says it was
/// not sent), `Cancel` drops it.
pub(super) fn on_action(state: AppState, action: WriteWarningAction) -> Out {
    let AppState {
        write_warning: parked,
        ..
    } = state;
    let state = AppState {
        write_warning: None,
        ..state
    };
    match (action, parked) {
        // The session was locked while the warning was open: the write is dropped, never sent.
        (WriteWarningAction::Confirm, Some(parked))
            if parked
                .command
                .session()
                .is_some_and(|id| state.session_read_only(id)) =>
        {
            let banner = Some(Banner::ReadOnly);
            (AppState { banner, ..state }, Vec::new())
        }
        (WriteWarningAction::Confirm, Some(parked)) if still_valid(&state, &parked) => {
            let ParkedWrite {
                command, pending, ..
            } = parked;
            let next = AppState {
                pending: Some(pending),
                ..state
            };
            (
                super::index_write_actions::sent(next, &command),
                vec![command],
            )
        }
        (WriteWarningAction::Confirm, Some(parked)) => {
            let banner = Some(match connection_unchanged(&state, &parked) {
                true => Banner::WriteNotSentBusy,
                false => Banner::WriteNotSentConnectionChanged,
            });
            (AppState { banner, ..state }, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// The parked write still targets the same, connected opening of the active session.
fn connection_unchanged(state: &AppState, parked: &ParkedWrite) -> bool {
    let session = state.session_id();
    parked.command.session() == Some(session)
        && state.is_open(session)
        && state.status == ConnStatus::Connected
        && state.latest_requested_epoch == parked.epoch
}

/// The parked write still targets the live active session and nothing else is being written.
fn still_valid(state: &AppState, parked: &ParkedWrite) -> bool {
    connection_unchanged(state, parked) && state.pending.is_none()
}

#[cfg(test)]
#[path = "write_guard_tests.rs"]
mod write_guard_tests;
