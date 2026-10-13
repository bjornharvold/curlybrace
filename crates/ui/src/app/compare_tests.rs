//! Shared fixtures for the compare reducer tests; the tests themselves live in the submodules.
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

#[path = "compare_tests/duplicates.rs"]
mod duplicates;
#[path = "compare_tests/entry.rs"]
mod entry;
#[path = "compare_tests/frames.rs"]
mod frames;
#[path = "compare_tests/results.rs"]
mod results;
#[path = "compare_tests/run.rs"]
mod run;
#[path = "compare_tests/setup.rs"]
mod setup;

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

/// One connection on both sides, and the same database and collection picked on both.
fn same_collection_state() -> AppState {
    let (state, a, _) = two_connections();
    let (state, _) = act(state, open_action(a, a));
    let (mut state, _) = update(state, ev(CompareEvent::Opened { session: 1 }));
    for side in [Side::Source, Side::Target] {
        let (s, _) = act(
            state,
            CompareAction::PickSide(side, Pick::Database("a".into())),
        );
        let listing = CompareEvent::Collections {
            session: 1,
            side,
            db: "a".into(),
            entries: vec![("users".into(), CollectionKind::Collection)],
        };
        let (s, _) = update(s, ev(listing));
        let pick = CompareAction::PickSide(side, Pick::Collection("users".into()));
        state = act(s, pick).0;
    }
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

fn finished_with_results() -> AppState {
    let summary = Summary {
        different: 1,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, summary));
    update(state, run_done()).0
}
