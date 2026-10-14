//! Session lifecycle, the setup form and the preflight check.
use super::*;

#[test]
fn open_sends_open_with_a_fresh_session_id_and_shows_the_workspace() {
    let (state, a, b) = two_connections();
    let (state, cmds) = act(state, open_action(a, b));
    assert_eq!(state.workspace, Workspace::Compare);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Open { session: 1, .. })]
    ));
    assert!(matches!(
        state.compare.session,
        Some(SessionPhase::Opening(1))
    ));
}

#[test]
fn opening_an_unknown_connection_does_nothing() {
    let (state, a, _) = two_connections();
    let (state, cmds) = act(state, open_action(a, uuid::Uuid::new_v4()));
    assert!(cmds.is_empty() && state.compare.session.is_none());
}

#[test]
fn opening_the_same_pair_twice_ignores_the_second_request() {
    let (state, a, b) = two_connections();
    let (state, _) = act(state, open_action(a, b));
    let (_, cmds) = act(state, open_action(a, b));
    assert!(cmds.is_empty());
}

#[test]
fn opening_a_different_pair_closes_the_old_session_and_never_reuses_its_id() {
    let (state, a, b) = two_connections();
    let (state, _) = act(state, open_action(a, b));
    let (state, cmds) = act(state, open_action(b, a));
    assert!(matches!(
        &cmds[..],
        [
            Command::Compare(CompareCommand::Close { session: 1 }),
            Command::Compare(CompareCommand::Open { session: 2, .. })
        ]
    ));
    assert!(matches!(
        state.compare.session,
        Some(SessionPhase::Opening(2))
    ));
}

#[test]
fn opened_moves_to_ready_and_lists_databases_for_both_sides() {
    let (_, _, _) = opened();
}

#[test]
fn opened_for_another_session_is_ignored() {
    let (state, a, b) = two_connections();
    let (state, _) = act(state, open_action(a, b));
    let (state, cmds) = update(state, ev(CompareEvent::Opened { session: 9 }));
    assert!(cmds.is_empty());
    assert!(matches!(
        state.compare.session,
        Some(SessionPhase::Opening(1))
    ));
}

#[test]
fn open_failed_clears_the_session_and_shows_a_banner() {
    let (state, a, b) = two_connections();
    let (state, _) = act(state, open_action(a, b));
    let e = CompareEvent::OpenFailed {
        session: 1,
        error: CoreError::Database("x".into()),
    };
    let (state, cmds) = update(state, ev(e));
    assert!(state.compare.session.is_none());
    assert!(cmds.is_empty());
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_open",
            ..
        })
    ));
}

#[test]
fn leaving_the_workspace_keeps_the_session_and_closing_it_sends_close() {
    let (state, _, _) = opened();
    let (state, cmds) = act(state, CompareAction::Hide);
    assert_eq!(state.workspace, Workspace::Browse);
    assert!(cmds.is_empty() && state.compare.session.is_some());
    let (state, _) = act(state, CompareAction::Show);
    assert_eq!(state.workspace, Workspace::Compare);
    let (state, cmds) = act(state, CompareAction::CloseSession);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Close { session: 1 })]
    ));
    assert!(state.compare.session.is_none());
}

#[test]
fn ids_are_never_reused_after_closing() {
    let (state, a, b) = opened();
    let (state, _) = act(state, CompareAction::CloseSession);
    let (_, cmds) = act(state, open_action(a, b));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Open { session: 2, .. })]
    ));
}

#[test]
fn events_after_close_are_ignored() {
    let (state, _, _) = opened();
    let (state, _) = act(state, CompareAction::CloseSession);
    let (state, cmds) = update(state, ev(CompareEvent::Opened { session: 1 }));
    assert!(state.compare.session.is_none() && cmds.is_empty());
}

#[test]
fn disconnecting_the_main_connection_does_not_close_the_compare_session() {
    let (state, a, _) = opened();
    let (state, _) = update(state, Input::Connect(a));
    let (state, cmds) = update(state, Input::Disconnect(a));
    assert!(state.open.is_empty(), "the browsing session really closed");
    assert!(
        state.compare.session.is_some(),
        "the compare session has its own connections"
    );
    assert!(!cmds.iter().any(|c| matches!(c, Command::Compare(_))));
}

#[test]
fn a_browsing_connect_and_a_cleared_session_leave_compare_untouched() {
    let (state, a, _) = opened();
    let (state, _) = act(state, CompareAction::SetKey("sku".into()));
    let before = format!("{:?}", state.compare);
    let (state, cmds) = update(state, Input::Connect(a));
    assert_eq!(format!("{:?}", state.compare), before);
    assert!(cmds.iter().all(|c| !matches!(c, Command::Compare(_))));
    let cleared = state.session_cleared();
    assert_eq!(format!("{:?}", cleared.compare), before);
}

#[test]
fn compare_events_are_handled_even_when_the_main_connection_is_not_connected() {
    let (state, _, _) = opened();
    assert_ne!(state.status, ConnStatus::Connected);
    let names = CompareEvent::Databases {
        session: 1,
        side: Side::Source,
        names: vec!["x".into()],
    };
    let (state, _) = update(state, ev(names));
    assert_eq!(state.compare.setup.source.databases, vec!["x".to_string()]);
}

#[test]
fn failures_of_other_sessions_are_ignored_and_current_ones_become_banners() {
    let (state, _, _) = opened();
    let other = ev(CompareEvent::Failed {
        session: 4,
        context: "compare_databases",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, other);
    assert!(state.banner.is_none());
    let mine = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_databases",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, mine);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_databases",
            ..
        })
    ));
}

#[test]
fn picking_a_view_keeps_the_form_not_ready_and_adds_no_pair() {
    let (state, cmds) = act(
        picked_form_state(),
        CompareAction::PickSide(Side::Target, Pick::Collection("v".into())),
    );
    assert!(cmds.is_empty());
    assert!(!state.compare.setup.ready());
    let (state, cmds) = act(state, CompareAction::AddPair);
    assert!(cmds.is_empty() && state.compare.pairs.entries.is_empty());
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty());
}

#[test]
fn repicking_the_same_database_or_collection_changes_nothing() {
    for pick in [Pick::Database("a".into()), Pick::Collection("users".into())] {
        let (after, cmds) = act(checked_state(), CompareAction::PickSide(Side::Source, pick));
        assert!(cmds.is_empty(), "no listing is requested again");
        assert_eq!(after.compare.preflight_view(), PreflightView::Ok);
    }
}

#[test]
fn start_is_refused_until_the_form_is_ready_and_the_preflight_allows_it() {
    let (state, _, _) = opened();
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty(), "nothing is picked yet");
    assert_eq!(state.compare.run_view(), RunView::Idle);
}

#[test]
fn start_needs_a_check_and_editing_the_form_invalidates_it() {
    let (_, cmds) = act(picked_state(), CompareAction::Start);
    assert!(cmds.is_empty(), "not checked yet");
    let (state, _) = act(checked_state(), CompareAction::SetKey("sku".into()));
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
    let (_, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty());
}

#[test]
fn an_unindexed_large_collection_blocks_start() {
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let blocked = Preflight {
        target_index: IndexStatus::NotIndexed,
        unindexed_too_large: true,
        ..indexed(5_000_000)
    };
    let (state, _) = update(state, checked_event(blocked));
    assert_eq!(
        state.compare.preflight_view(),
        PreflightView::NeedsIndex { side: Side::Target }
    );
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty());
    assert_eq!(state.compare.run_view(), RunView::Idle);
}

#[test]
fn a_bad_key_blocks_the_check() {
    let (state, _) = act(picked_state(), CompareAction::SetKey("a,,b".into()));
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty());
}

#[test]
fn only_one_check_is_in_flight_and_a_stale_answer_is_dropped() {
    let (state, cmds) = act(picked_state(), CompareAction::RunCheck);
    assert_eq!(cmds.len(), 1);
    let (state, _) = act(state, CompareAction::SetFilter(Side::Source, "{}".into()));
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty(), "no second check while one is in flight");

    let (state, _) = update(state, checked_event(indexed(1)));
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
    assert!(!state.compare.check_in_flight());
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty(), "the dropped answer does not unlock Start");

    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(cmds.len(), 1, "the user can check again");
    let (state, _) = update(state, checked_as(2, indexed(1)));
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}

#[test]
fn start_is_refused_while_a_check_is_in_flight() {
    let (state, _) = act(checked_state(), CompareAction::RunCheck);
    let (_, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty());
}

#[test]
fn a_checked_answer_with_nothing_in_flight_is_dropped() {
    let (state, _) = update(picked_state(), checked_event(indexed(1)));
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
}

fn check_failed_event() -> Input {
    ev(CompareEvent::Failed {
        session: 1,
        context: "compare_check",
        error: CoreError::NotFound,
    })
}

#[test]
fn a_failed_check_returns_the_preflight_to_unchecked_with_a_banner() {
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    assert_eq!(state.compare.preflight_view(), PreflightView::Checking);
    let (state, _) = update(state, check_failed_event());
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
    assert!(!state.compare.check_in_flight());
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_check",
            ..
        })
    ));
}

#[test]
fn a_stale_check_failure_is_quiet_and_never_resets_a_newer_state() {
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let (state, _) = act(state, CompareAction::SetKey("sku".into()));
    let (state, _) = update(state, check_failed_event());
    assert!(!state.compare.check_in_flight());
    assert!(state.banner.is_none(), "the failed check was obsolete");

    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(cmds.len(), 1);
    let (state, _) = update(state, checked_as(2, indexed(2)));
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
    let (state, _) = update(state, check_failed_event());
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}

#[test]
fn the_same_collection_on_both_sides_cannot_be_checked_or_started() {
    let state = same_collection_state();
    assert_eq!(
        state.compare.issues(),
        vec![crate::viewmodel::compare::SetupError::SameCollection]
    );
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty() && !state.compare.check_in_flight());
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty() && !state.compare.is_running());
    // A different filter on one side makes it a real comparison.
    let (state, _) = act(
        state,
        CompareAction::SetFilter(Side::Target, "{\"active\": true}".into()),
    );
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Check { session: 1, .. })]
    ));
}

#[test]
fn a_check_failure_with_nothing_in_flight_and_no_orphan_is_reported() {
    let (state, _) = update(picked_state(), check_failed_event());
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_check",
            ..
        })
    ));
}
