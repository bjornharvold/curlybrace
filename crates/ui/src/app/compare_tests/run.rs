//! Starting, running, cancelling and finishing a run.
use super::*;

#[test]
fn one_terminal_event_per_pair_and_the_run_terminal_update_the_run_view() {
    let state = running_state();
    let (state, _) = update(state, pair_done(1, Summary::default()));
    assert!(matches!(
        state.compare.run,
        RunView::Running {
            finished_pairs: 1,
            ..
        }
    ));
    let (state, _) = update(state, run_done());
    assert!(matches!(state.compare.run, RunView::Finished { .. }));
}

#[test]
fn a_late_pair_done_for_an_older_run_is_ignored() {
    let (after, cmds) = update(running_state(), pair_done(0, Summary::default()));
    assert!(matches!(
        after.compare.run,
        RunView::Running {
            finished_pairs: 0,
            ..
        }
    ));
    assert!(cmds.is_empty());
}

#[test]
fn run_one_events_after_run_two_started_are_ignored() {
    let (state, _) = act(finished_with_results(), CompareAction::Start);
    assert!(matches!(state.compare.run, RunView::Running { run: 2, .. }));
    let late_done = Summary {
        different: 9,
        ..Summary::default()
    };
    let (state, cmds) = update(state, pair_done(1, late_done));
    assert!(cmds.is_empty() && state.compare.results.is_none());
    let (state, _) = update(state, rows_event(Category::Different, 0, 3));
    assert!(state.compare.results.is_none());
    let (state, _) = update(state, run_done());
    assert!(matches!(
        state.compare.run,
        RunView::Running {
            run: 2,
            finished_pairs: 0,
            ..
        }
    ));
}

#[test]
fn progress_after_run_done_is_ignored() {
    let (state, _) = update(running_state(), run_done());
    let progress = ev(CompareEvent::Progress {
        session: 1,
        run: 1,
        pair: 1,
        scanned_source: 5,
        scanned_target: 5,
        differences: 1,
    });
    let (state, _) = update(state, progress);
    assert!(matches!(state.compare.run, RunView::Finished { .. }));
}

#[test]
fn events_of_another_session_are_ignored() {
    let late = ev(CompareEvent::RunDone { session: 5, run: 1 });
    let (after, _) = update(running_state(), late);
    assert!(matches!(after.compare.run, RunView::Running { .. }));
}

#[test]
fn progress_updates_the_running_counters_for_the_current_run_only() {
    let progress = |run| {
        ev(CompareEvent::Progress {
            session: 1,
            run,
            pair: 1,
            scanned_source: 10,
            scanned_target: 20,
            differences: 3,
        })
    };
    let (state, _) = update(running_state(), progress(0));
    assert!(matches!(
        state.compare.run,
        RunView::Running {
            scanned_source: 0,
            ..
        }
    ));
    let (state, _) = update(state, progress(1));
    assert!(matches!(
        state.compare.run,
        RunView::Running {
            scanned_source: 10,
            scanned_target: 20,
            differences: 3,
            ..
        }
    ));
}

#[test]
fn cancel_sends_cancel_for_the_current_run_only_while_running() {
    let (_, cmds) = act(running_state(), CompareAction::Cancel);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Cancel {
            session: 1,
            run: 1
        })]
    ));
    let (_, cmds) = act(checked_state(), CompareAction::Cancel);
    assert!(cmds.is_empty());
}

#[test]
fn a_cancelled_pair_ends_the_run_as_cancelled_and_keeps_partial_results() {
    let cancelled = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: 1,
        outcome: PairOutcome::Cancelled(Summary {
            different: 2,
            ..Summary::default()
        }),
    });
    let (state, _) = update(running_state(), cancelled);
    let (state, _) = update(state, run_done());
    assert!(matches!(
        state.compare.run,
        RunView::Finished {
            cancelled: true,
            ..
        }
    ));
    assert!(state.compare.results.is_some());
}

#[test]
fn a_failed_pair_shows_a_banner_and_no_results() {
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: 1,
        outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
    });
    let (state, _) = update(running_state(), failed);
    let (state, _) = update(state, run_done());
    assert!(state.compare.results.is_none());
    assert!(matches!(
        state.compare.run,
        RunView::Finished {
            failure: Some(CoreError::CompareDuplicateKey),
            ..
        }
    ));
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_start",
            ..
        })
    ));
}

#[test]
fn a_refused_start_returns_the_run_to_idle() {
    let refused = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_start",
        error: CoreError::CompareSetupInvalid,
    });
    let (state, _) = update(running_state(), refused);
    assert_eq!(state.compare.run, RunView::Idle);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_start",
            ..
        })
    ));
}

#[test]
fn restarting_discards_the_previous_results_first_and_uses_a_new_run_id() {
    let (state, cmds) = act(finished_with_results(), CompareAction::Start);
    assert!(matches!(
        &cmds[..],
        [
            Command::Compare(CompareCommand::Discard {
                session: 1,
                pair: 1
            }),
            Command::Compare(CompareCommand::Start {
                session: 1,
                run: 2,
                ..
            })
        ]
    ));
    assert!(state.compare.results.is_none());
}

#[test]
fn discard_sends_discard_and_clears_results_and_run() {
    let (state, cmds) = act(finished_with_results(), CompareAction::Discard);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard {
            session: 1,
            pair: 1
        })]
    ));
    assert!(state.compare.results.is_none());
    assert_eq!(state.compare.run, RunView::Idle);
}
