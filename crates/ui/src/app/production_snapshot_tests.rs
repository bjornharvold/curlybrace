//! An open session counts as production when the flag it opened with OR the saved connection's
//! live flag says so: ticking applies at once, unticking only on the next connect.
use super::reducer::{Input, update};
use super::state::AppState;
use crate::I18n;
use crate::viewmodel::EditorAction;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::Namespace;
use curlybrace_core::test_support::mock_objects::mock_connection;
use uuid::Uuid;

fn config(name: &str, production: bool) -> ConnectionConfig {
    ConnectionConfig {
        name: name.into(),
        production,
        id: Uuid::new_v4(),
        ..mock_connection()
    }
}

fn connected(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    let (mut s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    );
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["users".into()])
        .selecting(Namespace::new("app", "users"));
    s
}

fn insert() -> Input {
    Input::Editor(EditorAction::Save {
        original_id: None,
        text: "{}".into(),
    })
}

fn set_production(state: &mut AppState, id: Uuid, production: bool) {
    for c in state.conn.connections.iter_mut().filter(|c| c.id == id) {
        c.production = production;
    }
}

#[test]
fn unticking_production_on_the_saved_connection_leaves_the_open_session_guarded() {
    let cfg = config("prod", true);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    set_production(&mut s, id, false);
    assert!(s.active_is_production());
    assert!(s.active_badge().production);
    assert!(s.roots(&I18n::english())[0].badge().production);
    let (s, commands) = update(s, insert());
    assert!(commands.is_empty(), "the write is still parked");
    assert!(s.write_warning.is_some());
}

#[test]
fn ticking_production_on_the_saved_connection_guards_the_open_session_at_once() {
    let cfg = config("plain", false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    set_production(&mut s, id, true);
    assert!(s.active_is_production());
    assert!(s.session_production(id));
    assert!(s.active_badge().production);
    assert!(s.session_badge(id).production);
    assert!(s.roots(&I18n::english())[0].badge().production);
    let (s, commands) = update(s, insert());
    assert!(commands.is_empty(), "the write is parked");
    assert!(s.write_warning.is_some());
}

#[test]
fn connecting_again_after_ticking_keeps_the_open_session_guarded() {
    let cfg = config("plain", false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    set_production(&mut s, id, true);
    let (s, _) = update(s, Input::Connect(id));
    assert!(s.active_is_production());
}

#[test]
fn a_connection_deleted_while_open_uses_its_snapshot_alone() {
    let cfg = config("prod", true);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    s.conn.connections.clear();
    assert!(s.active_is_production());
    assert!(s.session_production(id));
    let cfg = config("plain", false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    s.conn.connections.clear();
    assert!(!s.active_is_production());
}

#[test]
fn reconnecting_picks_up_the_new_flag() {
    let cfg = config("prod", true);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    set_production(&mut s, id, false);
    let (s, _) = update(s, Input::Disconnect(id));
    let s = connected(s, id);
    assert!(!s.active_is_production());
    assert!(!s.session_production(id));
}

#[test]
fn two_sessions_keep_their_own_snapshots_when_parked_and_restored() {
    let (a, b) = (config("a", true), config("b", false));
    let (ida, idb) = (a.id, b.id);
    let s = connected(AppState::new(vec![a, b]), ida);
    let mut s = connected(s, idb);
    set_production(&mut s, ida, false);
    set_production(&mut s, idb, true);
    assert!(s.session_production(ida), "parked A keeps its snapshot");
    assert!(s.session_production(idb), "active B was ticked");
    set_production(&mut s, idb, false);
    assert!(!s.session_production(idb), "active B is plain again");
    let roots = s.roots(&I18n::english());
    assert_eq!(
        roots
            .iter()
            .map(|r| r.badge().production)
            .collect::<Vec<_>>(),
        [true, false]
    );
    let (s, _) = update(s, Input::Activate(ida));
    assert!(s.active_is_production());
    assert!(!s.session_production(idb), "parked B is plain");
    let (s, _) = update(s, Input::Activate(idb));
    assert!(!s.active_is_production());
    assert!(s.session_production(ida));
}

#[test]
fn the_embedded_session_stays_non_production() {
    let s = AppState::new(vec![config("prod", true)]);
    assert!(!s.active_is_production());
    assert!(!s.session_production(Uuid::nil()));
}
