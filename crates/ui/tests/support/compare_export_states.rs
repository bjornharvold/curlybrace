//! States for the export view tests: the pending list or the overview of finished pairs, then the
//! export modal in each state, all scripted through the real reducer. No database, no dialog.
use super::compare_apply_states::{
    act, finished, step, with_delete, with_deletes_on_both_sides, with_insert, with_replace,
};
use super::compare_states::SESSION;
use curlybrace_core::compare::protocol::{CompareEvent, ExportOutcome};
use curlybrace_core::runtime::Event;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::compare::{CompareState, ExportKind};
use std::path::PathBuf;

/// The file every fixture export writes to (never created: the tests send no command anywhere).
pub fn out_path() -> PathBuf {
    PathBuf::from("/exports/out.js")
}

pub fn event(state: AppState, e: CompareEvent) -> AppState {
    step(state, Input::Event(Event::Compare(e)))
}

/// Two Different rows queued as a replace (one side only).
pub fn plan() -> AppState {
    with_replace()
}

/// Inserts are queued: the id policy matters.
pub fn plan_with_inserts() -> AppState {
    with_insert()
}

/// Deletes on one side only.
pub fn plan_deletes() -> AppState {
    with_delete()
}

/// Writes to both sides: a script cannot target them.
pub fn plan_mixed() -> AppState {
    with_deletes_on_both_sides()
}

/// One finished pair, back on the overview (the report button lives there).
pub fn overview() -> AppState {
    act(finished(), CompareAction::BackToOverview)
}

/// The overview after its only pair's results were discarded: nothing to report.
pub fn overview_without_results() -> AppState {
    act(overview(), CompareAction::DiscardPair(0))
}

pub fn open(state: AppState, kind: ExportKind) -> AppState {
    let state = act(state, CompareAction::OpenExport(kind));
    assert!(state.compare.export.configuring().is_some(), "modal open");
    state
}

pub fn plan_modal() -> AppState {
    open(plan(), ExportKind::Plan)
}

pub fn report_modal() -> AppState {
    open(overview(), ExportKind::Report)
}

/// The report modal for a pair that holds `rows` differences (over the clipboard cap).
pub fn report_modal_with_rows(rows: usize) -> AppState {
    let state = overview();
    let mut results = state.compare.results.clone();
    for view in results.values_mut() {
        view.counts.different = rows;
        view.counts.only_in_source = 0;
        view.counts.only_in_target = 0;
    }
    let compare = CompareState {
        results,
        ..state.compare.clone()
    };
    open(AppState { compare, ..state }, ExportKind::Report)
}

pub fn choosing(state: AppState) -> AppState {
    let state = act(state, CompareAction::ChooseExportFile);
    assert!(state.compare.export.is_busy(), "the dialog is open");
    state
}

pub fn running(state: AppState) -> AppState {
    let state = act(choosing(state), CompareAction::ExportPathChosen(out_path()));
    assert!(state.compare.export.is_running(), "the job runs");
    state
}

pub fn running_plan() -> AppState {
    running(plan_modal())
}

pub fn running_report() -> AppState {
    running(report_modal())
}

pub fn running_clipboard() -> AppState {
    let state = act(report_modal(), CompareAction::CopyReport);
    assert!(state.compare.export.is_running(), "the job runs");
    state
}

fn run_of(state: &AppState) -> u64 {
    state.compare.export.running_run().expect("an export runs")
}

pub fn progressed(state: AppState, done: u64, total: u64) -> AppState {
    let run = run_of(&state);
    let progress = CompareEvent::ExportProgress {
        session: SESSION,
        run,
        done,
        total,
    };
    event(state, progress)
}

pub fn finished_with(state: AppState, outcome: ExportOutcome) -> AppState {
    let run = run_of(&state);
    let done = CompareEvent::ExportDone {
        session: SESSION,
        run,
        outcome,
    };
    event(state, done)
}

/// The clipboard report arrived and the job ended: the result shows `rows` copied.
pub fn copied(rows: u64) -> AppState {
    let state = running_clipboard();
    let run = run_of(&state);
    let text = CompareEvent::ExportText {
        session: SESSION,
        run,
        text: "pair,source,target,category,key,changed_paths\n".into(),
    };
    let outcome = ExportOutcome {
        written: rows,
        ..ExportOutcome::default()
    };
    finished_with(event(state, text), outcome)
}
