//! Starting, running, cancelling and finishing a run.
use super::*;

#[test]
fn one_terminal_event_per_pair_and_the_run_terminal_update_the_run_view() {
    let state = running_state();
    let (state, _) = update(state, pair_done(1, Summary::default()));
    assert!(matches!(
        state.compare.run_view(),
        RunView::Running {
            finished_pairs: 1,
            ..
        }
    ));
    let (state, _) = update(state, run_done());
    assert!(matches!(state.compare.run_view(), RunView::Finished { .. }));
}

#[test]
fn a_late_pair_done_for_an_older_run_is_ignored() {
    let (after, cmds) = update(running_state(), pair_done(0, Summary::default()));
    assert!(matches!(
        after.compare.run_view(),
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
    assert!(matches!(
        state.compare.run_view(),
        RunView::Running { run: 2, .. }
    ));
    let late_done = Summary {
        different: 9,
        ..Summary::default()
    };
    let (state, cmds) = update(state, pair_done(1, late_done));
    assert!(cmds.is_empty() && state.compare.results.is_empty());
    let (state, _) = update(state, rows_event(Category::Different, 0, 3));
    assert!(state.compare.results.is_empty());
    let (state, _) = update(state, run_done());
    assert!(matches!(
        state.compare.run_view(),
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
        pair: RUN_PAIR,
        scanned_source: 5,
        scanned_target: 5,
        differences: 1,
    });
    let (state, _) = update(state, progress);
    assert!(matches!(state.compare.run_view(), RunView::Finished { .. }));
}

#[test]
fn events_of_another_session_are_ignored() {
    let late = ev(CompareEvent::RunDone { session: 5, run: 1 });
    let (after, _) = update(running_state(), late);
    assert!(matches!(after.compare.run_view(), RunView::Running { .. }));
}

#[test]
fn progress_updates_the_running_counters_for_the_current_run_only() {
    let progress = |run| {
        ev(CompareEvent::Progress {
            session: 1,
            run,
            pair: RUN_PAIR,
            scanned_source: 10,
            scanned_target: 20,
            differences: 3,
        })
    };
    let (state, _) = update(running_state(), progress(0));
    assert!(matches!(
        state.compare.run_view(),
        RunView::Running {
            scanned_source: 0,
            ..
        }
    ));
    let (state, _) = update(state, progress(1));
    assert!(matches!(
        state.compare.run_view(),
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
fn a_cancelled_pair_ends_the_run_as_cancelled_with_no_results_view_and_no_rows_request() {
    let cancelled = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: RUN_PAIR,
        outcome: PairOutcome::Cancelled(Summary {
            different: 2,
            scanned_source: 7,
            scanned_target: 6,
            ..Summary::default()
        }),
    });
    let (state, cmds) = update(running_state(), cancelled);
    assert!(
        cmds.is_empty(),
        "the runtime keeps nothing for a cancelled pair"
    );
    assert!(state.banner.is_none());
    let (state, _) = update(state, run_done());
    assert!(matches!(
        state.compare.run_view(),
        RunView::Finished {
            cancelled: true,
            scanned_source: 7,
            scanned_target: 6,
            ..
        }
    ));
    assert!(state.compare.results.is_empty());
    assert!(state.compare.can_discard(), "Discard stays available");
}

#[test]
fn a_failed_pair_shows_a_banner_and_no_results() {
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: RUN_PAIR,
        outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
    });
    let (state, _) = update(running_state(), failed);
    let (state, _) = update(state, run_done());
    assert!(state.compare.results.is_empty());
    assert!(matches!(
        state.compare.run_view(),
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
    assert_eq!(state.compare.run_view(), RunView::Idle);
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
                pair: RUN_PAIR
            }),
            Command::Compare(CompareCommand::Start {
                session: 1,
                run: 2,
                ..
            })
        ]
    ));
    assert!(state.compare.results.is_empty());
}

#[test]
fn discard_sends_discard_and_clears_results_and_run() {
    let (state, cmds) = act(finished_with_results(), CompareAction::Discard);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard {
            session: 1,
            pair: RUN_PAIR
        })]
    ));
    assert!(state.compare.results.is_empty());
    assert_eq!(state.compare.run_view(), RunView::Idle);
}

fn cancelled_done(run: u64) -> Input {
    ev(CompareEvent::PairDone {
        session: 1,
        run,
        pair: RUN_PAIR,
        outcome: PairOutcome::Cancelled(Summary::default()),
    })
}

#[test]
fn cancel_is_marked_at_once_and_sent_once() {
    let (state, cmds) = act(running_state(), CompareAction::Cancel);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Cancel {
            session: 1,
            run: 1
        })]
    ));
    assert!(state.compare.cancel_requested() && !state.compare.can_cancel());
    let (state, cmds) = act(state, CompareAction::Cancel);
    assert!(cmds.is_empty(), "a second click sends nothing");
    let (state, _) = update(state, cancelled_done(1));
    assert!(
        state.compare.cancel_requested() && !state.compare.can_cancel(),
        "stays marked until the run terminal, so no second Cancel can be sent"
    );
    let (state, _) = update(state, run_done());
    assert!(!state.compare.cancel_requested());
}

#[test]
fn the_cancel_mark_is_cleared_by_the_run_terminal_and_by_a_new_run() {
    let (state, _) = act(running_state(), CompareAction::Cancel);
    let (state, _) = update(state, cancelled_done(1));
    let (state, _) = update(state, run_done());
    assert!(!state.compare.cancel_requested());
    assert!(state.compare.can_start(), "the verdict outlives a run");
    let (state, _) = act(state, CompareAction::Start);
    assert!(state.compare.is_running() && !state.compare.cancel_requested());
}

#[test]
fn a_finished_run_keeps_the_scanned_counts_of_its_pair() {
    let summary = Summary {
        equal: 9,
        scanned_source: 12,
        scanned_target: 9,
        only_in_source: 3,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, summary));
    let (state, _) = update(state, run_done());
    assert!(matches!(
        state.compare.run_view(),
        RunView::Finished {
            scanned_source: 12,
            scanned_target: 9,
            ..
        }
    ));
}
