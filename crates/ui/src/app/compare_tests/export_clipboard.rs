//! The clipboard report: its command, the cap, and the text channel (`take_clipboard_text`).
use super::confirm::with_pending;
use super::export::{
    done_event, draft_of, exports, plan_modal, report_modal, run_of, running_clipboard,
    running_report,
};
use super::*;
use crate::viewmodel::compare::{ExportBlock, ExportUi, ResultsView};
use curlybrace_core::compare::export::report::CLIPBOARD_MAX_ROWS;
use curlybrace_core::compare::protocol::{ExportOutcome, ReportTarget};

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
    state
        .compare
        .results
        .insert(RUN_PAIR, ResultsView { counts, ..view });
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
