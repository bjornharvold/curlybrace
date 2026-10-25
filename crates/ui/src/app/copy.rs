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
    /// Show the dialog for the pending paste.
    Open,
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

/// The state with `change` applied to the dialog while it is being edited: once Confirm was
/// pressed the request is fixed, whatever input still arrives.
fn with_editing_dialog(state: AppState, change: impl FnOnce(CopyDialog) -> CopyDialog) -> AppState {
    let editing = state
        .copy
        .as_ref()
        .is_some_and(|d| matches!(d.phase, CopyPhase::Editing));
    match editing {
        true => with_dialog(state, change),
        false => state,
    }
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
        CopyInput::Open => open(state),
        CopyInput::SetPolicy(policy) => none(with_editing_dialog(state, |d| d.with_policy(policy))),
        CopyInput::SetName { index, name } => rename(state, index, name),
        CopyInput::ToggleIndexesAndOptions => {
            none(with_editing_dialog(state, CopyDialog::with_options_toggled))
        }
        CopyInput::TypeConfirmation(text) => {
            none(with_editing_dialog(state, |d| d.with_typed(text)))
        }
        CopyInput::Confirm => confirm(state),
        CopyInput::Cancel => cancel(state),
        CopyInput::Close => close(state),
    }
}

/// Takes the pending paste into a dialog and asks for its counts. A paste that arrives while a
/// dialog is open is dropped.
fn open(state: AppState) -> Out {
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
    let dialog = CopyDialog::opened(request);
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
    if state.copy_draining.is_some() {
        // The slot is still taken by a cancelled preview: the dialog shows it is counting and
        // the preview goes out when that run ends (`drain_ended`).
        return none(with_dialog(state, CopyDialog::preview_held));
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
    // A preview that a rename already cancelled is not cancelled twice.
    match (dialog.preview_run, dialog.restart_preview) {
        (Some(preview), restarting) => (
            with_dialog(state, |d| d.with_phase(CopyPhase::Starting)),
            match restarting {
                true => Vec::new(),
                false => send(CopyCommand::Cancel { run: preview }),
            },
        ),
        // The slot is held by a cancelled preview of a closed dialog: the copy waits for it.
        (None, _) if state.copy_draining.is_some() => {
            none(with_dialog(state, |d| d.with_phase(CopyPhase::Starting)))
        }
        (None, _) => start_copy(state, false),
    }
}

/// The target was locked after the dialog opened: nothing is sent, and the report says so.
fn refused_read_only(state: AppState) -> Out {
    let dialog = state.copy.as_ref();
    let preview = dialog.and_then(|d| d.preview_run);
    let already_cancelling = dialog.is_some_and(|d| d.restart_preview);
    let (state, run) = next_run(state);
    let report = CopyRun::new(run, false).ended(RunOutcome::Failed(CoreError::ReadOnly));
    // No preview follows: a rename that was waiting for the slot is dropped with it.
    let state = with_dialog(state, |d| {
        d.with_phase(CopyPhase::Running(report)).with_restart(false)
    });
    // The preview still holds the run slot: it is stopped unless a rename already stopped it.
    let commands = match (preview, already_cancelling) {
        (Some(run), false) => send(CopyCommand::Cancel { run }),
        _ => Vec::new(),
    };
    (state, commands)
}

/// Sends the copy the dialog describes. `counts_incomplete`: the preview was cut short.
fn start_copy(state: AppState, counts_incomplete: bool) -> Out {
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    // The dialog may have changed since Confirm (the target became production): it is checked
    // again, and a copy that no longer passes goes back to the person.
    let production = state.session_production(dialog.request.target);
    if !(dialog.names_valid() && dialog.confirmation_passes(production)) {
        return none(with_dialog(state, |d| {
            d.preview_ended(None).with_phase(CopyPhase::Editing)
        }));
    }
    let request = dialog.final_request();
    let (state, run) = next_run(state);
    let structure_requested = request.with_indexes_and_options;
    let phase = CopyPhase::Running(
        CopyRun::new(run, counts_incomplete).with_structure_requested(structure_requested),
    );
    let state = with_dialog(state, |d| {
        d.with_phase(phase).preview_ended(None).with_restart(false)
    });
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

/// Closes the dialog. A preview still in flight keeps the run slot until its terminal event, so
/// its id is remembered; the held preview or copy of a later dialog waits for it.
fn close_dialog(state: AppState) -> AppState {
    let in_flight = state.copy.as_ref().and_then(|d| d.preview_run);
    AppState {
        copy_draining: in_flight.or(state.copy_draining),
        copy: None,
        ..state
    }
}

/// An event of the cancelled preview that is still draining: only its end matters. It frees the
/// slot, and the copy or the preview the open dialog was holding goes out now.
fn drain_event(state: AppState, event: CopyEvent) -> Out {
    let ends = matches!(
        event,
        CopyEvent::PreviewDone { .. } | CopyEvent::Done { .. } | CopyEvent::Failed { .. }
    );
    if !ends {
        return none(state);
    }
    let state = AppState {
        copy_draining: None,
        ..state
    };
    let Some(dialog) = &state.copy else {
        return none(state);
    };
    match (&dialog.phase, dialog.restart_preview) {
        (CopyPhase::Starting, _) => start_copy(state, true),
        (CopyPhase::Editing, true) => start_preview(with_dialog(state, |d| d.with_restart(false))),
        _ => none(state),
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
    let run = run_of(&event);
    if state.copy_draining == Some(run) {
        return drain_event(state, event);
    }
    let Some(dialog) = &state.copy else {
        return none(state);
    };
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
