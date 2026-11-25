//! The document export in the app: the modal's actions, the file dialog's answers, the job and
//! its events.
//!
//! `start` is the only place that builds `ExportCommand::Start`. It is reached from exactly one
//! action, `PathChosen`, honoured only while the file dialog is open (`Choosing`), and it
//! re-checks the connection against the state as it is now. The runtime's own refusals (an export
//! already running, a connection not open) would read as "not supported" in the banner, so the
//! gates here decide before anything is sent. Paths and filters are never logged.
use std::path::PathBuf;

use curlybrace_core::error::CoreError;
use curlybrace_core::export::protocol::{DocExportRequest, ExportCommand, ExportEvent};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Query};

use super::CurlybraceApp;
use super::file_picker::{self, PickPoll};
use super::reducer::{Input, Out};
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::doc_export::{
    DocExportDraft, DocExportSource, DocExportUi, doc_export_block,
};

/// The banner context of a failed export (`failures.export_documents`).
const FAILURE_CONTEXT: &str = "export_documents";

#[derive(Clone, PartialEq)]
pub enum DocExportAction {
    /// Opens the modal for `ns`: the whole collection, or the documents of the applied query.
    Open {
        ns: Namespace,
        whole_collection: bool,
    },
    Edit(DocExportDraft),
    /// The file dialog opens (the app layer reacts to the new state).
    ChooseFile,
    PathChosen(PathBuf),
    PathCancelled,
    Cancel,
    Close,
}

/// Prints the variant only: the namespace and the chosen path are never shown.
impl std::fmt::Debug for DocExportAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open {
                whole_collection, ..
            } => f
                .debug_struct("Open")
                .field("whole_collection", whole_collection)
                .finish_non_exhaustive(),
            Self::Edit(draft) => f.debug_tuple("Edit").field(draft).finish(),
            Self::ChooseFile => f.write_str("ChooseFile"),
            Self::PathChosen(_) => f.write_str("PathChosen(..)"),
            Self::PathCancelled => f.write_str("PathCancelled"),
            Self::Cancel => f.write_str("Cancel"),
            Self::Close => f.write_str("Close"),
        }
    }
}

fn none(state: AppState) -> Out {
    (state, Vec::new())
}

fn with_export(state: AppState, doc_export: DocExportUi) -> AppState {
    AppState {
        doc_export,
        ..state
    }
}

pub(super) fn on_action(state: AppState, action: DocExportAction) -> Out {
    match action {
        DocExportAction::Open {
            ns,
            whole_collection,
        } => open(state, ns, whole_collection),
        DocExportAction::Edit(draft) => {
            let next = state.doc_export.clone().edited(|_| draft);
            none(with_export(state, next))
        }
        DocExportAction::ChooseFile => choose_file(state),
        DocExportAction::PathChosen(path) => path_chosen(state, path),
        DocExportAction::PathCancelled => back_to_modal(state),
        DocExportAction::Cancel => cancel(state),
        DocExportAction::Close => {
            let next = state.doc_export.clone().closed();
            none(with_export(state, next))
        }
    }
}

fn connected(state: &AppState) -> bool {
    state.status == ConnStatus::Connected
}

/// Opens the modal when the gates allow. A refused open changes nothing (the entry's reason is
/// `doc_export_block`'s, shown by the view).
fn open(state: AppState, ns: Namespace, whole_collection: bool) -> Out {
    // A whole-collection export reads no query, so a collation on the shown page is no reason.
    let gated = match whole_collection {
        true => Query::default(),
        false => state.page_query.clone(),
    };
    if doc_export_block(connected(&state), &gated, state.doc_export.is_busy()).is_some() {
        return none(state);
    }
    let source = DocExportSource {
        session: state.session_id(),
        ns,
        filter: (!whole_collection).then(|| state.page_query.filter.clone()),
        ignores_options: !whole_collection
            && [&state.page_query.sort, &state.page_query.projection]
                .into_iter()
                .any(|d| d.as_ref().is_some_and(|d| !d.is_empty())),
    };
    let next = state.doc_export.clone().open(source);
    none(with_export(state, next))
}

fn choose_file(state: AppState) -> Out {
    let shown = match &state.doc_export {
        DocExportUi::Configuring { source, .. } => source.session == state.session_id(),
        _ => false,
    };
    match connected(&state) && shown {
        true => {
            let next = state.doc_export.clone().choosing();
            none(with_export(state, next))
        }
        false => none(state),
    }
}

/// The modal again, with the options kept; from the open dialog or from a running job.
fn reconfiguring(ui: DocExportUi) -> DocExportUi {
    match ui {
        DocExportUi::Choosing { source, draft } | DocExportUi::Running { source, draft, .. } => {
            DocExportUi::Configuring { source, draft }
        }
        other => other,
    }
}

/// The dialog was cancelled: back to the modal. Ignored when no dialog is open.
fn back_to_modal(state: AppState) -> Out {
    match state.doc_export {
        DocExportUi::Choosing { .. } => {
            let next = reconfiguring(state.doc_export.clone());
            none(with_export(state, next))
        }
        _ => none(state),
    }
}

fn path_chosen(state: AppState, path: PathBuf) -> Out {
    match &state.doc_export {
        DocExportUi::Choosing { source, draft } => {
            let (source, draft) = (source.clone(), *draft);
            start(state, source, draft, path)
        }
        _ => none(state),
    }
}

/// The only constructor of `ExportCommand::Start`. The connection is checked as it is now (the
/// state may have moved since the modal opened); the filter was fixed when the modal opened, so
/// only the connection and that the session is still the one shown are re-checked. A start that
/// no longer holds sends nothing and falls back to the modal.
fn start(state: AppState, source: DocExportSource, draft: DocExportDraft, path: PathBuf) -> Out {
    let same_session = state.session_id() == source.session;
    let blocked = doc_export_block(connected(&state) && same_session, &Query::default(), false);
    if blocked.is_some() {
        let next = reconfiguring(state.doc_export.clone());
        return none(with_export(state, next));
    }
    let run = state.doc_export_runs + 1;
    let request = DocExportRequest {
        session: source.session,
        ns: source.ns.clone(),
        filter: source.filter.clone().unwrap_or_default(),
        format: draft.format,
        mode: draft.mode,
        path: path.clone(),
    };
    let next = state.doc_export.clone().started(run, path, None);
    let state = AppState {
        doc_export: next,
        doc_export_runs: run,
        ..state
    };
    (
        state,
        vec![Command::Export(ExportCommand::Start { run, request })],
    )
}

/// Sends Cancel once for the running export.
fn cancel(state: AppState) -> Out {
    let run = match &state.doc_export {
        DocExportUi::Running {
            run,
            cancel_requested: false,
            ..
        } => *run,
        _ => return none(state),
    };
    let next = state.doc_export.clone().cancelling();
    (
        with_export(state, next),
        vec![Command::Export(ExportCommand::Cancel { run })],
    )
}

/// Whether `ev` belongs to the document export.
pub(super) fn is_export_event(ev: &Event) -> bool {
    matches!(ev, Event::Export(_))
}

/// Events of the export run; the state ignores every other run, and what it ignores changes
/// nothing.
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Event::Export(ev) = ev else {
        return none(state);
    };
    match ev {
        ExportEvent::Progress { run, done, total } => {
            let next = state.doc_export.clone().progressed(run, done, total);
            none(with_export(state, next))
        }
        ExportEvent::Done { run, outcome } => {
            let next = state.doc_export.clone().finished(run, Ok(outcome));
            none(with_export(state, next))
        }
        ExportEvent::Failed { run, error } => failed(state, run, error),
    }
}

/// The export ended without a `Done` (refused, or the file could not be written): the banner
/// shows it and the modal is open again with the options kept. A failure of another run is stale.
fn failed(state: AppState, run: u64, error: CoreError) -> Out {
    match &state.doc_export {
        DocExportUi::Running { run: current, .. } if *current == run => {
            let next = reconfiguring(state.doc_export.clone());
            let state = AppState {
                banner: Some(Banner::Failure {
                    context: FAILURE_CONTEXT,
                    error,
                }),
                banner_session: state.active,
                ..with_export(state, next)
            };
            none(state)
        }
        _ => none(state),
    }
}

impl CurlybraceApp {
    /// Opens the save dialog of the document export when one waits in `Choosing` and none is open.
    pub(super) fn ensure_doc_picker(&mut self) {
        let DocExportUi::Choosing { source, draft } = &self.state.doc_export else {
            return;
        };
        // A dialog left over from an export the person went back from is replaced: dropping it
        // drops its receiver, so its late answer is never read.
        if self
            .doc_pick
            .as_ref()
            .is_some_and(|pick| pick.is_for(source, draft))
        {
            return;
        }
        self.doc_pick = Some(file_picker::start_doc_pick(
            self.picker.clone(),
            source.clone(),
            *draft,
            self.repaint.clone(),
        ));
    }

    /// Turns the dialog's answer into the matching action, but only when the export it was opened
    /// for is still the one waiting; a stale answer is dropped.
    pub(super) fn poll_doc_picker(&mut self) {
        let Some(PickPoll::Done(path)) = self.doc_pick.as_ref().map(|p| p.poll()) else {
            return;
        };
        let asked = self
            .doc_pick
            .take()
            .map(file_picker::PendingDocPick::into_parts);
        let still_waiting = matches!(
            (&self.state.doc_export, &asked),
            (DocExportUi::Choosing { source, draft }, Some((s, d))) if source == s && draft == d
        );
        if !still_waiting {
            return;
        }
        let action = match path {
            Some(path) => DocExportAction::PathChosen(path),
            None => DocExportAction::PathCancelled,
        };
        self.dispatch(Input::DocExport(action));
    }
}
