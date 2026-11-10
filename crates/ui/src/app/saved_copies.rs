//! The list of saved undo copies: open it, read the listing from the runtime, delete a copy after
//! asking. Reads and deletes go through the runtime's `SnapshotList` / `SnapshotDelete`, which
//! need no session, so the list works from the connection manager as well.
use super::bulk_actions::numbered;
use super::reducer::{Out, failure};
use super::state::AppState;
use curlybrace_core::bulk::BulkRunId;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use curlybrace_core::snapshot::SnapshotInfo;

/// What the person did in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopiesAction {
    Open,
    Close,
    /// "Delete" on a row: asks before anything is removed.
    Ask(BulkRunId),
    /// "Keep it" on the question.
    Keep,
    /// "Delete the copy" on the question.
    Delete(BulkRunId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedCopies {
    /// The listing; `None` until the first answer.
    pub list: Option<Vec<SnapshotInfo>>,
    /// The listing or deletion not answered yet: its request id, and the run when it is a
    /// deletion (so a copy still listed afterwards can be told).
    pub awaiting: Option<(u64, Option<BulkRunId>)>,
    /// The copy the person is being asked about.
    pub asking: Option<BulkRunId>,
}

impl SavedCopies {
    pub fn busy(&self) -> bool {
        self.awaiting.is_some()
    }
}

pub(super) fn on_action(state: AppState, action: CopiesAction) -> Out {
    match action {
        CopiesAction::Open => open(state),
        CopiesAction::Close => (
            AppState {
                copies: None,
                ..state
            },
            Vec::new(),
        ),
        CopiesAction::Ask(run) => asked(state, Some(run)),
        CopiesAction::Keep => asked(state, None),
        CopiesAction::Delete(run) => delete(state, run),
    }
}

/// Opens the list and reads it. Without a folder for the copies (undo is off) there is nothing
/// to list.
fn open(state: AppState) -> Out {
    if state.snapshot_dir.is_none() || state.copies.is_some() {
        return (state, Vec::new());
    }
    let (request, state) = numbered(state);
    let copies = SavedCopies {
        list: None,
        awaiting: Some((request, None)),
        asking: None,
    };
    (
        AppState {
            copies: Some(copies),
            ..state
        },
        vec![Command::SnapshotList { request }],
    )
}

fn asked(state: AppState, run: Option<BulkRunId>) -> Out {
    let copies = state
        .copies
        .clone()
        .filter(|c| !c.busy())
        .map(|c| SavedCopies { asking: run, ..c });
    match copies {
        Some(copies) => (
            AppState {
                copies: Some(copies),
                ..state
            },
            Vec::new(),
        ),
        None => (state, Vec::new()),
    }
}

/// Deletes the copy that was asked about, and only that one.
fn delete(state: AppState, run: BulkRunId) -> Out {
    let allowed = state
        .copies
        .as_ref()
        .is_some_and(|c| !c.busy() && c.asking == Some(run));
    if !allowed {
        return (state, Vec::new());
    }
    let (request, state) = numbered(state);
    let copies = state.copies.clone().map(|c| SavedCopies {
        awaiting: Some((request, Some(run))),
        asking: None,
        ..c
    });
    (
        AppState { copies, ..state },
        vec![Command::SnapshotDelete { run, request }],
    )
}

/// The listing the runtime answered. A copy that was deleted and is still listed is still on
/// disk, which the person must be told; the list shows what is really there either way.
pub(super) fn on_listing(state: AppState, request: u64, snapshots: &[SnapshotInfo]) -> AppState {
    let Some((awaited, deleted)) = state.copies.as_ref().and_then(|c| c.awaiting) else {
        return state;
    };
    if awaited != request {
        return state;
    }
    let still_there = deleted.is_some_and(|run| snapshots.iter().any(|s| s.run == run));
    let copies = state.copies.clone().map(|c| SavedCopies {
        list: Some(snapshots.to_vec()),
        awaiting: None,
        ..c
    });
    let state = AppState { copies, ..state };
    match still_there {
        true => failure(state, "snapshot_delete", CoreError::SnapshotDeleteFailed),
        false => state,
    }
}

#[cfg(test)]
#[path = "saved_copies_tests.rs"]
mod tests;
