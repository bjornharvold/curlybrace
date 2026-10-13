//! The export workflow of the reducer: gates, the one command builder, events and the busy rules.
use super::confirm::{WORD, tab, tab_of, toggled, wide_deletes, with_pending};
use super::*;
use crate::viewmodel::compare::{
    ExportBlock, ExportDraft, ExportKind, ExportUi, PlanOptions, ReportOptions,
};
use curlybrace_core::compare::apply::IdPolicy;
use curlybrace_core::compare::export::report::CLIPBOARD_MAX_ROWS;
use curlybrace_core::compare::protocol::{ExportFormat, ExportOutcome, ReportTarget};
use std::collections::BTreeSet;
use std::path::PathBuf;
use uuid::Uuid;

pub(super) fn out_path() -> PathBuf {
    PathBuf::from("/exports/out.js")
}

pub(super) fn exports(cmds: &[Command]) -> Vec<&CompareCommand> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Compare(
                c @ (CompareCommand::Export { .. } | CompareCommand::ExportReport { .. }),
            ) => Some(c),
            _ => None,
        })
        .collect()
}

pub(super) fn plan_modal() -> AppState {
    let (state, cmds) = act(with_pending(), CompareAction::OpenExport(ExportKind::Plan));
    assert!(cmds.is_empty());
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
    state
}

pub(super) fn report_modal() -> AppState {
    let state = tab_of(Category::Different);
    let (state, cmds) = act(state, CompareAction::OpenExport(ExportKind::Report));
    assert!(cmds.is_empty());
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
    state
}

pub(super) fn choosing(state: AppState) -> AppState {
    let (state, cmds) = act(state, CompareAction::ChooseExportFile);
    assert!(cmds.is_empty(), "choosing a file sends nothing");
    assert!(matches!(state.compare.export, ExportUi::Choosing(_)));
    state
}

pub(super) fn started(state: AppState) -> (AppState, Vec<Command>) {
    act(choosing(state), CompareAction::ExportPathChosen(out_path()))
}

pub(super) fn running_plan() -> AppState {
    started(plan_modal()).0
}

pub(super) fn running_report() -> AppState {
    started(report_modal()).0
}

fn running_clipboard() -> AppState {
    let (state, cmds) = act(report_modal(), CompareAction::CopyReport);
    assert_eq!(exports(&cmds).len(), 1);
    state
}

/// The run id the next export takes.
fn next_run(state: &AppState) -> u64 {
    state.compare.next_run
}

fn done_event(run: u64, outcome: ExportOutcome) -> Input {
    ev(CompareEvent::ExportDone {
        session: 1,
        run,
        outcome,
    })
}

fn progress(session: u64, run: u64, done: u64, total: u64) -> Input {
    ev(CompareEvent::ExportProgress {
        session,
        run,
        done,
        total,
    })
}

fn run_of(state: &AppState) -> u64 {
    state.compare.export.running_run().expect("an export runs")
}

fn is_failure_banner(state: &AppState) -> bool {
    matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_export",
            ..
        })
    )
}

// ---- gates ----

#[test]
fn open_plan_needs_something_pending() {
    let (state, cmds) = act(
        tab_of(Category::Different),
        CompareAction::OpenExport(ExportKind::Plan),
    );
    assert!(cmds.is_empty());
    assert_eq!(state.compare.export, ExportUi::Idle);
    let blocked = state.compare.export_block(ExportKind::Plan);
    assert_eq!(blocked, Some(ExportBlock::EmptyPending));
}

#[test]
fn open_report_needs_a_completed_pair() {
    let state = checked_state();
    let (state, _) = act(state, CompareAction::OpenExport(ExportKind::Report));
    assert_eq!(state.compare.export, ExportUi::Idle);
    assert_eq!(
        state.compare.export_block(ExportKind::Report),
        Some(ExportBlock::NoCompletedPair)
    );
}

#[test]
fn every_reason_blocks_open_and_names_its_key() {
    let mut not_ready = with_pending();
    not_ready.compare.session = Some(SessionPhase::Opening(1));
    let mut checking = with_pending();
    checking.compare.orphaned_check = Some(77);
    let mut busy = plan_modal();
    busy.compare.export = ExportUi::Choosing(draft_of(&busy));
    let cases = [
        (not_ready, ExportBlock::SessionNotReady),
        (run_with_pending(), ExportBlock::RunActive),
        (checking, ExportBlock::CheckInFlight),
        (super::confirm::confirmed(), ExportBlock::ApplyBusy),
        (busy, ExportBlock::ExportBusy),
    ];
    for (state, reason) in cases {
        assert_eq!(state.compare.export_block(ExportKind::Plan), Some(reason));
        let before = state.compare.export.clone();
        let (after, cmds) = act(state, CompareAction::OpenExport(ExportKind::Plan));
        assert!(cmds.is_empty(), "{reason:?}");
        assert_eq!(after.compare.export, before, "{reason:?}");
    }
    let keys: BTreeSet<&str> = [
        ExportBlock::SessionNotReady,
        ExportBlock::ExportBusy,
        ExportBlock::ApplyBusy,
        ExportBlock::RunActive,
        ExportBlock::CheckInFlight,
        ExportBlock::EmptyPending,
        ExportBlock::NoCompletedPair,
        ExportBlock::NoPairChosen,
        ExportBlock::MixedSides,
        ExportBlock::ClipboardTooLarge,
    ]
    .iter()
    .map(|b| b.i18n_key())
    .collect();
    assert_eq!(keys.len(), 10, "one distinct key per reason");
    assert!(keys.iter().all(|k| k.starts_with("compare.export.reason.")));
}

fn draft_of(state: &AppState) -> ExportDraft {
    state.compare.export.draft().expect("a draft").clone()
}

#[test]
fn open_with_everything_clear_starts_on_the_defaults() {
    let state = plan_modal();
    let draft = draft_of(&state);
    assert_eq!(draft.kind, ExportKind::Plan);
    assert_eq!(draft.plan, PlanOptions::default());
    assert_eq!(draft.plan.format, ExportFormat::Mongosh);
    let report = report_modal();
    let draft = draft_of(&report);
    assert_eq!(draft.kind, ExportKind::Report);
    assert_eq!(draft.report.pairs, BTreeSet::from([ENTRY]));
    assert!(!draft.report.include_paths);
}

#[test]
fn opening_again_while_the_modal_is_open_keeps_the_edits() {
    let (state, _) = act(plan_modal(), CompareAction::SetExportLabel("kept".into()));
    let (state, _) = act(state, CompareAction::OpenExport(ExportKind::Plan));
    assert_eq!(draft_of(&state).plan.label, "kept");
}

// ---- options ----

#[test]
fn options_are_edited_only_while_configuring_and_the_label_is_kept_as_typed() {
    let (state, _) = act(
        plan_modal(),
        CompareAction::SetExportLabel("a\r\nb,=x".into()),
    );
    let (state, _) = act(
        state,
        CompareAction::SetExportFormat(ExportFormat::JsonPlan),
    );
    let (state, _) = act(state, CompareAction::SetExportIdPolicy(IdPolicy::Generate));
    let draft = draft_of(&state);
    assert_eq!(draft.plan.label, "a\r\nb,=x");
    assert_eq!(draft.plan.format, ExportFormat::JsonPlan);
    assert_eq!(draft.plan.id_policy, IdPolicy::Generate);
    let locked = choosing(state);
    let before = locked.compare.export.clone();
    for edit in [
        CompareAction::SetExportLabel("other".into()),
        CompareAction::SetExportFormat(ExportFormat::Mongosh),
        CompareAction::SetExportIdPolicy(IdPolicy::Keep),
        CompareAction::ToggleExportPair(ENTRY),
        CompareAction::SetExportIncludePaths(true),
    ] {
        let (after, cmds) = act(copy_of(&locked), edit);
        assert!(cmds.is_empty());
        assert_eq!(after.compare.export, before);
    }
}

#[test]
fn report_pairs_toggle_and_unknown_entries_are_ignored() {
    let (state, _) = act(report_modal(), CompareAction::ToggleExportPair(ENTRY));
    assert!(draft_of(&state).report.pairs.is_empty());
    let (state, _) = act(state, CompareAction::ToggleExportPair(ENTRY));
    assert_eq!(draft_of(&state).report.pairs, BTreeSet::from([ENTRY]));
    let (state, _) = act(state, CompareAction::ToggleExportPair(99));
    assert_eq!(draft_of(&state).report.pairs, BTreeSet::from([ENTRY]));
}

#[test]
fn a_report_with_no_pair_chosen_cannot_choose_a_file() {
    let (state, _) = act(report_modal(), CompareAction::ToggleExportPair(ENTRY));
    let (state, cmds) = act(state, CompareAction::ChooseExportFile);
    assert!(cmds.is_empty());
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
    let draft = draft_of(&state);
    assert_eq!(
        state.compare.file_block(&draft),
        Some(ExportBlock::NoPairChosen)
    );
}

// ---- choosing and the one command builder ----

#[test]
fn choose_export_file_moves_configuring_to_choosing_only() {
    let (idle, cmds) = act(with_pending(), CompareAction::ChooseExportFile);
    assert!(cmds.is_empty() && idle.compare.export == ExportUi::Idle);
    let modal = choosing(plan_modal());
    let before = modal.compare.export.clone();
    let (again, cmds) = act(modal, CompareAction::ChooseExportFile);
    assert!(cmds.is_empty() && again.compare.export == before);
    let running = running_plan();
    let before = running.compare.export.clone();
    let (after, cmds) = act(running, CompareAction::ChooseExportFile);
    assert!(cmds.is_empty() && after.compare.export == before);
}

#[test]
fn the_path_is_honoured_in_choosing_only() {
    let path = || CompareAction::ExportPathChosen(out_path());
    for state in [with_pending(), plan_modal(), running_plan()] {
        let before = state.compare.export.clone();
        let (after, cmds) = act(state, path());
        assert!(exports(&cmds).is_empty(), "{before:?}");
        assert_eq!(after.compare.export, before);
    }
}

#[test]
fn a_plan_command_carries_exactly_the_options() {
    let (state, _) = act(
        plan_modal(),
        CompareAction::SetExportFormat(ExportFormat::JsonPlan),
    );
    let (state, _) = act(state, CompareAction::SetExportLabel("Q3 sync".into()));
    let (state, _) = act(state, CompareAction::SetExportIdPolicy(IdPolicy::Generate));
    let run = next_run(&state);
    let (ops, bulk) = state.compare.pending.to_command_parts();
    let (state, cmds) = started(state);
    let sent = exports(&cmds);
    assert_eq!(sent.len(), 1);
    let CompareCommand::Export {
        session,
        run: sent_run,
        request,
        path,
    } = sent[0]
    else {
        panic!("a plan export, got {:?}", sent[0]);
    };
    assert_eq!((*session, *sent_run, path), (1, run, &out_path()));
    assert_eq!(request.format, ExportFormat::JsonPlan);
    assert_eq!(request.label, "Q3 sync");
    assert_eq!(request.id_policy, IdPolicy::Generate);
    assert_eq!((&request.ops, &request.bulk), (&ops, &bulk));
    assert!(!ops.is_empty() || !bulk.is_empty());
    assert_eq!(run_of(&state), run);
    assert_eq!(state.compare.next_run, run + 1, "run ids are never reused");
}

#[test]
fn a_report_command_carries_only_completed_pairs_and_the_options() {
    let (state, _) = act(report_modal(), CompareAction::SetExportIncludePaths(true));
    let (mut state, _) = act(state, CompareAction::SetExportLabel("nightly".into()));
    // An entry that is not completed (gone, or never ran) is in the set: it must not be sent.
    state.compare.export = state.compare.export.clone().edited(|d| ExportDraft {
        report: ReportOptions {
            pairs: BTreeSet::from([ENTRY, 7]),
            ..d.report
        },
        ..d
    });
    let (state, cmds) = started(state);
    let sent = exports(&cmds);
    assert_eq!(sent.len(), 1);
    let CompareCommand::ExportReport {
        session,
        run,
        request,
        target,
    } = sent[0]
    else {
        panic!("a report export, got {:?}", sent[0]);
    };
    assert_eq!((*session, *run), (1, run_of(&state)));
    assert_eq!(request.pairs, vec![RUN_PAIR]);
    assert!(request.include_paths);
    assert_eq!(request.label, "nightly");
    assert_eq!(target, &ReportTarget::File(out_path()));
}

#[test]
fn cancelling_the_dialog_returns_to_the_modal_with_the_options() {
    let (state, _) = act(plan_modal(), CompareAction::SetExportLabel("kept".into()));
    let (state, _) = act(choosing(state), CompareAction::ExportPathCancelled);
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
    assert_eq!(draft_of(&state).plan.label, "kept");
    let (idle, cmds) = act(with_pending(), CompareAction::ExportPathCancelled);
    assert!(cmds.is_empty() && idle.compare.export == ExportUi::Idle);
}

#[test]
fn a_changed_state_refuses_the_start_and_sends_nothing() {
    // Mongosh was chosen on writes that land on both sides (reached without the modal's gate).
    let mut state = wide_deletes();
    state.compare.export = ExportUi::Choosing(ExportDraft {
        kind: ExportKind::Plan,
        plan: PlanOptions::default(),
        report: ReportOptions::default(),
    });
    let (state, cmds) = act(state, CompareAction::ExportPathChosen(out_path()));
    assert!(exports(&cmds).is_empty());
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
}

#[test]
fn a_script_for_writes_on_both_sides_is_refused_at_the_gate_json_is_not() {
    let (state, _) = act(wide_deletes(), CompareAction::OpenExport(ExportKind::Plan));
    assert_eq!(
        draft_of(&state).plan.format,
        ExportFormat::JsonPlan,
        "the modal opens on the format that works"
    );
    let (script, _) = act(
        copy_of(&state),
        CompareAction::SetExportFormat(ExportFormat::Mongosh),
    );
    let draft = draft_of(&script);
    assert_eq!(
        script.compare.file_block(&draft),
        Some(ExportBlock::MixedSides)
    );
    let (script, cmds) = act(script, CompareAction::ChooseExportFile);
    assert!(cmds.is_empty() && matches!(script.compare.export, ExportUi::Configuring(_)));
    let (json, cmds) = started(state);
    assert_eq!(exports(&cmds).len(), 1);
    assert!(json.compare.export.is_running());
}

// ---- clipboard ----

#[test]
fn copy_report_sends_one_clipboard_command_and_keeps_the_text_in_state_until_taken() {
    let (state, cmds) = act(report_modal(), CompareAction::CopyReport);
    let sent = exports(&cmds);
    assert_eq!(sent.len(), 1);
    assert!(matches!(
        sent[0],
        CompareCommand::ExportReport {
            target: ReportTarget::Clipboard,
            ..
        }
    ));
    let run = run_of(&state);
    let text = ev(CompareEvent::ExportText {
        session: 1,
        run,
        text: "pair,key\r\n1,secret-key\r\n".into(),
    });
    let (state, cmds) = update(state, text);
    assert!(cmds.is_empty());
    assert!(
        !format!("{:?}", state.compare).contains("secret-key"),
        "never in Debug output"
    );
    let outcome = ExportOutcome {
        written: 1,
        ..ExportOutcome::default()
    };
    let (state, _) = update(state, done_event(run, outcome));
    assert!(matches!(
        state.compare.export,
        ExportUi::Copied { rows: 1, .. }
    ));
    let (state, text) = state.take_clipboard_text();
    assert_eq!(text.as_deref(), Some("pair,key\r\n1,secret-key\r\n"));
    assert!(matches!(
        state.compare.export,
        ExportUi::Copied {
            rows: 1,
            text: None
        }
    ));
    let (_, again) = state.take_clipboard_text();
    assert_eq!(again, None, "handed over once");
}

#[test]
fn copy_report_is_for_reports_within_the_cap() {
    let (state, cmds) = act(plan_modal(), CompareAction::CopyReport);
    assert!(cmds.is_empty() && matches!(state.compare.export, ExportUi::Configuring(_)));
    let (state, cmds) = act(with_pending(), CompareAction::CopyReport);
    assert!(cmds.is_empty() && state.compare.export == ExportUi::Idle);
    let at_cap = with_rows_in_report(CLIPBOARD_MAX_ROWS);
    let (_, cmds) = act(at_cap, CompareAction::CopyReport);
    assert_eq!(exports(&cmds).len(), 1, "the cap itself fits");
    let over = with_rows_in_report(CLIPBOARD_MAX_ROWS + 1);
    let draft = draft_of(&over);
    assert_eq!(
        over.compare.copy_block(&draft),
        Some(ExportBlock::ClipboardTooLarge)
    );
    let (over, cmds) = act(over, CompareAction::CopyReport);
    assert!(cmds.is_empty() && matches!(over.compare.export, ExportUi::Configuring(_)));
    let (_, cmds) = act(over, CompareAction::ChooseExportFile);
    assert!(cmds.is_empty(), "choosing a file stays possible");
}

/// The report modal for a pair whose stored counts add up to `rows`.
fn with_rows_in_report(rows: usize) -> AppState {
    let mut state = report_modal();
    let view = state
        .compare
        .results
        .get(&RUN_PAIR)
        .cloned()
        .expect("results");
    let mut counts = view.counts;
    counts.different = rows;
    counts.only_in_source = 0;
    counts.only_in_target = 0;
    state.compare.results.insert(
        RUN_PAIR,
        crate::viewmodel::compare::ResultsView { counts, ..view },
    );
    state
}

#[test]
fn a_cancelled_clipboard_report_drops_its_text() {
    let state = running_clipboard();
    let run = run_of(&state);
    let text = ev(CompareEvent::ExportText {
        session: 1,
        run,
        text: "x".into(),
    });
    let (state, _) = update(state, text);
    let outcome = ExportOutcome {
        cancelled: true,
        ..ExportOutcome::default()
    };
    let (state, _) = update(state, done_event(run, outcome));
    assert!(matches!(
        state.compare.export,
        ExportUi::Finished { path: None, .. }
    ));
    assert_eq!(state.take_clipboard_text().1, None);
}

#[test]
fn text_for_a_file_export_or_another_run_is_not_kept() {
    let file = running_report();
    let run = run_of(&file);
    let text = |run| {
        ev(CompareEvent::ExportText {
            session: 1,
            run,
            text: "keys".into(),
        })
    };
    let (file, _) = update(file, text(run));
    let (file, _) = update(file, done_event(run, ExportOutcome::default()));
    assert!(matches!(file.compare.export, ExportUi::Finished { .. }));
    assert_eq!(file.take_clipboard_text().1, None);
    let clip = running_clipboard();
    let run = run_of(&clip);
    let before = clip.compare.export.clone();
    let (clip, _) = update(clip, text(run + 5));
    assert_eq!(clip.compare.export, before);
}

// ---- events ----

#[test]
fn progress_and_done_fold_for_the_matching_run_and_session() {
    let state = running_plan();
    let run = run_of(&state);
    let (state, cmds) = update(state, progress(1, run, 3, 9));
    assert!(cmds.is_empty());
    assert!(matches!(
        state.compare.export,
        ExportUi::Running {
            done: 3,
            total: 9,
            ..
        }
    ));
    let before = state.compare.export.clone();
    for stale in [progress(1, run + 1, 5, 9), progress(2, run, 5, 9)] {
        let (state, _) = update(copy_of(&state), stale);
        assert_eq!(state.compare.export, before);
    }
    for stale in [
        done_event(run + 1, ExportOutcome::default()),
        ev(CompareEvent::ExportDone {
            session: 2,
            run,
            outcome: ExportOutcome::default(),
        }),
    ] {
        let (state, _) = update(copy_of(&state), stale);
        assert_eq!(state.compare.export, before);
    }
}

#[test]
fn done_shows_the_outcome_including_cancel_and_what_file_remains() {
    let outcome = ExportOutcome {
        written: 4,
        skipped: 1,
        cancelled: true,
        partial_removed: false,
        file_left: true,
        ..ExportOutcome::default()
    };
    let state = running_plan();
    let run = run_of(&state);
    let (state, _) = update(state, done_event(run, outcome));
    assert_eq!(
        state.compare.export,
        ExportUi::Finished {
            outcome,
            kind: ExportKind::Plan,
            path: Some(out_path()),
        }
    );
    let removed = ExportOutcome {
        partial_removed: true,
        cancelled: true,
        ..ExportOutcome::default()
    };
    let state = running_plan();
    let run = run_of(&state);
    let (state, _) = update(state, done_event(run, removed));
    assert!(matches!(
        state.compare.export,
        ExportUi::Finished { outcome, .. } if outcome.partial_removed && outcome.cancelled
    ));
    let (late, _) = update(state, done_event(run, outcome));
    assert!(
        matches!(late.compare.export, ExportUi::Finished { outcome, .. } if outcome == removed),
        "a second done changes nothing"
    );
}

#[test]
fn a_failed_export_raises_the_banner_and_returns_to_the_modal() {
    let failed = || {
        ev(CompareEvent::Failed {
            session: 1,
            context: "compare_export",
            error: CoreError::NotFound,
        })
    };
    let (state, _) = act(plan_modal(), CompareAction::SetExportLabel("kept".into()));
    let from_running = started(state).0;
    let (state, cmds) = update(from_running, failed());
    assert!(cmds.is_empty() && is_failure_banner(&state));
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
    assert_eq!(draft_of(&state).plan.label, "kept");
    let (state, _) = update(choosing(plan_modal()), failed());
    assert!(is_failure_banner(&state));
    assert!(matches!(state.compare.export, ExportUi::Configuring(_)));
    let (idle, _) = update(with_pending(), failed());
    assert!(is_failure_banner(&idle) && idle.compare.export == ExportUi::Idle);
    let (modal, _) = update(plan_modal(), failed());
    assert!(is_failure_banner(&modal) && matches!(modal.compare.export, ExportUi::Configuring(_)));
}

#[test]
fn a_failure_of_another_session_changes_nothing() {
    let stale = ev(CompareEvent::Failed {
        session: 9,
        context: "compare_export",
        error: CoreError::NotFound,
    });
    let state = running_plan();
    let before = state.compare.export.clone();
    let (state, _) = update(state, stale);
    assert_eq!(state.compare.export, before);
    assert!(state.banner.is_none());
}

// ---- cancel and close ----

#[test]
fn cancel_sends_one_cancel_for_the_running_export() {
    let state = running_plan();
    let run = run_of(&state);
    let (state, cmds) = act(state, CompareAction::CancelExport);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Cancel { session: 1, run: r })] if *r == run
    ));
    assert!(matches!(
        state.compare.export,
        ExportUi::Running {
            cancel_requested: true,
            ..
        }
    ));
    let (_, again) = act(state, CompareAction::CancelExport);
    assert!(again.is_empty(), "one Cancel only");
    for other in [plan_modal(), choosing(plan_modal()), with_pending()] {
        let (_, cmds) = act(other, CompareAction::CancelExport);
        assert!(cmds.is_empty());
    }
}

#[test]
fn close_export_closes_the_modal_and_the_result_but_not_a_job() {
    let (state, _) = act(plan_modal(), CompareAction::CloseExport);
    assert_eq!(state.compare.export, ExportUi::Idle);
    for busy in [choosing(plan_modal()), running_plan()] {
        let before = busy.compare.export.clone();
        let (after, cmds) = act(busy, CompareAction::CloseExport);
        assert!(cmds.is_empty() && after.compare.export == before);
    }
    let state = running_plan();
    let run = run_of(&state);
    let (state, _) = update(state, done_event(run, ExportOutcome::default()));
    let (state, _) = act(state, CompareAction::CloseExport);
    assert_eq!(state.compare.export, ExportUi::Idle);
}

// ---- busy rules ----

fn refused_while_busy() -> Vec<CompareAction> {
    vec![
        CompareAction::Start,
        CompareAction::StartPassing,
        CompareAction::RunCheck,
        CompareAction::Rerun(ENTRY),
        CompareAction::RemovePair(ENTRY),
        CompareAction::DiscardPair(ENTRY),
        CompareAction::FindDuplicates(ENTRY),
        CompareAction::ReviewApply { word: WORD.into() },
        CompareAction::ToggleRow(super::confirm::key(2)),
        CompareAction::SelectAllInCategory,
        CompareAction::AddToPending(RowAction::CopyToSource),
        CompareAction::RemovePending(0),
        CompareAction::ClearPending,
        CompareAction::CloseSession,
        CompareAction::OpenExport(ExportKind::Plan),
        CompareAction::OpenExport(ExportKind::Report),
    ]
}

fn assert_refused(state: AppState, action: CompareAction, what: &str) {
    let export = state.compare.export.clone();
    let pending = state.compare.pending.clone();
    let entries = state.compare.pairs.entries.len();
    let (after, cmds) = act(state, action.clone());
    assert!(cmds.is_empty(), "{action:?} sent {cmds:?} {what}");
    assert_eq!(after.compare.export, export, "{action:?} {what}");
    assert!(
        after.compare.pending.same_changes(&pending),
        "{action:?} {what}"
    );
    assert_eq!(
        after.compare.pairs.entries.len(),
        entries,
        "{action:?} {what}"
    );
    assert!(after.compare.session.is_some(), "{action:?} {what}");
}

#[test]
fn choosing_and_running_refuse_the_apply_busy_set() {
    type Build = (&'static str, fn() -> AppState);
    let builds: [Build; 3] = [
        ("choosing a plan", || choosing(plan_modal())),
        ("running a plan", running_plan),
        ("running a clipboard report", running_clipboard),
    ];
    for (what, build) in builds {
        for action in refused_while_busy() {
            assert_refused(build(), action, what);
        }
        let (other, _) = (build(), ());
        let (b, c) = (Uuid::from_u128(8), Uuid::from_u128(9));
        assert_refused(other, open_action(b, c), what);
    }
}

#[test]
fn the_same_actions_work_again_once_the_export_is_closed() {
    let state = running_plan();
    let run = run_of(&state);
    let (state, _) = update(state, done_event(run, ExportOutcome::default()));
    let (state, _) = act(state, CompareAction::CloseExport);
    let (queued, _) = act(copy_of(&state), CompareAction::RemovePending(0));
    assert!(queued.compare.pending.is_empty(), "remove works again");
    let (state, cmds) = act(state, CompareAction::CloseSession);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Close { session: 1 })]
    ));
    assert!(state.compare.session.is_none());
    let (chosen_back, _) = act(choosing(plan_modal()), CompareAction::ExportPathCancelled);
    let (_, cmds) = act(chosen_back, CompareAction::ClearPending);
    assert!(cmds.is_empty());
}

#[test]
fn closing_the_session_drops_the_export_state() {
    for state in [plan_modal(), running_plan_finished()] {
        let (state, cmds) = act(state, CompareAction::CloseSession);
        assert_eq!(cmds.len(), 1);
        assert_eq!(state.compare.export, ExportUi::Idle);
    }
}

fn running_plan_finished() -> AppState {
    let state = running_plan();
    let run = run_of(&state);
    update(state, done_event(run, ExportOutcome::default())).0
}

/// A compare run is still active and rows are queued (queued while it ran).
fn run_with_pending() -> AppState {
    let ticked = toggled(tab(Category::Different, 3, false), &[0, 1]);
    let (state, _) = act(ticked, CompareAction::AddToPending(RowAction::CopyToTarget));
    assert!(state.compare.is_running() && !state.compare.pending.is_empty());
    state
}
