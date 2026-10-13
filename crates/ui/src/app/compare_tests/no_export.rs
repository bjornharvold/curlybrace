//! The export gate: no sequence of user actions or events sends `Export`/`ExportReport` except
//! `ExportPathChosen` from a file dialog that is open (the file) and `CopyReport` from the
//! report modal (the clipboard), and nothing in any export state sends `Apply`.
use super::confirm::{applies, with_pending};
use super::export::{
    choosing, exports, out_path, plan_modal, report_modal, running_plan, running_report,
};
use super::no_write::all_actions;
use super::*;
use crate::viewmodel::compare::{ExportKind, ExportUi};
use curlybrace_core::compare::apply::IdPolicy;
use curlybrace_core::compare::protocol::ExportFormat;
use curlybrace_core::compare::protocol::ExportOutcome;
use std::path::PathBuf;

/// (name, builder, the action that may send an export there, if any)
type StartState = (&'static str, fn() -> AppState, Option<CompareAction>);

fn path_chosen() -> Option<CompareAction> {
    Some(CompareAction::ExportPathChosen(out_path()))
}

/// Every action of the exhaustive chain plus the other payloads of the export actions.
fn actions() -> Vec<CompareAction> {
    let extra = [
        CompareAction::OpenExport(ExportKind::Report),
        CompareAction::SetExportFormat(ExportFormat::Mongosh),
        CompareAction::SetExportIdPolicy(IdPolicy::Keep),
        CompareAction::SetExportIncludePaths(false),
        CompareAction::ToggleExportPair(99),
        CompareAction::ExportPathChosen(PathBuf::from("/exports/other")),
    ];
    all_actions().into_iter().chain(extra).collect()
}

fn cancelling_plan() -> AppState {
    let state = act(running_plan(), CompareAction::CancelExport).0;
    assert!(matches!(
        state.compare.export,
        ExportUi::Running {
            cancel_requested: true,
            ..
        }
    ));
    state
}

fn finished() -> AppState {
    let state = running_plan();
    let run = state.compare.export.running_run().expect("running");
    let done = ev(CompareEvent::ExportDone {
        session: 1,
        run,
        outcome: ExportOutcome::default(),
    });
    update(state, done).0
}

fn copied() -> AppState {
    let state = act(report_modal(), CompareAction::CopyReport).0;
    let run = state.compare.export.running_run().expect("running");
    let text = ev(CompareEvent::ExportText {
        session: 1,
        run,
        text: "t".into(),
    });
    let (state, _) = update(state, text);
    let outcome = ExportOutcome {
        written: 1,
        ..ExportOutcome::default()
    };
    let done = ev(CompareEvent::ExportDone {
        session: 1,
        run,
        outcome,
    });
    update(state, done).0
}

fn states() -> Vec<StartState> {
    vec![
        ("idle with pending", with_pending, None),
        (
            "idle with results",
            || super::confirm::tab_of(Category::Different),
            None,
        ),
        ("plan modal", plan_modal, None),
        (
            "report modal",
            report_modal,
            Some(CompareAction::CopyReport),
        ),
        (
            "plan, dialog open",
            || choosing(plan_modal()),
            path_chosen(),
        ),
        (
            "report, dialog open",
            || choosing(report_modal()),
            path_chosen(),
        ),
        ("plan running", running_plan, None),
        ("report running", running_report, None),
        ("plan running, cancel requested", cancelling_plan, None),
        ("finished", finished, None),
        ("copied", copied, None),
    ]
}

/// May this action send an export from this state?
fn allowed(action: &CompareAction, only: &Option<CompareAction>) -> bool {
    matches!(
        (action, only),
        (
            CompareAction::ExportPathChosen(_),
            Some(CompareAction::ExportPathChosen(_))
        ) | (CompareAction::CopyReport, Some(CompareAction::CopyReport))
    )
}

#[test]
fn only_the_path_chosen_from_the_dialog_and_the_clipboard_copy_send_an_export() {
    for (name, build, only) in states() {
        for action in actions() {
            let (state, cmds) = act(build(), action.clone());
            let sent = exports(&cmds).len();
            assert_eq!(
                sent,
                usize::from(allowed(&action, &only)),
                "{action:?} in {name}"
            );
            assert!(applies(&cmds).is_empty(), "{action:?} in {name} sent Apply");
            if sent == 1 {
                assert!(state.compare.export.is_running(), "{action:?} in {name}");
            }
        }
    }
}

#[test]
fn a_second_path_chosen_sends_nothing() {
    let (state, cmds) = act(
        choosing(plan_modal()),
        CompareAction::ExportPathChosen(out_path()),
    );
    assert_eq!(exports(&cmds).len(), 1);
    let again = CompareAction::ExportPathChosen(out_path());
    let (state, cmds) = act(state, again.clone());
    assert!(exports(&cmds).is_empty());
    let (_, cmds) = act(state, again);
    assert!(exports(&cmds).is_empty());
}

#[test]
fn a_dialog_answer_after_the_modal_was_cancelled_sends_nothing() {
    let (state, _) = act(choosing(plan_modal()), CompareAction::ExportPathCancelled);
    let (_, cmds) = act(state, CompareAction::ExportPathChosen(out_path()));
    assert!(exports(&cmds).is_empty(), "the dialog is no longer open");
}

/// Walks every action but the two senders, in both orders, from every state.
#[test]
fn no_sequence_without_the_two_senders_sends_an_export() {
    let walk: Vec<CompareAction> = actions()
        .into_iter()
        .filter(|a| {
            !matches!(
                a,
                CompareAction::ExportPathChosen(_) | CompareAction::CopyReport
            )
        })
        .collect();
    let backward: Vec<CompareAction> = walk.iter().rev().cloned().collect();
    for (name, build, _) in states() {
        for order in [&walk, &backward] {
            let mut state = build();
            for action in order.iter() {
                let (next, cmds) = act(state, action.clone());
                assert!(exports(&cmds).is_empty(), "{action:?} after {name}");
                assert!(applies(&cmds).is_empty(), "{action:?} after {name}");
                state = next;
            }
        }
    }
}

#[test]
fn no_runtime_event_sends_an_export() {
    let outcome = ExportOutcome::default();
    let events = vec![
        CompareEvent::ExportProgress {
            session: 1,
            run: 3,
            done: 1,
            total: 2,
        },
        CompareEvent::ExportText {
            session: 1,
            run: 3,
            text: "t".into(),
        },
        CompareEvent::ExportDone {
            session: 1,
            run: 3,
            outcome,
        },
        CompareEvent::Failed {
            session: 1,
            context: "compare_export",
            error: CoreError::NotFound,
        },
        CompareEvent::RunDone { session: 1, run: 1 },
        CompareEvent::Opened { session: 1 },
    ];
    for (name, build, _) in states() {
        for event in &events {
            let (_, cmds) = update(build(), ev(event.clone()));
            assert!(exports(&cmds).is_empty(), "{event:?} in {name}");
            assert!(applies(&cmds).is_empty(), "{event:?} in {name}");
        }
    }
}
