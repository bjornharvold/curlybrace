//! The paste dialog's reducer: opening it on a pending paste, the counts, the choices, Confirm,
//! Cancel and the events of a copy run.
//!
//! The runtime has ONE run slot, so a preview and a copy never overlap: an edit that needs fresh
//! counts, and a Confirm pressed while counting, first cancel the preview and then wait for its
//! terminal event before the next run is sent. `start_copy` is the only place that builds
//! `CopyCommand::Start`, and it is reached only from `confirm` (when the dialog allows) or from
//! the end of the preview that `confirm` cancelled. Nothing is written before Confirm.
use super::copy_state::{CopyDialog, CopyPhase, CopyRun, RunOutcome};
use super::reducer::Out;
use super::state::AppState;
use curlybrace_core::compare::protocol::RunId;
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::protocol::{CopyCommand, CopyEvent};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;

/// What the person did in the paste dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum CopyInput {
    /// Show the dialog for the pending paste; `word` is the localized confirmation word.
    Open {
        word: String,
    },
    SetPolicy(ConflictPolicy),
    SetName {
        index: usize,
        name: String,
    },
    ToggleIndexesAndOptions,
    TypeConfirmation(String),
    Confirm,
    /// Closes the dialog before the copy started; stops a running copy after its current batch.
    Cancel,
    /// Closes the report of a finished copy.
    Close,
}

fn none(state: AppState) -> Out {
    (state, Vec::new())
}

fn send(command: CopyCommand) -> Vec<Command> {
    vec![Command::Copy(command)]
}

/// The state with `change` applied to the dialog, if one is open.
fn with_dialog(state: AppState, change: impl FnOnce(CopyDialog) -> CopyDialog) -> AppState {
    let AppState { copy, .. } = state;
    AppState {
        copy: copy.map(change),
        ..state
    }
}

fn next_run(state: AppState) -> (AppState, RunId) {
    let run = state.copy_runs + 1;
    (
        AppState {
            copy_runs: run,
            ..state
        },
        run,
    )
}

pub(super) fn on_input(state: AppState, input: CopyInput) -> Out {
    match input {
        CopyInput::Open { word } => open(state, &word),
        CopyInput::SetPolicy(policy) => none(with_dialog(state, |d| d.with_policy(policy))),
        CopyInput::SetName { index, name } => rename(state, index, name),
        CopyInput::ToggleIndexesAndOptions => {
            none(with_dialog(state, CopyDialog::with_options_toggled))
        }
        CopyInput::TypeConfirmation(text) => none(with_dialog(state, |d| d.with_typed(text))),
        CopyInput::Confirm => confirm(state),
        CopyInput::Cancel => cancel(state),
        CopyInput::Close => close(state),
    }
}

/// Takes the pending paste into a dialog and asks for its counts. A paste that arrives while a
/// dialog is open is dropped.
fn open(state: AppState, word: &str) -> Out {
    let Some(request) = state.pending_paste.clone() else {
        return none(state);
    };
    let state = AppState {
        pending_paste: None,
        ..state
    };
    if state.copy.is_some() {
        return none(state);
    }
    let dialog = CopyDialog::opened(request, word);
    start_preview(AppState {
        copy: Some(dialog),
        ..state
    })
}

/// Sends a preview of the dialog as it is, unless a name is unusable.
fn start_preview(state: AppState) -> Out {
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    if !dialog.names_valid() {
        return none(state);
    }
    let request = dialog.final_request();
    let (state, run) = next_run(state);
    let state = with_dialog(state, |d| d.counting_run(run));
    (state, send(CopyCommand::Preview { run, request }))
}

/// A name was edited: the counts were for another target, so they are dropped and asked again.
fn rename(state: AppState, index: usize, name: String) -> Out {
    let editing = state
        .copy
        .as_ref()
        .is_some_and(|d| matches!(d.phase, CopyPhase::Editing));
    if !editing {
        return none(state);
    }
    let state = with_dialog(state, |d| d.with_name(index, name));
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    match (dialog.preview_run, dialog.restart_preview) {
        // Already being cancelled: the restart picks up the latest names when it ends.
        (Some(_), true) => none(state),
        (Some(run), false) => (
            with_dialog(state, |d| d.with_restart(true)),
            send(CopyCommand::Cancel { run }),
        ),
        (None, _) => start_preview(state),
    }
}

fn confirm(state: AppState) -> Out {
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    let production = state.session_production(dialog.request.target);
    if !dialog.can_confirm(production) {
        return none(state);
    }
    if state.session_read_only(dialog.request.target) {
        return refused_read_only(state);
    }
    match dialog.preview_run {
        Some(preview) => (
            with_dialog(state, |d| d.with_phase(CopyPhase::Starting)),
            send(CopyCommand::Cancel { run: preview }),
        ),
        None => start_copy(state, false),
    }
}

/// The target was locked after the dialog opened: nothing is sent, and the report says so.
fn refused_read_only(state: AppState) -> Out {
    let (state, run) = next_run(state);
    let report = CopyRun::new(run, false).ended(RunOutcome::Failed(CoreError::ReadOnly));
    none(with_dialog(state, |d| {
        d.with_phase(CopyPhase::Running(report))
    }))
}

/// Sends the copy the dialog describes. `counts_incomplete`: the preview was cut short.
fn start_copy(state: AppState, counts_incomplete: bool) -> Out {
    let Some(request) = state.copy.as_ref().map(CopyDialog::final_request) else {
        return none(state);
    };
    let (state, run) = next_run(state);
    let phase = CopyPhase::Running(CopyRun::new(run, counts_incomplete));
    let state = with_dialog(state, |d| d.with_phase(phase).preview_ended(None));
    (state, send(CopyCommand::Start { run, request }))
}

fn cancel(state: AppState) -> Out {
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    match &dialog.phase {
        CopyPhase::Running(run) if run.can_cancel() => {
            let id = run.run;
            (
                with_dialog(state, |d| d.with_run(CopyRun::cancelling)),
                send(CopyCommand::Cancel { run: id }),
            )
        }
        CopyPhase::Running(run) if run.is_finished() => none(close_dialog(state)),
        CopyPhase::Running(_) => none(state),
        CopyPhase::Editing | CopyPhase::Starting => {
            let preview = dialog.preview_run;
            let commands = preview
                .map(|run| send(CopyCommand::Cancel { run }))
                .unwrap_or_default();
            (close_dialog(state), commands)
        }
    }
}

fn close(state: AppState) -> Out {
    let finished = state
        .copy
        .as_ref()
        .and_then(CopyDialog::run)
        .is_some_and(CopyRun::is_finished);
    match finished {
        true => none(close_dialog(state)),
        false => none(state),
    }
}

fn close_dialog(state: AppState) -> AppState {
    AppState {
        copy: None,
        ..state
    }
}

/// The run an event belongs to.
fn run_of(event: &CopyEvent) -> RunId {
    match event {
        CopyEvent::Planned { run, .. }
        | CopyEvent::PreviewProgress { run, .. }
        | CopyEvent::PreviewItemDone { run, .. }
        | CopyEvent::PreviewDone { run }
        | CopyEvent::Progress { run, .. }
        | CopyEvent::ItemDone { run, .. }
        | CopyEvent::Done { run, .. }
        | CopyEvent::Failed { run, .. } => *run,
    }
}

/// Applies an event of the dialog's preview or copy; an event of any other run is dropped.
pub(super) fn on_event(state: AppState, event: CopyEvent) -> Out {
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    let run = run_of(&event);
    if dialog.preview_run == Some(run) {
        return preview_event(state, event);
    }
    match dialog.run() {
        Some(copy) if copy.run == run && !copy.is_finished() => {
            none(with_dialog(state, |d| copy_event(d, event)))
        }
        _ => none(state),
    }
}

fn preview_event(state: AppState, event: CopyEvent) -> Out {
    match event {
        CopyEvent::Planned { items, .. } => none(with_dialog(state, |d| d.with_plan(items))),
        CopyEvent::PreviewProgress { index, scanned, .. } => {
            none(with_dialog(state, |d| d.with_scanned(index, scanned)))
        }
        CopyEvent::PreviewItemDone { index, counts, .. } => {
            none(with_dialog(state, |d| d.with_count(index, counts)))
        }
        CopyEvent::PreviewDone { .. } | CopyEvent::Done { .. } => preview_ended(state, None),
        CopyEvent::Failed { error, .. } => preview_ended(state, Some(error)),
        CopyEvent::Progress { .. } | CopyEvent::ItemDone { .. } => none(state),
    }
}

/// The preview released the run slot: the copy Confirm was waiting for, or a fresh preview for
/// the names edited meanwhile, goes out now.
fn preview_ended(state: AppState, error: Option<CoreError>) -> Out {
    let state = with_dialog(state, |d| d.preview_ended(error));
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    if matches!(dialog.phase, CopyPhase::Starting) {
        return start_copy(state, true);
    }
    match dialog.restart_preview {
        true => start_preview(with_dialog(state, |d| d.with_restart(false))),
        false => none(state),
    }
}

fn copy_event(dialog: CopyDialog, event: CopyEvent) -> CopyDialog {
    dialog.with_run(|run| match event {
        CopyEvent::Planned { items, .. } => run.with_plan(&items),
        CopyEvent::Progress {
            index,
            progress,
            estimated_total,
            ..
        } => run.with_progress(index, progress, estimated_total),
        CopyEvent::ItemDone { index, report, .. } => run.with_item_done(index, report),
        CopyEvent::Done { cancelled, .. } => run.ended(match cancelled {
            true => RunOutcome::Cancelled,
            false => RunOutcome::Completed,
        }),
        CopyEvent::Failed { error, .. } => run.ended(RunOutcome::Failed(error)),
        CopyEvent::PreviewProgress { .. }
        | CopyEvent::PreviewItemDone { .. }
        | CopyEvent::PreviewDone { .. } => run,
    })
}
