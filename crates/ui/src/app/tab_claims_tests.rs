//! A profiler change of a parked tab is settled by the answer to the CHANGE even when the current
//! tab is waiting for a read of the same database (#159): `ProfilingState` does not say which it
//! answers, so the holders of a change claim it first, then the holders of a read.
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{act, ev, parked, two_tabs};
use crate::viewmodel::queries::InFlight;
use crate::viewmodel::{CollectionTab, IndexesAction, QueriesAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::ProfilingState;
use uuid::Uuid;

fn answer(session: Uuid) -> Event {
    Event::ProfilingState {
        session,
        db: "app".into(),
        outcome: Ok(ProfilingState {
            level: 0,
            slow_ms: 100,
        }),
    }
}

/// The parked tab has a profiler change in flight (`pending` names it) and the current tab a
/// read of the same database.
fn change_in_parked_read_in_current() -> (AppState, Uuid, TabId) {
    let t = two_tabs();
    let q = |a| Input::Indexes(IndexesAction::Queries(a));
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Indexes));
    let (s, _) = act(s, t.second, q(QueriesAction::Open));
    let s = ev(s, answer(t.a));
    let (s, _) = update(s, Input::Tab(CollectionTab::Indexes));
    let (s, read) = update(s, q(QueriesAction::Open));
    assert!(
        read.iter()
            .any(|c| matches!(c, Command::ProfilingState { .. })),
        "{read:?}"
    );
    let (s, _) = act(s, t.second, q(QueriesAction::AskEnable));
    let (s, change) = act(s, t.second, q(QueriesAction::ConfirmEnable));
    assert!(
        change
            .iter()
            .any(|c| matches!(c, Command::SetProfilingLevel { .. })),
        "{change:?}"
    );
    assert!(matches!(
        s.indexes.queries.in_flight,
        Some(InFlight::Read(_))
    ));
    assert!(matches!(
        parked(&s, t.second).indexes.queries.in_flight,
        Some(InFlight::Write(_))
    ));
    (s, t.a, t.second)
}

#[test]
fn the_answer_to_a_parked_change_settles_it_before_the_current_tabs_read() {
    let (s, a, second) = change_in_parked_read_in_current();
    let s = ev(s, answer(a));
    assert!(
        parked(&s, second).indexes.queries.in_flight.is_none(),
        "the change is settled"
    );
    assert!(s.pending.is_none(), "so later writes are not refused");
    assert!(
        matches!(s.indexes.queries.in_flight, Some(InFlight::Read(_))),
        "the current tab still waits for its read"
    );
}

#[test]
fn the_failure_of_the_current_tabs_read_then_settles_the_read_and_not_the_change() {
    let (s, a, second) = change_in_parked_read_in_current();
    let s = ev(s, answer(a));
    let s = ev(
        s,
        Event::Failed {
            session: a,
            context: "profiling_state",
            error: CoreError::Database("no".into()),
        },
    );
    assert!(s.indexes.queries.in_flight.is_none(), "the read is settled");
    assert!(parked(&s, second).indexes.queries.in_flight.is_none());
    assert!(s.pending.is_none());
}

#[test]
fn the_second_answer_settles_the_read() {
    let (s, a, second) = change_in_parked_read_in_current();
    let s = ev(s, answer(a));
    let s = ev(s, answer(a));
    assert!(s.indexes.queries.in_flight.is_none());
    assert!(parked(&s, second).indexes.queries.in_flight.is_none());
    assert!(s.pending.is_none());
}
