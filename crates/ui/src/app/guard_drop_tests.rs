//! The query bar's guard goes with the run it belongs to: a Clear, another collection, the
//! switch turned off, a lost connection, a reconnect or a closed session drop it, and an answer
//! that arrives afterwards sends nothing.

use super::explain_tests::{act, query_a};
use super::guard_actions::GuardState;
use super::guard_tests::{checked, checked_in, guarded, query_b, scan_report, sorted_only};
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::{FindAction, SidebarAction};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::guard::GuardReport;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

#[test]
fn clear_drops_the_check_and_the_warning() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::Clear);
    assert_eq!(*s.guard, GuardState::None);
    let (_, cmds) = checked(s, 1, Ok(GuardReport::default()));
    assert!(cmds.is_empty(), "{cmds:?}");

    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, _) = act(s, FindAction::Clear);
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn opening_a_collection_drops_the_check_and_the_warning() {
    let open = || Input::Sidebar(SidebarAction::Open(Namespace::new("app", "pets")));
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = update(s, open());
    assert_eq!(*s.guard, GuardState::None);
    let (_, cmds) = checked(s, 1, Ok(GuardReport::default()));
    assert!(cmds.is_empty(), "{cmds:?}");

    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, _) = update(s, open());
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn turning_the_guard_off_drops_the_check_and_the_warning() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::SetGuard(false));
    assert_eq!(*s.guard, GuardState::None);
    let (_, cmds) = checked(s, 1, Ok(GuardReport::default()));
    assert!(cmds.is_empty(), "{cmds:?}");

    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, _) = act(s, FindAction::SetGuard(false));
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn turning_the_guard_on_keeps_a_warning() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, _) = act(s, FindAction::SetGuard(true));
    assert!(matches!(*s.guard, GuardState::Warn { .. }), "{:?}", s.guard);
}

/// A saved connection `id`, open, connected and browsing `app.people` with the guard on.
fn saved_session() -> (AppState, Uuid) {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let sidebar = guarded().sidebar;
    let view = SessionView {
        status: ConnStatus::Connected,
        sidebar,
        ..SessionView::default()
    };
    let mut state = AppState::new(vec![cfg]).opened_with(id, view);
    state.explain.guard = true;
    (state, id)
}

fn lost(state: AppState, session: Uuid) -> AppState {
    let event = Event::Failed {
        session,
        context: "list_databases",
        error: CoreError::Unreachable,
    };
    update(state, Input::Event(event)).0
}

#[test]
fn a_lost_connection_and_a_reconnect_drop_the_check() {
    let (s, id) = saved_session();
    let (s, cmds) = act(s, FindAction::Run(query_a()));
    assert!(matches!(cmds.as_slice(), [Command::GuardCheck { session, .. }] if *session == id));
    let s = lost(s, id);
    assert_eq!(s.status, ConnStatus::Failed);
    assert_eq!(*s.guard, GuardState::None);
    let (s, _) = update(s, Input::Connect(id));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn a_lost_connection_drops_the_warning() {
    let (s, id) = saved_session();
    let (s, _) = act(s, FindAction::Run(query_a()));
    let (s, _) = checked_in(s, id, 1, Ok(scan_report(query_a())));
    assert!(matches!(*s.guard, GuardState::Warn { .. }), "{:?}", s.guard);
    let s = lost(s, id);
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn closing_the_session_drops_its_check() {
    let (s, id) = saved_session();
    let (s, _) = act(s, FindAction::Run(query_a()));
    let (s, _) = update(s, Input::Disconnect(id));
    let (s, cmds) = checked_in(s, id, 1, Ok(GuardReport::default()));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn an_answer_for_a_session_in_the_background_goes_on_there() {
    let (s, id) = saved_session();
    let (s, _) = act(s, FindAction::Run(query_a()));
    let other = ConnectionConfig::new("b", "mongodb://other");
    let other_id = other.id;
    let mut s = s;
    s.conn.connections.push(other);
    let s = s.opened_with(other_id, SessionView::default());
    assert_eq!(s.active, Some(other_id));
    let (s, cmds) = checked_in(s, id, 1, Ok(GuardReport::default()));
    match cmds.as_slice() {
        [Command::Find { session, .. }, Command::ExplainFind { .. }] => assert_eq!(*session, id),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.active, Some(other_id));
    assert_eq!(*s.guard, GuardState::None);
    assert_eq!(s.guard_request, 1);
}

#[test]
fn every_run_takes_a_new_request_number() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::Run(query_b()));
    let (s, cmds) = act(s, FindAction::Run(sorted_only()));
    assert!(
        matches!(cmds.as_slice(), [Command::GuardCheck { request: 3, .. }]),
        "{cmds:?}"
    );
    assert_eq!(s.guard_request, 3);
}
