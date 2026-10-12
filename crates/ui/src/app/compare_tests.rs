use super::*;
use crate::app::reducer::{Input, update};
use crate::app::state::Banner;
use crate::app::state::{AppState, ConnStatus, Workspace};
use crate::viewmodel::compare::{PreflightView, RunView, SessionPhase};
use bson::Bson;
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, PairOutcome, Preflight, RowView, Side,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{CollectionKind, IndexStatus};

fn two_connections() -> (AppState, uuid::Uuid, uuid::Uuid) {
    let (a, b) = (
        ConnectionConfig::new("a", "mongodb://a"),
        ConnectionConfig::new("b", "mongodb://b"),
    );
    let ids = (a.id, b.id);
    (AppState::new(vec![a, b]), ids.0, ids.1)
}

fn ev(e: CompareEvent) -> Input {
    Input::Event(Event::Compare(e))
}

fn act(state: AppState, a: CompareAction) -> (AppState, Vec<Command>) {
    update(state, Input::Compare(a))
}

fn open_action(source: uuid::Uuid, target: uuid::Uuid) -> CompareAction {
    CompareAction::Open { source, target }
}

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

fn opened() -> (AppState, uuid::Uuid, uuid::Uuid) {
    let (state, a, b) = two_connections();
    let (state, _) = act(state, open_action(a, b));
    let (state, cmds) = update(state, ev(CompareEvent::Opened { session: 1 }));
    assert!(matches!(
        state.compare.session,
        Some(SessionPhase::Ready(1))
    ));
    assert_eq!(cmds.len(), 2, "one database listing per side");
    (state, a, b)
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
fn start_is_refused_until_the_form_is_ready_and_the_preflight_allows_it() {
    let (state, _, _) = opened();
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty(), "nothing is picked yet");
    assert_eq!(state.compare.run, RunView::Idle);
}

fn picked_state() -> AppState {
    let (mut state, _, _) = opened();
    for (side, db) in [(Side::Source, "a"), (Side::Target, "b")] {
        let (s, cmds) = act(
            state,
            CompareAction::PickSide(side, Pick::Database(db.into())),
        );
        assert!(matches!(
            &cmds[..],
            [Command::Compare(CompareCommand::Collections {
                session: 1,
                ..
            })]
        ));
        let listing = CompareEvent::Collections {
            session: 1,
            side,
            db: db.into(),
            entries: vec![
                ("users".into(), CollectionKind::Collection),
                ("v".into(), CollectionKind::View),
            ],
        };
        let (s, _) = update(s, ev(listing));
        let (s, _) = act(
            s,
            CompareAction::PickSide(side, Pick::Collection("users".into())),
        );
        state = s;
    }
    let (state, _) = act(state, CompareAction::SetKey("_id".into()));
    state
}

fn indexed(count: u64) -> Preflight {
    Preflight {
        source_count: count,
        target_count: count,
        source_index: IndexStatus::Indexed,
        target_index: IndexStatus::Indexed,
        unindexed_too_large: false,
    }
}

fn checked_event(preflight: Preflight) -> Input {
    ev(CompareEvent::Checked {
        session: 1,
        pair: 1,
        preflight,
    })
}

fn checked_state() -> AppState {
    let (state, cmds) = act(picked_state(), CompareAction::RunCheck);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Check { session: 1, pair })] if pair.id == 1
    ));
    let (state, cmds) = update(state, checked_event(indexed(10)));
    assert!(cmds.is_empty());
    assert_eq!(state.compare.preflight, PreflightView::Ok);
    state
}

fn running_state() -> AppState {
    let (state, cmds) = act(checked_state(), CompareAction::Start);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Start {
            session: 1,
            run: 1,
            ..
        })]
    ));
    state
}

#[test]
fn picking_a_view_keeps_the_form_not_ready() {
    let (state, cmds) = act(
        picked_state(),
        CompareAction::PickSide(Side::Target, Pick::Collection("v".into())),
    );
    assert!(cmds.is_empty());
    assert!(!state.compare.setup.ready());
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty());
}

#[test]
fn start_needs_a_check_and_editing_the_form_invalidates_it() {
    let (_, cmds) = act(picked_state(), CompareAction::Start);
    assert!(cmds.is_empty(), "not checked yet");
    let (state, _) = act(checked_state(), CompareAction::SetKey("sku".into()));
    assert_eq!(state.compare.preflight, PreflightView::Unchecked);
    let (_, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty());
}

#[test]
fn a_check_answer_that_arrives_after_an_edit_is_dropped() {
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let (state, _) = act(state, CompareAction::SetFilter(Side::Source, "{}".into()));
    let (state, _) = update(state, checked_event(indexed(1)));
    assert_eq!(state.compare.preflight, PreflightView::Unchecked);
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
        state.compare.preflight,
        PreflightView::NeedsIndex { side: Side::Target }
    );
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty());
    assert_eq!(state.compare.run, RunView::Idle);
}

#[test]
fn a_bad_key_blocks_the_check() {
    let (state, _) = act(picked_state(), CompareAction::SetKey("a,,b".into()));
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty());
}

fn pair_done(run: u64, summary: Summary) -> Input {
    ev(CompareEvent::PairDone {
        session: 1,
        run,
        pair: 1,
        outcome: PairOutcome::Completed(summary),
    })
}

fn run_done() -> Input {
    ev(CompareEvent::RunDone { session: 1, run: 1 })
}

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
    let state = running_state();
    let (after, cmds) = update(state, pair_done(0, Summary::default()));
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

fn with_differences() -> AppState {
    let summary = Summary {
        different: 250,
        only_in_source: 4,
        ..Summary::default()
    };
    let (state, cmds) = update(running_state(), pair_done(1, summary));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows {
            session: 1,
            pair: 1,
            category: Category::Different,
            offset: 0,
            limit: 100
        })]
    ));
    state
}

fn rows_event(category: Category, offset: usize, n: i32) -> Input {
    ev(CompareEvent::Rows {
        session: 1,
        pair: 1,
        category,
        offset,
        total: 250,
        rows: (0..n)
            .map(|i| RowView {
                key: KeyTuple(vec![Bson::Int32(i)]),
                has_source: true,
                has_target: true,
            })
            .collect(),
    })
}

fn with_rows() -> AppState {
    update(with_differences(), rows_event(Category::Different, 0, 3)).0
}

#[test]
fn pair_done_requests_the_first_page_and_fresh_rows_fill_it() {
    let state = with_rows();
    assert_eq!(state.compare.results.as_ref().unwrap().rows.len(), 3);
}

#[test]
fn stale_rows_are_dropped_by_the_reducer() {
    let state = with_differences();
    let (state, _) = update(state, rows_event(Category::OnlyInSource, 0, 3));
    let (state, _) = update(state, rows_event(Category::Different, 100, 3));
    assert!(state.compare.results.unwrap().rows.is_empty());
}

#[test]
fn paging_category_and_select_send_the_matching_commands() {
    let (state, cmds) = act(with_rows(), CompareAction::Page(1));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows { offset: 100, .. })]
    ));
    let (state, cmds) = act(state, CompareAction::SelectCategory(Category::OnlyInSource));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows {
            category: Category::OnlyInSource,
            offset: 0,
            ..
        })]
    ));
    let (_, cmds) = act(state, CompareAction::Page(-1));
    assert!(cmds.is_empty(), "already at the first page");
}

#[test]
fn select_requests_detail_and_back_and_the_answer_fill_the_view() {
    let key = KeyTuple(vec![Bson::Int32(1)]);
    let (state, cmds) = act(with_rows(), CompareAction::Select(key.clone()));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Detail {
            session: 1,
            pair: 1,
            ..
        })]
    ));
    assert!(state.compare.detail.as_ref().unwrap().loading);
    let answer = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key,
        source: Some(bson::doc! {"a": 1}),
        target: Some(bson::doc! {"a": 2}),
        changes: vec![],
    };
    let (state, _) = update(state, ev(answer));
    assert!(!state.compare.detail.as_ref().unwrap().loading);
    let (state, _) = act(state, CompareAction::Back);
    assert!(state.compare.detail.is_none());
}

#[test]
fn a_detail_answer_for_another_key_is_dropped() {
    let (state, _) = act(
        with_rows(),
        CompareAction::Select(KeyTuple(vec![Bson::Int32(1)])),
    );
    let answer = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key: KeyTuple(vec![Bson::Int32(2)]),
        source: None,
        target: None,
        changes: vec![],
    };
    let (state, _) = update(state, ev(answer));
    assert!(state.compare.detail.unwrap().loading);
}

#[test]
fn selecting_an_unlisted_key_does_nothing() {
    let key = KeyTuple(vec![Bson::Int32(99)]);
    let (state, cmds) = act(with_rows(), CompareAction::Select(key));
    assert!(cmds.is_empty() && state.compare.detail.is_none());
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
fn a_failed_check_returns_the_preflight_to_unchecked() {
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    assert_eq!(state.compare.preflight, PreflightView::Checking);
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_check",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, failed);
    assert_eq!(state.compare.preflight, PreflightView::Unchecked);
}

fn finished_with_results() -> AppState {
    let summary = Summary {
        different: 1,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, summary));
    update(state, run_done()).0
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
    let (state, _, _) = opened();
    let (state, cmds) = update(state, Input::Disconnect);
    assert!(
        state.compare.session.is_some(),
        "the compare session has its own connections"
    );
    assert!(!cmds.iter().any(|c| matches!(c, Command::Compare(_))));
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
