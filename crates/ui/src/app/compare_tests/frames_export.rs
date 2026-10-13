//! Headless pseudo-locale frames of the export controls and the modal in every state: the entry
//! buttons, the plan and report options, waiting for the dialog, running, the results and the
//! clipboard note. Every string comes from the translation layer.
use super::confirm::{pending_for, tab_of, with_pending};
use super::export::{
    choosing, done_event, plan_modal, report_modal, run_of, running_clipboard, running_plan,
    running_report,
};
use super::frames::{assert_translated, has, workspace};
use super::*;
use crate::viewmodel::compare::{ExportKind, RowAction};
use curlybrace_core::compare::protocol::ExportOutcome;

fn finish(state: AppState, outcome: ExportOutcome) -> AppState {
    let run = run_of(&state);
    update(state, done_event(run, outcome)).0
}

fn progress(state: AppState, done: u64, total: u64) -> AppState {
    let run = run_of(&state);
    let event = CompareEvent::ExportProgress {
        session: 1,
        run,
        done,
        total,
    };
    update(state, Input::Event(Event::Compare(event))).0
}

fn overview() -> AppState {
    update(
        tab_of(Category::Different),
        Input::Compare(CompareAction::BackToOverview),
    )
    .0
}

#[test]
fn the_entry_buttons_are_translated() {
    let texts = assert_translated(&workspace(with_pending()), "export changes button");
    assert!(has(&texts, "Export changes…"));
    let texts = assert_translated(&workspace(overview()), "export report button");
    assert!(has(&texts, "Export report…"));
}

#[test]
fn a_disabled_entry_button_names_its_reason() {
    let mut state = with_pending();
    state.compare.overview.active = Some(crate::viewmodel::compare::ActiveRun {
        run: 5,
        cancel_requested: false,
        failure_noted: false,
        pairs: 1,
    });
    let texts = assert_translated(&workspace(state), "export blocked");
    assert!(has(&texts, "Wait for the comparison to finish."));
}

#[test]
fn the_plan_modal_is_translated() {
    let texts = assert_translated(&workspace(plan_modal()), "plan modal");
    assert!(has(&texts, "Export the pending changes"));
    assert!(has(&texts, "mongosh script"));
    assert!(has(&texts, "JSON plan"));
    assert!(has(&texts, "Label"));
    assert!(has(&texts, "will contain document data"));
    assert!(has(&texts, "Choose file and export"));
    assert!(has(&texts, "Cancel"));
}

#[test]
fn the_plan_modal_with_inserts_offers_the_id_policy() {
    let inserts = pending_for(Category::OnlyInSource, RowAction::CopyToTarget);
    let (state, _) = update(
        inserts,
        Input::Compare(CompareAction::OpenExport(ExportKind::Plan)),
    );
    let texts = assert_translated(&workspace(state), "plan modal with inserts");
    assert!(has(&texts, "Keep the original _id values"));
}

#[test]
fn the_report_modal_is_translated() {
    let texts = assert_translated(&workspace(report_modal()), "report modal");
    assert!(has(&texts, "Export the differences as a report"));
    assert!(has(&texts, "Include the changed field paths"));
    assert!(has(&texts, "Keys can be personal data"));
    assert!(has(&texts, "Copy to clipboard"));
}

#[test]
fn a_report_over_the_clipboard_cap_names_the_reason() {
    let mut state = report_modal();
    for view in state.compare.results.values_mut() {
        view.counts.different = 10_001;
    }
    let texts = assert_translated(&workspace(state), "report over cap");
    assert!(has(&texts, "too many rows for the clipboard"));
}

#[test]
fn waiting_for_the_dialog_is_translated() {
    let texts = assert_translated(&workspace(choosing(plan_modal())), "choosing");
    assert!(has(&texts, "Waiting for the file dialog"));
}

#[test]
fn the_running_states_are_translated() {
    let state = progress(running_plan(), 3, 8);
    let texts = assert_translated(&workspace(state), "running plan");
    assert!(has(&texts, "Exporting"));
    assert!(has(&texts, "3 of 8"));
    assert!(has(&texts, "Cancel export"));
    let texts = assert_translated(&workspace(running_report()), "running report");
    assert!(has(&texts, "0 of 0"));
    let texts = assert_translated(&workspace(running_clipboard()), "running clipboard");
    assert!(has(&texts, "Cancel export"));
    let cancelled = update(running_plan(), Input::Compare(CompareAction::CancelExport)).0;
    let texts = assert_translated(&workspace(cancelled), "cancelling");
    assert!(has(&texts, "Cancelling…"));
}

#[test]
fn a_finished_plan_is_translated() {
    let outcome = ExportOutcome {
        written: 5,
        skipped: 2,
        failed: 1,
        not_run: 3,
        bytes: 1234,
        ..ExportOutcome::default()
    };
    let texts = assert_translated(&workspace(finish(running_plan(), outcome)), "finished");
    assert!(has(&texts, "Export finished"));
    assert!(has(&texts, "Written: 5"));
    assert!(has(&texts, "Skipped: 2"));
    assert!(has(&texts, "Failed: 1"));
    assert!(has(&texts, "Not run: 3"));
    assert!(has(&texts, "Size: 1234"));
    assert!(has(&texts, "Dismiss"));
}

#[test]
fn a_cancelled_export_with_its_partial_file_removed_is_translated() {
    let outcome = ExportOutcome {
        cancelled: true,
        partial_removed: true,
        ..ExportOutcome::default()
    };
    let texts = assert_translated(&workspace(finish(running_plan(), outcome)), "cancelled");
    assert!(has(&texts, "The export was cancelled"));
    assert!(has(&texts, "unfinished file was removed"));
}

#[test]
fn a_partial_file_left_behind_is_translated() {
    let outcome = ExportOutcome {
        cancelled: true,
        file_left: true,
        ..ExportOutcome::default()
    };
    let texts = assert_translated(&workspace(finish(running_plan(), outcome)), "file left");
    assert!(has(&texts, "could not be removed"));
}

#[test]
fn a_finished_report_is_translated() {
    let outcome = ExportOutcome {
        written: 10,
        skipped: 2,
        failed: 1,
        bytes: 900,
        ..ExportOutcome::default()
    };
    let texts = assert_translated(&workspace(finish(running_report(), outcome)), "report");
    assert!(has(&texts, "Rows written: 10"));
    assert!(has(&texts, "empty paths cell"));
}

#[test]
fn the_clipboard_result_is_translated() {
    let state = running_clipboard();
    let run = run_of(&state);
    let text = CompareEvent::ExportText {
        session: 1,
        run,
        text: "pair\n".into(),
    };
    let (state, _) = update(state, Input::Event(Event::Compare(text)));
    let outcome = ExportOutcome {
        written: 7,
        ..ExportOutcome::default()
    };
    let texts = assert_translated(&workspace(finish(state, outcome)), "copied");
    assert!(has(&texts, "Rows copied: 7"));
    assert!(has(&texts, "Dismiss"));
}
