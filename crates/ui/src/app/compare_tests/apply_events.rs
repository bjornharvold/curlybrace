//! Apply events, failures and the work an apply blocks.
use super::confirm::{key, running_apply};
use super::*;
use crate::viewmodel::compare::{ApplyUi, PairStatus, RowAction};
use curlybrace_core::compare::apply::ApplyReport;

fn progress(session: u64, run: u64, pair: u32, done: u64) -> Input {
    ev(CompareEvent::ApplyProgress {
        session,
        run,
        pair,
        done,
        total: 2,
    })
}

fn pair_done_ev(session: u64, run: u64, pair: u32, report: ApplyReport) -> Input {
    ev(CompareEvent::ApplyPairDone {
        session,
        run,
        pair,
        report,
    })
}

fn applied(n: u64) -> ApplyReport {
    ApplyReport {
        applied: n,
        ..ApplyReport::default()
    }
}

fn done_ev(session: u64, run: u64) -> Input {
    ev(CompareEvent::ApplyDone { session, run })
}

fn row_applied(state: &AppState) -> bool {
    state
        .compare
        .overview
        .rows
        .iter()
        .find(|r| r.pair == RUN_PAIR)
        .is_some_and(|r| r.applied)
}

#[test]
fn progress_and_pair_done_fold_into_the_running_apply() {
    let (state, cmds) = update(running_apply(), progress(1, 2, RUN_PAIR, 1));
    assert!(cmds.is_empty());
    let ApplyUi::Running { rows, .. } = &state.compare.apply else {
        panic!("running")
    };
    assert_eq!(rows[0].done, 1);
    let (state, _) = update(state, pair_done_ev(1, 2, RUN_PAIR, applied(2)));
    let ApplyUi::Running { rows, .. } = &state.compare.apply else {
        panic!("running")
    };
    assert_eq!(rows[0].report.as_ref().unwrap().applied, 2);
    assert!(
        row_applied(&state),
        "the pair is stale once something was written"
    );
    assert!(state.compare.pending.is_empty() && state.compare.selection == Default::default());
}

#[test]
fn a_pair_that_wrote_nothing_keeps_its_pending_changes() {
    let stale = ApplyReport {
        stale: 2,
        ..ApplyReport::default()
    };
    let (state, _) = update(running_apply(), pair_done_ev(1, 2, RUN_PAIR, stale));
    assert!(!row_applied(&state));
    assert!(!state.compare.pending.is_empty());
}

#[test]
fn apply_done_finishes_marks_rows_and_clears_pending_and_selection() {
    let (state, _) = update(running_apply(), pair_done_ev(1, 2, RUN_PAIR, applied(2)));
    let (state, cmds) = update(state, done_ev(1, 2));
    assert!(cmds.is_empty());
    let ApplyUi::Finished {
        cancelled,
        requested,
        ..
    } = &state.compare.apply
    else {
        panic!("finished: {:?}", state.compare.apply)
    };
    assert!(!cancelled && *requested == 2);
    assert_eq!(state.compare.apply.summary().unwrap().applied, 2);
    assert!(row_applied(&state));
    assert!(state.compare.pending.is_empty());
    assert!(matches!(
        state.compare.overview.rows[0].status,
        PairStatus::Completed(_)
    ));
}

#[test]
fn apply_done_without_any_pair_report_counts_everything_as_not_run() {
    let (state, _) = update(running_apply(), done_ev(1, 2));
    let summary = state.compare.apply.summary().unwrap();
    assert_eq!((summary.not_run, summary.applied), (2, 0));
    assert!(!row_applied(&state));
    assert!(
        !state.compare.pending.is_empty(),
        "nothing was written, nothing is cleared"
    );
}

#[test]
fn events_of_another_run_or_session_change_nothing() {
    let before = running_apply();
    for input in [
        progress(1, 1, RUN_PAIR, 1),
        progress(5, 2, RUN_PAIR, 1),
        pair_done_ev(1, 9, RUN_PAIR, applied(2)),
        pair_done_ev(5, 2, RUN_PAIR, applied(2)),
        pair_done_ev(1, 2, 77, applied(2)),
        done_ev(1, 9),
        done_ev(5, 2),
    ] {
        let (after, cmds) = update(copy_of(&before), input);
        assert!(cmds.is_empty());
        assert_eq!(after.compare.apply, before.compare.apply);
        assert!(!row_applied(&after));
        assert_eq!(after.compare.pending, before.compare.pending);
    }
}

#[test]
fn late_events_after_dismiss_change_nothing() {
    let (state, _) = update(running_apply(), done_ev(1, 2));
    let (state, _) = act(state, CompareAction::DismissApplyResult);
    assert_eq!(state.compare.apply, ApplyUi::Idle);
    for input in [
        progress(1, 2, RUN_PAIR, 1),
        pair_done_ev(1, 2, RUN_PAIR, applied(2)),
        done_ev(1, 2),
    ] {
        let (after, _) = update(copy_of(&state), input);
        assert_eq!(after.compare.apply, ApplyUi::Idle);
        assert!(!row_applied(&after));
    }
}

#[test]
fn dismiss_closes_a_result_but_not_a_running_apply() {
    let (running, _) = act(running_apply(), CompareAction::DismissApplyResult);
    assert!(running.compare.apply.is_running());
    let (state, _) = update(running, done_ev(1, 2));
    let (state, _) = act(state, CompareAction::DismissApplyResult);
    assert_eq!(state.compare.apply, ApplyUi::Idle);
}

#[test]
fn a_refused_apply_raises_the_banner_returns_to_idle_and_keeps_pending() {
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_apply",
        error: CoreError::NotFound,
    });
    let (state, cmds) = update(running_apply(), failed);
    assert!(cmds.is_empty());
    assert_eq!(state.compare.apply, ApplyUi::Idle);
    assert!(!state.compare.pending.is_empty());
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_apply",
            ..
        })
    ));
}

#[test]
fn a_failed_event_of_another_session_is_ignored() {
    let failed = ev(CompareEvent::Failed {
        session: 4,
        context: "compare_apply",
        error: CoreError::NotFound,
    });
    let before = running_apply();
    let (state, _) = update(copy_of(&before), failed);
    assert_eq!(state.compare.apply, before.compare.apply);
    assert!(state.banner.is_none());
}

#[test]
fn closing_or_replacing_the_session_returns_to_idle() {
    let (closed, _) = act(running_apply(), CompareAction::CloseSession);
    assert_eq!(closed.compare.apply, ApplyUi::Idle);
    let running = running_apply();
    let (a, b) = running.compare.connections.unwrap();
    let (replaced, _) = act(running, open_action(b, a));
    assert_eq!(replaced.compare.apply, ApplyUi::Idle);
    let (reviewing, _) = act(super::confirm::confirmed(), CompareAction::CloseSession);
    assert_eq!(reviewing.compare.apply, ApplyUi::Idle);
}

/// Every action that starts or changes other work, fired while `state` applies (or reviews).
fn blocked_actions() -> Vec<CompareAction> {
    vec![
        CompareAction::Start,
        CompareAction::StartPassing,
        CompareAction::RunCheck,
        CompareAction::Rerun(ENTRY),
        CompareAction::RemovePair(ENTRY),
        CompareAction::DiscardPair(ENTRY),
        CompareAction::FindDuplicates(ENTRY),
    ]
}

#[test]
fn work_is_refused_while_an_apply_runs_or_is_reviewed() {
    for build in [running_apply, super::confirm::confirmed] {
        for action in blocked_actions() {
            let before = build();
            let (after, cmds) = act(copy_of(&before), action.clone());
            assert!(cmds.is_empty(), "{action:?}");
            assert_eq!(after.compare.apply, before.compare.apply, "{action:?}");
            assert_eq!(after.compare.pending, before.compare.pending, "{action:?}");
            assert_eq!(after.compare.results, before.compare.results, "{action:?}");
            assert_eq!(
                after.compare.overview, before.compare.overview,
                "{action:?}"
            );
        }
    }
}

#[test]
fn the_pending_list_and_selection_are_read_only_while_busy() {
    for build in [running_apply, super::confirm::confirmed] {
        let before = build();
        for action in [
            CompareAction::ToggleRow(key(2)),
            CompareAction::SelectPage,
            CompareAction::SelectAllInCategory,
            CompareAction::ClearSelection,
            CompareAction::AddToPending(RowAction::CopyToSource),
            CompareAction::AddKeyToPending(key(2), RowAction::CopyToSource),
            CompareAction::RemovePending(0),
            CompareAction::ClearPending,
        ] {
            let (after, _) = act(copy_of(&before), action.clone());
            assert_eq!(after.compare.pending, before.compare.pending, "{action:?}");
            assert_eq!(
                after.compare.selection, before.compare.selection,
                "{action:?}"
            );
        }
    }
}

#[test]
fn work_resumes_once_the_result_is_dismissed() {
    let (state, _) = update(running_apply(), done_ev(1, 2));
    let (state, _) = act(state, CompareAction::DismissApplyResult);
    let (_, cmds) = act(state, CompareAction::DiscardPair(ENTRY));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard {
            pair: RUN_PAIR,
            ..
        })]
    ));
}
