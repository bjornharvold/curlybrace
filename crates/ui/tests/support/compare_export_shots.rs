//! Screenshot states for the export dialogs, scripted through the real reducer on the three-pair
//! run of `compare_apply_states::shots` (`customers`, `invoices` and a third pair). Each builder
//! shapes the `&mut AppState` of one screenshot; nothing opens a native dialog.
use super::compare_apply_states::shots::pending_panel;
use super::compare_states::{self as cs, SESSION};
use curlybrace_core::compare::protocol::{CompareEvent, ExportFormat, ExportOutcome};
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::compare::{CompareState, ExportKind, RowAction};
use std::path::PathBuf;

/// A long path with a deep folder chain, to prove the result wraps instead of overflowing.
const LONG_PATH: &str = "/Users/someone/Documents/database-exports/2026/october/production-replica-to-staging/pending-changes-customers-invoices-orders-2026-10-14.js";

fn long_path() -> PathBuf {
    PathBuf::from(LONG_PATH)
}

fn plan_label(state: &mut AppState) {
    cs::act(
        state,
        CompareAction::SetExportLabel("Before the October migration".into()),
    );
}

/// The pending list is on the overview and the modal for the script is open, a label typed.
pub fn export_plan_modal(state: &mut AppState) {
    pending_panel(state);
    cs::act(state, CompareAction::OpenExport(ExportKind::Plan));
    cs::act(state, CompareAction::SetExportFormat(ExportFormat::Mongosh));
    plan_label(state);
    assert!(
        state.compare.export.configuring().is_some(),
        "the modal is open"
    );
}

/// Writes to both sides are queued: the script is refused with its reason, JSON stays possible.
pub fn export_plan_modal_mixed_sides(state: &mut AppState) {
    pending_panel(state);
    cs::act(state, CompareAction::OpenPair(0));
    cs::act(state, CompareAction::ToggleRow(cs::key(108)));
    cs::act(state, CompareAction::AddToPending(RowAction::CopyToSource));
    cs::act(state, CompareAction::BackToOverview);
    cs::act(state, CompareAction::OpenExport(ExportKind::Plan));
    let draft = state
        .compare
        .export
        .configuring()
        .expect("the modal is open");
    assert_eq!(
        draft.plan.format,
        ExportFormat::JsonPlan,
        "the script is refused"
    );
}

/// The overview with finished pairs, the report modal open: paths included, one pair unticked,
/// and a result set above the clipboard cap so the copy button shows its reason.
pub fn export_report_modal(state: &mut AppState) {
    pending_panel(state);
    let mut results = state.compare.results.clone();
    for view in results.values_mut().take(1) {
        view.counts.different = 12_500;
    }
    let compare = CompareState {
        results,
        ..state.compare.clone()
    };
    state.compare = compare;
    cs::act(state, CompareAction::OpenExport(ExportKind::Report));
    cs::act(state, CompareAction::SetExportIncludePaths(true));
    cs::act(
        state,
        CompareAction::SetExportLabel("Weekly drift check".into()),
    );
    let first = state.compare.completed_entries()[2].0;
    cs::act(state, CompareAction::ToggleExportPair(first));
    assert!(
        state.compare.export.configuring().is_some(),
        "the modal is open"
    );
}

/// Waiting for the native file dialog.
pub fn export_choosing(state: &mut AppState) {
    export_plan_modal(state);
    cs::act(state, CompareAction::ChooseExportFile);
    assert!(state.compare.export.is_busy(), "the dialog is open");
}

fn running_plan(state: &mut AppState) {
    export_choosing(state);
    cs::act(state, CompareAction::ExportPathChosen(long_path()));
    assert!(state.compare.export.is_running(), "the job runs");
}

fn run_of(state: &AppState) -> u64 {
    state.compare.export.running_run().expect("an export runs")
}

/// A plan export part way through.
pub fn export_running(state: &mut AppState) {
    running_plan(state);
    let progress = CompareEvent::ExportProgress {
        session: SESSION,
        run: run_of(state),
        done: 37,
        total: 120,
    };
    cs::event(state, progress);
}

fn done(state: &mut AppState, outcome: ExportOutcome) {
    let event = CompareEvent::ExportDone {
        session: SESSION,
        run: run_of(state),
        outcome,
    };
    cs::event(state, event);
}

/// A cancelled plan export whose partial file could not be removed.
pub fn export_finished_with_file_left(state: &mut AppState) {
    running_plan(state);
    let outcome = ExportOutcome {
        written: 37,
        skipped: 2,
        failed: 1,
        not_run: 80,
        bytes: 48_213,
        cancelled: true,
        partial_removed: false,
        file_left: true,
    };
    done(state, outcome);
}

/// A report written to a file: rows, stale and unreadable rows, the size and the path.
pub fn export_finished_report(state: &mut AppState) {
    export_report_modal(state);
    cs::act(state, CompareAction::ChooseExportFile);
    cs::act(state, CompareAction::ExportPathChosen(long_path()));
    let outcome = ExportOutcome {
        written: 4_705,
        skipped: 3,
        failed: 1,
        bytes: 612_944,
        ..ExportOutcome::default()
    };
    done(state, outcome);
}

/// The report copied to the clipboard.
pub fn export_copied(state: &mut AppState) {
    pending_panel(state);
    cs::act(state, CompareAction::OpenExport(ExportKind::Report));
    cs::act(state, CompareAction::CopyReport);
    let text = CompareEvent::ExportText {
        session: SESSION,
        run: run_of(state),
        text: "pair,source,target,category,key,changed_paths\n".into(),
    };
    cs::event(state, text);
    done(
        state,
        ExportOutcome {
            written: 9,
            ..ExportOutcome::default()
        },
    );
}

/// The overview with the pending panel and its export button, no modal.
pub fn pending_panel_with_export_button(state: &mut AppState) {
    pending_panel(state);
}
