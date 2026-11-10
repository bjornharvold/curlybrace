//! The administration dialogs (drop, rename, duplicate, create view): opening one from the
//! sidebar, editing it, and sending its write through the same read-only and production guards
//! as every other write. The dialog stays open while the write is in flight and shows what the
//! core refused, so a bad name can be fixed and sent again.

use super::actions::gated;
use super::reducer::{Out, failure};
use super::state::{AppState, Banner, Pending};
use super::write_guard::{ParkedWrite, WriteKind};
use crate::viewmodel::{
    AdminAction, AdminDialog, AdminKind, AdminRequest, FindBarState, IndexesState,
};
use curlybrace_core::error::CoreError;

/// The write warning's wording for what the dialog does.
fn write_kind(kind: AdminKind) -> WriteKind {
    match kind {
        AdminKind::DropCollection => WriteKind::DropCollection,
        AdminKind::DropDatabase => WriteKind::DropDatabase,
        AdminKind::Rename => WriteKind::RenameCollection,
        AdminKind::Duplicate => WriteKind::DuplicateCollection,
        AdminKind::CreateView => WriteKind::CreateView,
    }
}

/// A write of the dialog is in flight or waits for the production warning: the dialog holds still.
pub fn busy(state: &AppState) -> bool {
    state.pending.is_some() || state.write_warning.is_some()
}

fn unchanged(state: AppState) -> Out {
    (state, Vec::new())
}

/// A sidebar menu asked for a dialog. A read-only connection never gets one; one dialog at a
/// time.
pub(super) fn requested(state: AppState, request: AdminRequest) -> Out {
    gated(state, |s| {
        if s.active_is_read_only() {
            let banner = Some(Banner::ReadOnly);
            return (AppState { banner, ..s }, Vec::new());
        }
        if s.admin.is_some() {
            return unchanged(s);
        }
        let dialog = AdminDialog::open(request).with_session(s.session_id());
        (
            AppState {
                admin: Some(Box::new(dialog)),
                ..s
            },
            Vec::new(),
        )
    })
}

pub(super) fn on_action(state: AppState, action: AdminAction) -> Out {
    let Some(dialog) = state.admin.clone().map(|d| *d) else {
        return unchanged(state);
    };
    match action {
        AdminAction::Cancelled if !busy(&state) => (
            AppState {
                admin: None,
                ..state
            },
            Vec::new(),
        ),
        AdminAction::Confirmed => confirmed(state, dialog),
        _ if busy(&state) => unchanged(state),
        AdminAction::Typed(text) => edited(state, dialog.with_typed(text)),
        AdminAction::NameEdited(text) => edited(state, dialog.with_name(text)),
        AdminAction::SourceEdited(text) => edited(state, dialog.with_source(text)),
        AdminAction::PipelineEdited(text) => edited(state, dialog.with_pipeline(text)),
        AdminAction::DropTargetToggled(on) => edited(state, dialog.with_drop_target(on)),
        AdminAction::Cancelled => unchanged(state),
    }
}

fn edited(state: AppState, dialog: AdminDialog) -> Out {
    (
        AppState {
            admin: Some(Box::new(dialog)),
            ..state
        },
        Vec::new(),
    )
}

/// Sends the dialog's write once it is ready: refused on a read-only connection, parked for the
/// production warning, otherwise sent at once with the in-flight marker set.
fn confirmed(state: AppState, dialog: AdminDialog) -> Out {
    gated(state, |s| {
        if s.active_is_read_only() {
            let banner = Some(Banner::ReadOnly);
            return (AppState { banner, ..s }, Vec::new());
        }
        let session = s.session_id();
        let Some(command) = dialog
            .command(session)
            .filter(|_| dialog.session == session && !busy(&s))
        else {
            return unchanged(s);
        };
        let pending = Pending {
            ns: dialog.target_ns(),
            editor: None,
        };
        if s.active_is_production() {
            let write_warning = Some(ParkedWrite {
                command,
                pending,
                kind: write_kind(dialog.kind),
                epoch: s.latest_requested_epoch,
            });
            return (AppState { write_warning, ..s }, Vec::new());
        }
        let next = AppState {
            pending: Some(pending),
            admin: Some(Box::new(dialog.with_error(None))),
            ..s
        };
        (next, vec![command])
    })
}

/// An administration write succeeded: its dialog closes, and what the write removed or renamed
/// is no longer shown (a later Find or Refresh on it would only answer `NotFound`).
pub(super) fn answered(state: AppState, db: &str) -> AppState {
    let ours = state.pending.as_ref().is_some_and(|p| p.ns.db == db) && state.admin.is_some();
    if !ours {
        return state;
    }
    let dialog = state.admin.clone().map(|d| *d);
    let state = AppState {
        admin: None,
        pending: None,
        ..state
    };
    match dialog {
        Some(d) if hides_selection(&d, &state) => deselected(state),
        _ => state,
    }
}

/// Whether the finished write took away the collection (or database) that is open.
fn hides_selection(dialog: &AdminDialog, state: &AppState) -> bool {
    let Some(open) = state.selected_ns() else {
        return false;
    };
    match dialog.kind {
        AdminKind::DropDatabase => open.db == dialog.db,
        AdminKind::DropCollection | AdminKind::Rename => {
            open.db == dialog.db && Some(open.collection.as_str()) == dialog.collection.as_deref()
        }
        AdminKind::Duplicate | AdminKind::CreateView => false,
    }
}

/// Nothing is open: the selection, the page and the editor are cleared, and the Indexes tab starts
/// over.
fn deselected(state: AppState) -> AppState {
    let mut sidebar = state.sidebar;
    sidebar.selected = None;
    AppState {
        sidebar,
        find: FindBarState::default(),
        loading_page: false,
        page: None,
        page_key: None,
        verdict: Default::default(),
        tree: state.tree.results_replaced(),
        editor: None,
        indexes: IndexesState::default(),
        ..state
    }
}

/// An administration write failed. The dialog stays open with the refusal beside its fields; with
/// no dialog (it was closed meanwhile) the failure is a banner like any other.
pub(super) fn failed(state: AppState, context: &'static str, error: CoreError) -> AppState {
    let state = AppState {
        pending: None,
        ..state
    };
    match state.admin.clone().map(|d| *d) {
        Some(dialog) => AppState {
            admin: Some(Box::new(dialog.with_error(Some(error)))),
            ..state
        },
        None => failure(state, context, error),
    }
}
