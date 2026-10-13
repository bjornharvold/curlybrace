//! Find duplicate keys: the request, its single-flight rule and its answers.
use super::*;
use curlybrace_core::compare::protocol::DuplicateReport;
use curlybrace_core::store::DuplicateGroup;

use crate::viewmodel::compare::DuplicatesView;

fn report(value: &str, count: u64) -> DuplicateReport {
    DuplicateReport {
        groups: vec![DuplicateGroup {
            key: KeyTuple(vec![Bson::String(value.into())]),
            count,
        }],
        more: false,
    }
}

fn answer(session: u64, pair: u32) -> Input {
    ev(CompareEvent::DuplicateKeys {
        session,
        pair,
        source: report("s", 3),
        target: report("t", 2),
    })
}

fn failed_event(context: &'static str) -> Input {
    ev(CompareEvent::Failed {
        session: 1,
        context,
        error: CoreError::Database("boom".into()),
    })
}

/// A run that finished on a duplicate key.
fn duplicate_failure() -> AppState {
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: 1,
        outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
    });
    let (state, _) = update(running_state(), failed);
    update(state, run_done()).0
}

/// The earlier failed run leaves its own banner; only a duplicates banner matters here.
fn duplicates_banner(state: &AppState) -> bool {
    matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_duplicates",
            ..
        })
    )
}

fn loading() -> AppState {
    act(duplicate_failure(), CompareAction::FindDuplicates).0
}

fn loaded() -> AppState {
    update(loading(), answer(1, 1)).0
}

#[test]
fn find_duplicates_sends_the_command_for_the_current_pair() {
    let state = duplicate_failure();
    let expected = state.compare.pair().unwrap();
    let (state, cmds) = act(state, CompareAction::FindDuplicates);
    match &cmds[..] {
        [Command::Compare(CompareCommand::DuplicateKeys { session: 1, pair })] => {
            assert_eq!(pair.id, 1);
            assert_eq!(pair.key, expected.key);
            assert_eq!(pair.source, expected.source);
            assert_eq!(pair.target, expected.target);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loading { stale: false }
    );
}

#[test]
fn it_is_refused_unless_the_run_failed_on_a_duplicate_key() {
    let other_failure = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: 1,
        outcome: PairOutcome::Failed(CoreError::CompareKeyMissing),
    });
    let (failed, _) = update(running_state(), other_failure);
    let (failed, _) = update(failed, run_done());
    for state in [checked_state(), finished_with_results(), failed] {
        let (after, cmds) = act(state, CompareAction::FindDuplicates);
        assert!(cmds.is_empty());
        assert_eq!(after.compare.duplicates, DuplicatesView::Idle);
    }
}

#[test]
fn a_second_request_while_loading_sends_nothing() {
    let (after, cmds) = act(loading(), CompareAction::FindDuplicates);
    assert!(cmds.is_empty());
    assert_eq!(
        after.compare.duplicates,
        DuplicatesView::Loading { stale: false }
    );
}

#[test]
fn the_answer_populates_the_view() {
    let (state, cmds) = update(loading(), answer(1, 1));
    assert!(cmds.is_empty());
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loaded {
            source: report("s", 3),
            target: report("t", 2)
        }
    );
    assert!(!duplicates_banner(&state));
}

#[test]
fn an_edit_while_loading_marks_the_answer_stale_and_it_is_dropped() {
    let (state, _) = act(loading(), CompareAction::SetKey("sku".into()));
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loading { stale: true }
    );
    assert!(!state.compare.can_find_duplicates());
    let (state, cmds) = update(state, answer(1, 1));
    assert!(cmds.is_empty());
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
    assert!(
        !state.compare.can_find_duplicates(),
        "the edit needs a fresh check first"
    );
}

#[test]
fn a_stale_failure_resets_the_view_without_a_banner() {
    let (state, _) = act(loading(), CompareAction::SetKey("sku".into()));
    let (state, _) = update(state, failed_event("compare_duplicates"));
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
    assert!(!duplicates_banner(&state));
}

#[test]
fn a_loaded_panel_is_cleared_by_an_edit() {
    let (state, _) = act(loaded(), CompareAction::SetKey("sku".into()));
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
}

#[test]
fn answers_for_another_session_or_pair_are_ignored() {
    for input in [answer(9, 1), answer(1, 2)] {
        let (state, cmds) = update(loading(), input);
        assert!(cmds.is_empty());
        assert_eq!(
            state.compare.duplicates,
            DuplicatesView::Loading { stale: false }
        );
    }
}

#[test]
fn an_answer_with_nothing_requested_is_ignored() {
    let (state, _) = update(duplicate_failure(), answer(1, 1));
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
    let before = loaded().compare.duplicates;
    let different = ev(CompareEvent::DuplicateKeys {
        session: 1,
        pair: 1,
        source: report("other-s", 9),
        target: report("other-t", 8),
    });
    let (after, cmds) = update(loaded(), different);
    assert!(cmds.is_empty());
    assert_eq!(after.compare.duplicates, before);
}

#[test]
fn a_timed_out_lookup_gets_its_own_banner_context() {
    let timeout = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_duplicates",
        error: CoreError::Database("timeout".into()),
    });
    let (state, _) = update(loading(), timeout);
    assert_eq!(state.compare.duplicates, DuplicatesView::Failed);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_duplicates_timeout",
            ..
        })
    ));
    assert!(state.compare.can_find_duplicates(), "Try again is offered");
}

#[test]
fn a_failed_request_clears_loading_and_shows_the_banner() {
    let (state, _) = update(loading(), failed_event("compare_duplicates"));
    assert_eq!(state.compare.duplicates, DuplicatesView::Failed);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_duplicates",
            ..
        })
    ));
    assert!(state.compare.can_find_duplicates(), "Try again is offered");
}

/// Every way a panel can be reset, from both a loaded and an in-flight panel.
#[test]
fn restart_discard_close_and_session_drop_reset_the_panel() {
    let (started, _) = act(loaded(), CompareAction::Start);
    assert_eq!(started.compare.duplicates, DuplicatesView::Idle);
    let (discarded, _) = act(loaded(), CompareAction::Discard);
    assert_eq!(discarded.compare.duplicates, DuplicatesView::Idle);
    for start in [loaded(), loading()] {
        let (closed, _) = act(start, CompareAction::CloseSession);
        assert_eq!(closed.compare.duplicates, DuplicatesView::Idle);
    }
    for start in [loaded(), loading()] {
        let (dropped, _) = update(start, open_failed());
        assert_eq!(dropped.compare.duplicates, DuplicatesView::Idle);
    }
}

fn open_failed() -> Input {
    ev(CompareEvent::OpenFailed {
        session: 1,
        error: CoreError::NotFound,
    })
}

#[test]
fn a_late_answer_for_a_dropped_session_is_ignored() {
    let (closed, _) = act(loading(), CompareAction::CloseSession);
    let (after, _) = update(closed, answer(1, 1));
    assert_eq!(after.compare.duplicates, DuplicatesView::Idle);
    let (dropped, _) = update(loading(), open_failed());
    let (after, _) = update(dropped, answer(1, 1));
    assert_eq!(after.compare.duplicates, DuplicatesView::Idle);
}

fn with_other_connection(state: AppState) -> (AppState, uuid::Uuid) {
    let other = ConnectionConfig::new("c", "mongodb://c");
    let id = other.id;
    let connections = [state.conn.connections.clone(), vec![other]].concat();
    let conn = crate::views::connections::ConnectionsState {
        connections,
        ..state.conn
    };
    (AppState { conn, ..state }, id)
}

#[test]
fn opening_another_pair_resets_the_panel() {
    for start in [loaded(), loading()] {
        let (source, _) = start.compare.connections.unwrap();
        let (state, other) = with_other_connection(start);
        let (state, _) = act(state, open_action(source, other));
        assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
        let (state, _) = update(state, answer(1, 1));
        assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
    }
}

#[test]
fn discard_while_loading_marks_the_answer_stale() {
    let (state, _) = act(loading(), CompareAction::Discard);
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loading { stale: true }
    );
}

#[test]
fn start_while_loading_then_a_new_duplicate_failure_still_drops_the_old_answer() {
    let (state, _) = act(loading(), CompareAction::Start);
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loading { stale: true }
    );
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 2,
        pair: 1,
        outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
    });
    let (state, _) = update(state, failed);
    let (state, _) = update(state, ev(CompareEvent::RunDone { session: 1, run: 2 }));
    assert!(state.compare.run_failed_on_duplicate_key());
    assert!(!state.compare.can_find_duplicates());
    let (state, cmds) = act(state, CompareAction::FindDuplicates);
    assert!(cmds.is_empty(), "nothing is sent while the old one is out");
    let (state, _) = update(state, answer(1, 1));
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle, "not Loaded");
    let (state, cmds) = act(state, CompareAction::FindDuplicates);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::DuplicateKeys {
            session: 1,
            ..
        })]
    ));
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loading { stale: false }
    );
}

#[test]
fn find_is_offered_only_while_the_setup_is_unchanged_since_the_failed_run() {
    let (edited, _) = act(duplicate_failure(), CompareAction::SetKey("sku".into()));
    assert!(!edited.compare.can_find_duplicates());
    let (edited, cmds) = act(edited, CompareAction::FindDuplicates);
    assert!(cmds.is_empty());
    assert_eq!(edited.compare.duplicates, DuplicatesView::Idle);
    let (rechecked, _) = act(edited, CompareAction::RunCheck);
    let (rechecked, _) = update(rechecked, checked_event(indexed(10)));
    assert!(rechecked.compare.can_find_duplicates());
}
