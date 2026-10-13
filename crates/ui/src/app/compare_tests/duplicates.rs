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
pub(super) fn duplicate_failure() -> AppState {
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: RUN_PAIR,
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

fn loading_view(stale: bool) -> DuplicatesView {
    DuplicatesView::Loading {
        entry: ENTRY,
        pair: DUP_PAIR,
        stale,
    }
}

fn loading() -> AppState {
    act(duplicate_failure(), CompareAction::FindDuplicates(ENTRY)).0
}

fn loaded() -> AppState {
    update(loading(), answer(1, DUP_PAIR)).0
}

#[test]
fn find_duplicates_sends_the_command_for_the_current_pair() {
    let state = duplicate_failure();
    let expected = state.compare.spec_for(ENTRY, DUP_PAIR).unwrap().unwrap();
    let (state, cmds) = act(state, CompareAction::FindDuplicates(ENTRY));
    match &cmds[..] {
        [Command::Compare(CompareCommand::DuplicateKeys { session: 1, pair })] => {
            assert_eq!(pair.id, DUP_PAIR, "a fresh id, not the failed run's");
            assert_eq!(pair.key, expected.key);
            assert_eq!(pair.source, expected.source);
            assert_eq!(pair.target, expected.target);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(state.compare.duplicates, loading_view(false));
    assert_eq!(state.compare.next_pair, DUP_PAIR + 1, "the id is spent");
}

#[test]
fn it_is_refused_unless_the_run_failed_on_a_duplicate_key() {
    let other_failure = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: RUN_PAIR,
        outcome: PairOutcome::Failed(CoreError::CompareKeyMissing),
    });
    let (failed, _) = update(running_state(), other_failure);
    let (failed, _) = update(failed, run_done());
    for state in [checked_state(), finished_with_results(), failed] {
        let (after, cmds) = act(state, CompareAction::FindDuplicates(ENTRY));
        assert!(cmds.is_empty());
        assert_eq!(after.compare.duplicates, DuplicatesView::Idle);
    }
}

#[test]
fn a_second_request_while_loading_sends_nothing() {
    let (after, cmds) = act(loading(), CompareAction::FindDuplicates(ENTRY));
    assert!(cmds.is_empty());
    assert_eq!(after.compare.duplicates, loading_view(false));
}

#[test]
fn the_answer_populates_the_view() {
    let (state, cmds) = update(loading(), answer(1, DUP_PAIR));
    assert!(cmds.is_empty());
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loaded {
            entry: ENTRY,
            source: report("s", 3),
            target: report("t", 2)
        }
    );
    assert!(!duplicates_banner(&state));
}

#[test]
fn an_edit_while_loading_marks_the_answer_stale_and_it_is_dropped() {
    let (state, _) = act(loading(), CompareAction::SetKey("sku".into()));
    assert_eq!(state.compare.duplicates, loading_view(true));
    assert!(!state.compare.can_find_duplicates());
    let (state, cmds) = update(state, answer(1, DUP_PAIR));
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
    for input in [answer(9, DUP_PAIR), answer(1, 7), answer(1, RUN_PAIR)] {
        let (state, cmds) = update(loading(), input);
        assert!(cmds.is_empty());
        assert_eq!(state.compare.duplicates, loading_view(false));
    }
}

#[test]
fn an_answer_with_nothing_requested_is_ignored() {
    let (state, _) = update(duplicate_failure(), answer(1, DUP_PAIR));
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
    let before = loaded().compare.duplicates;
    let different = ev(CompareEvent::DuplicateKeys {
        session: 1,
        pair: DUP_PAIR,
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
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Failed { entry: ENTRY }
    );
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
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Failed { entry: ENTRY }
    );
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
    let (discarded, _) = act(loaded(), CompareAction::DiscardPair(ENTRY));
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
    let (after, _) = update(closed, answer(1, DUP_PAIR));
    assert_eq!(after.compare.duplicates, DuplicatesView::Idle);
    let (dropped, _) = update(loading(), open_failed());
    let (after, _) = update(dropped, answer(1, DUP_PAIR));
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
        let (state, _) = update(state, answer(1, DUP_PAIR));
        assert_eq!(state.compare.duplicates, DuplicatesView::Idle);
    }
}

#[test]
fn discard_while_loading_marks_the_answer_stale() {
    let (state, _) = act(loading(), CompareAction::DiscardPair(ENTRY));
    assert_eq!(state.compare.duplicates, loading_view(true));
}

#[test]
fn start_while_loading_then_a_new_duplicate_failure_still_drops_the_old_answer() {
    let (state, _) = act(loading(), CompareAction::Start);
    assert_eq!(state.compare.duplicates, loading_view(true));
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 2,
        pair: DUP_PAIR + 1,
        outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
    });
    let (state, _) = update(state, failed);
    let (state, _) = update(state, ev(CompareEvent::RunDone { session: 1, run: 2 }));
    assert!(state.compare.failed_on_duplicate_key(ENTRY));
    assert!(!state.compare.can_find_duplicates());
    let (state, cmds) = act(state, CompareAction::FindDuplicates(ENTRY));
    assert!(cmds.is_empty(), "nothing is sent while the old one is out");
    let (state, _) = update(state, answer(1, DUP_PAIR));
    assert_eq!(state.compare.duplicates, DuplicatesView::Idle, "not Loaded");
    let (state, cmds) = act(state, CompareAction::FindDuplicates(ENTRY));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::DuplicateKeys {
            session: 1,
            ..
        })]
    ));
    assert_eq!(
        state.compare.duplicates,
        DuplicatesView::Loading {
            entry: ENTRY,
            pair: 5,
            stale: false
        },
        "a fresh id again: the first lookup's 3 is never reused"
    );
}

#[test]
fn find_is_offered_only_while_the_setup_is_unchanged_since_the_failed_run() {
    let (edited, _) = act(duplicate_failure(), CompareAction::SetKey("sku".into()));
    assert!(!edited.compare.can_find_duplicates());
    let (edited, cmds) = act(edited, CompareAction::FindDuplicates(ENTRY));
    assert!(cmds.is_empty());
    assert_eq!(edited.compare.duplicates, DuplicatesView::Idle);
    let (rechecked, _) = act(edited, CompareAction::RunCheck);
    let (rechecked, _) = update(rechecked, checked_as(3, indexed(10)));
    assert!(rechecked.compare.can_find_duplicates());
}

// ---- several entries ----

mod entries {
    use super::super::pairs::multi_state;
    use super::*;

    fn failure(run: u64, pair: u32, error: CoreError) -> Input {
        ev(CompareEvent::PairDone {
            session: 1,
            run,
            pair,
            outcome: PairOutcome::Failed(error),
        })
    }

    /// Three entries (alpha, beta, gamma) run: alpha fails on a missing key, beta and gamma on
    /// a duplicate key. Check probes are 1-3, the run's pairs 4-6, the next pair id 7.
    fn mixed_failures() -> AppState {
        let (state, _) = act(
            multi_state(&["alpha", "beta", "gamma"]),
            CompareAction::RunCheck,
        );
        let state = (1..=3).fold(state, |s, probe| update(s, checked_as(probe, indexed(5))).0);
        let (state, _) = act(state, CompareAction::Start);
        let events = [
            failure(1, 4, CoreError::CompareKeyMissing),
            failure(1, 5, CoreError::CompareDuplicateKey),
            failure(1, 6, CoreError::CompareDuplicateKey),
            ev(CompareEvent::RunDone { session: 1, run: 1 }),
        ];
        events.into_iter().fold(state, |s, e| update(s, e).0)
    }

    #[test]
    fn find_duplicates_is_offered_only_for_a_duplicate_key_failure_of_that_entry() {
        let state = mixed_failures();
        assert!(!state.compare.can_find_duplicates_for(0), "another failure");
        assert!(state.compare.can_find_duplicates_for(1));
        assert!(state.compare.can_find_duplicates_for(2));
        let (same, cmds) = act(copy_of(&state), CompareAction::FindDuplicates(0));
        assert!(cmds.is_empty() && same.compare.duplicates == DuplicatesView::Idle);
        let (unknown, cmds) = act(copy_of(&state), CompareAction::FindDuplicates(42));
        assert!(cmds.is_empty() && unknown.compare.duplicates == DuplicatesView::Idle);
        let (state, cmds) = act(state, CompareAction::FindDuplicates(1));
        match &cmds[..] {
            [Command::Compare(CompareCommand::DuplicateKeys { session: 1, pair })] => {
                assert_eq!(pair.source.collection, "beta");
                assert_eq!(pair.id, 7, "a fresh id, never the failed pair's 5");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(state.compare.duplicates.entry(), Some(1));
    }

    #[test]
    fn duplicates_answer_for_another_entry_is_ignored() {
        let (state, _) = act(mixed_failures(), CompareAction::FindDuplicates(1));
        // The answers that belong to neither this lookup nor this entry change nothing.
        for pair in [5, 6, 8] {
            let (after, cmds) = update(copy_of(&state), answer(1, pair));
            assert!(cmds.is_empty());
            assert!(matches!(
                after.compare.duplicates,
                DuplicatesView::Loading {
                    entry: 1,
                    pair: 7,
                    stale: false
                }
            ));
        }
        let (state, _) = update(state, answer(1, 7));
        assert!(matches!(
            state.compare.duplicates,
            DuplicatesView::Loaded { entry: 1, .. }
        ));
    }

    #[test]
    fn a_lookup_for_another_entry_replaces_the_panel() {
        let (state, _) = act(mixed_failures(), CompareAction::FindDuplicates(1));
        let (state, _) = update(state, answer(1, 7));
        let (state, cmds) = act(state, CompareAction::FindDuplicates(2));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            state.compare.duplicates,
            DuplicatesView::Loading {
                entry: 2,
                pair: 8,
                ..
            }
        ));
    }

    #[test]
    fn find_duplicates_never_reuses_a_pair_id_even_when_asked_again() {
        let (state, _) = act(mixed_failures(), CompareAction::FindDuplicates(1));
        let (state, _) = update(state, failed_event("compare_duplicates"));
        assert_eq!(
            state.compare.duplicates,
            DuplicatesView::Failed { entry: 1 }
        );
        let (_, cmds) = act(state, CompareAction::FindDuplicates(1));
        assert!(matches!(
            &cmds[..],
            [Command::Compare(CompareCommand::DuplicateKeys { pair, .. })] if pair.id == 8
        ));
    }
}
