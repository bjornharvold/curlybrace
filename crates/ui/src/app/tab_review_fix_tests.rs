//! Fixes after the review of slice 2 (#159): what a copy takes, what a dormant tab must not do,
//! tabs nobody can see, the stale mark after Run.
use super::TabAction;
use super::tests::tabs;
use crate::app::state::AppState;
use crate::app::tabs::test_support::{ev, parked, two_tabs, users};
use crate::viewmodel::aggregate_run::RunStatus;
use curlybrace_core::runtime::Event;

const RUN: u64 = 987_654_321;

/// `s` with a `$out` run (number `RUN`) in flight in the tab being looked at.
fn running(mut s: AppState) -> AppState {
    s.aggregate.run.status = RunStatus::Running;
    s.aggregate.run.run = RUN;
    s.aggregate.run.stage = "$out".into();
    s
}

fn ran(session: uuid::Uuid) -> Event {
    Event::AggregateRan {
        session,
        run: RUN,
        ns: users(),
        target: users(),
        outcome: Ok(None),
    }
}

#[test]
fn a_duplicate_does_not_take_the_sources_run_answer() {
    let t = two_tabs();
    let source = t.s.active_tab().expect("source");
    let s = running(t.s);
    let (s, _) = tabs(s, TabAction::Duplicate(source));
    let copy = s.active_tab().expect("copy");
    assert_eq!(
        s.aggregate.run.status,
        RunStatus::Idle,
        "the copy never ran anything, and its connection never dropped"
    );
    assert_eq!(s.aggregate.run.run, 0);
    // Back to the source: the copy is parked now.
    let (s, _) = tabs(s, TabAction::Activate(source));
    assert_eq!(s.aggregate.run.status, RunStatus::Running);
    let s = ev(s, ran(t.a));
    assert_ne!(
        s.aggregate.run.status,
        RunStatus::Running,
        "the answer reached the tab that ran"
    );
    assert_eq!(parked(&s, copy).aggregate.run.status, RunStatus::Idle);
}

#[test]
fn a_new_tab_needs_a_connected_session() {
    use crate::app::state::ConnStatus;
    for status in [ConnStatus::Failed, ConnStatus::Connecting] {
        let mut s = two_tabs().s;
        s.status = status;
        let before = s.tabs.order.len();
        let (s, commands) = tabs(s, TabAction::New);
        assert!(commands.is_empty());
        assert_eq!(s.tabs.order.len(), before, "{status:?}");
    }
}

#[test]
fn a_reopened_tab_takes_no_run_either() {
    let t = two_tabs();
    let source = t.s.active_tab().expect("source");
    let s = running(t.s);
    let (s, _) = tabs(s, TabAction::Close(source));
    let (s, _) = tabs(s, TabAction::ReopenClosed);
    assert_eq!(s.aggregate.run.status, RunStatus::Idle);
    assert_eq!(s.aggregate.run.run, 0);
}
