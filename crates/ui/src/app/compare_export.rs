//! The export workflow of the Compare reducer: the modal's options, the file dialog, the job and
//! its events.
//!
//! `start_export` is the only place that builds `CompareCommand::Export` and `ExportReport`. It is
//! reached from exactly two actions: `ExportPathChosen`, honoured only while the file dialog is
//! open (`Choosing`), and `CopyReport`, honoured only from the report modal (the clipboard, no
//! file). It re-checks the gates against the state as it is now, so a start whose conditions no
//! longer hold sends nothing and falls back to the modal.
//!
//! The clipboard report is the one thing the reducer cannot express as a `Command`: the runtime
//! answers with `ExportText` and the app layer must copy it. There is no effect channel (`Out` is
//! the new state and runtime commands), so the text waits in `ExportUi::Copied` and the app layer
//! takes it once with `AppState::take_clipboard_text` after each update; the state keeps no copy
//! afterwards and its `Debug` never prints it.
use std::path::PathBuf;

use super::compare::{CompareAction, cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{
    CompareState, ExportDraft, ExportKind, ExportUi, PlanOptions, ReportOptions,
};
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, ExportRequest, ReportRequest, ReportTarget,
};
use curlybrace_core::error::CoreError;

impl AppState {
    /// The clipboard report, handed over once: the returned state holds no copy of the text.
    /// The app layer calls this after every update and copies what it gets. Never log it.
    pub fn take_clipboard_text(self) -> (AppState, Option<String>) {
        let (export, text) = self.compare.export.clone().text_taken();
        match text {
            Some(_) => (with_export(self, export), text),
            None => (self, None),
        }
    }
}

fn with_export(state: AppState, export: ExportUi) -> AppState {
    let compare = CompareState {
        export,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

pub(super) fn on_action(state: AppState, action: CompareAction) -> Out {
    match action {
        CompareAction::OpenExport(kind) => open(state, kind),
        CompareAction::SetExportFormat(format) => edit(state, |d| ExportDraft {
            plan: PlanOptions { format, ..d.plan },
            ..d
        }),
        CompareAction::SetExportLabel(label) => edit(state, |d| with_label(d, label)),
        CompareAction::SetExportIdPolicy(id_policy) => edit(state, |d| ExportDraft {
            plan: PlanOptions {
                id_policy,
                ..d.plan
            },
            ..d
        }),
        CompareAction::SetExportIncludePaths(include_paths) => edit(state, |d| ExportDraft {
            report: ReportOptions {
                include_paths,
                ..d.report
            },
            ..d
        }),
        CompareAction::ToggleExportPair(entry) => toggle_pair(state, entry),
        CompareAction::ChooseExportFile => choose_file(state),
        CompareAction::ExportPathChosen(path) => path_chosen(state, path),
        CompareAction::ExportPathCancelled => back_to_modal(state),
        CompareAction::CopyReport => copy_report(state),
        CompareAction::CancelExport => cancel(state),
        CompareAction::CloseExport => close(state),
        // `compare.rs` routes only the export actions here.
        _ => none(state),
    }
}

/// The label belongs to the kind the modal shows; it is stored as typed (core sanitises it).
fn with_label(d: ExportDraft, label: String) -> ExportDraft {
    match d.kind {
        ExportKind::Plan => ExportDraft {
            plan: PlanOptions { label, ..d.plan },
            ..d
        },
        ExportKind::Report => ExportDraft {
            report: ReportOptions { label, ..d.report },
            ..d
        },
    }
}

fn edit(state: AppState, change: impl FnOnce(ExportDraft) -> ExportDraft) -> Out {
    let export = state.compare.export.clone().edited(change);
    none(with_export(state, export))
}

/// Opens the modal when the gates allow and no modal is open (its edits are kept).
fn open(state: AppState, kind: ExportKind) -> Out {
    let c = &state.compare;
    if c.export.configuring().is_some() || c.export_block(kind).is_some() {
        return none(state);
    }
    let draft = ExportDraft {
        kind,
        plan: PlanOptions {
            format: c.default_plan_format(),
            ..PlanOptions::default()
        },
        report: ReportOptions {
            pairs: c.completed_entries().into_iter().map(|(e, _)| e).collect(),
            ..ReportOptions::default()
        },
    };
    none(with_export(state, ExportUi::Configuring(draft)))
}

/// Ticks or unticks an entry of the report; only an entry with finished results can be ticked.
fn toggle_pair(state: AppState, entry: u32) -> Out {
    let completed: Vec<u32> = state
        .compare
        .completed_entries()
        .into_iter()
        .map(|(e, _)| e)
        .collect();
    edit(state, |d| {
        let mut pairs = d.report.pairs.clone();
        if !pairs.remove(&entry) && completed.contains(&entry) {
            pairs.insert(entry);
        }
        ExportDraft {
            report: ReportOptions { pairs, ..d.report },
            ..d
        }
    })
}

/// The file dialog opens (the app layer reacts to the new state); nothing is sent.
fn choose_file(state: AppState) -> Out {
    let allowed = state
        .compare
        .export
        .configuring()
        .is_some_and(|d| state.compare.file_block(d).is_none());
    match allowed {
        true => {
            let export = state.compare.export.clone().choosing();
            none(with_export(state, export))
        }
        false => none(state),
    }
}

/// The dialog was cancelled: back to the modal. Ignored when no dialog is open.
fn back_to_modal(state: AppState) -> Out {
    match state.compare.export {
        ExportUi::Choosing(_) => {
            let export = state.compare.export.clone().reconfiguring();
            none(with_export(state, export))
        }
        _ => none(state),
    }
}

/// The dialog answered. Only an open dialog (`Choosing`) can start the file export.
fn path_chosen(state: AppState, path: PathBuf) -> Out {
    let draft = match &state.compare.export {
        ExportUi::Choosing(draft) => draft.clone(),
        _ => return none(state),
    };
    start_export(state, draft, Some(path))
}

/// Copies the report to the clipboard from the modal; no file, no dialog.
fn copy_report(state: AppState) -> Out {
    let draft = match &state.compare.export {
        ExportUi::Configuring(draft) if draft.kind == ExportKind::Report => draft.clone(),
        _ => return none(state),
    };
    start_export(state, draft, None)
}

/// The only constructor of `CompareCommand::Export` and `CompareCommand::ExportReport`.
///
/// `path` is the chosen file, `None` for the clipboard. Whatever the gates say now (the state may
/// have moved since the modal opened) decides: blocked or unbuildable means the modal again and
/// nothing sent.
fn start_export(state: AppState, draft: ExportDraft, path: Option<PathBuf>) -> Out {
    let c = &state.compare;
    let blocked = match path {
        Some(_) => c.file_block(&draft),
        None => c.copy_block(&draft),
    };
    let run = c.next_run;
    let command = ready(&state)
        .filter(|_| blocked.is_none())
        .and_then(|session| command_for(c, session, run, &draft, path.as_ref()));
    let Some(command) = command else {
        return none(with_export(state, ExportUi::Configuring(draft)));
    };
    let compare = CompareState {
        export: ExportUi::Choosing(draft).started(run, path),
        next_run: run + 1,
        ..c.clone()
    };
    (with_compare(state, compare), vec![cmd(command)])
}

fn command_for(
    c: &CompareState,
    session: u64,
    run: u64,
    draft: &ExportDraft,
    path: Option<&PathBuf>,
) -> Option<CompareCommand> {
    match (draft.kind, path) {
        (ExportKind::Plan, Some(path)) => {
            let (ops, bulk) = c.pending.to_command_parts();
            Some(CompareCommand::Export {
                session,
                run,
                request: ExportRequest {
                    format: draft.plan.format,
                    label: draft.plan.label.clone(),
                    id_policy: draft.plan.id_policy,
                    ops,
                    bulk,
                },
                path: path.clone(),
            })
        }
        (ExportKind::Plan, None) => None,
        (ExportKind::Report, path) => Some(CompareCommand::ExportReport {
            session,
            run,
            request: ReportRequest {
                pairs: c.chosen_pairs(draft).into_iter().map(|(_, p)| p).collect(),
                include_paths: draft.report.include_paths,
                label: draft.report.label.clone(),
            },
            target: path.map_or(ReportTarget::Clipboard, |p| ReportTarget::File(p.clone())),
        }),
    }
}

/// Sends Cancel once for the running export.
fn cancel(state: AppState) -> Out {
    let c = &state.compare;
    let (Some(session), Some(run)) = (ready(&state), c.export.running_run()) else {
        return none(state);
    };
    if !c.export.can_cancel() {
        return none(state);
    }
    let export = c.export.clone().cancelling();
    (
        with_export(state, export),
        vec![cmd(CompareCommand::Cancel { session, run })],
    )
}

/// Closes the modal or the result; a dialog that is open and a job that runs stay.
fn close(state: AppState) -> Out {
    let export = state.compare.export.clone().closed();
    none(with_export(state, export))
}

/// Events of the export run. `compare::on_event` has checked the session; the state ignores
/// every other run, and what it ignores changes nothing.
pub(super) fn on_event(state: AppState, ev: CompareEvent) -> Out {
    let before = state.compare.export.clone();
    let after = match ev {
        CompareEvent::ExportProgress {
            run, done, total, ..
        } => before.clone().progressed(run, done, total),
        CompareEvent::ExportText { run, text, .. } => before.clone().text_arrived(run, text),
        CompareEvent::ExportDone { run, outcome, .. } => before.clone().finished(run, outcome),
        _ => return none(state),
    };
    match after == before {
        true => none(state),
        false => none(with_export(state, after)),
    }
}

/// The export ended without a `ExportDone` (refused, or the file could not be written): the
/// banner shows it and the modal is open again with the options kept.
pub(super) fn failed(state: AppState, error: CoreError) -> Out {
    // Only a running job is ended by a failure. One that arrives while the dialog is open is
    // stale (nothing was sent yet): the banner shows and the dialog's answer still counts.
    let export = match state.compare.export {
        ExportUi::Running { .. } => state.compare.export.clone().reconfiguring(),
        _ => state.compare.export.clone(),
    };
    none(failure_banner(
        with_export(state, export),
        "compare_export",
        error,
    ))
}
