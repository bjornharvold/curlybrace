//! The compare session snapshots which of its two connections are production when it opens.
use super::*;

fn flagged(source: bool, target: bool) -> (AppState, uuid::Uuid, uuid::Uuid) {
    let (mut state, a, b) = two_connections();
    state.conn.connections[0].production = source;
    state.conn.connections[1].production = target;
    (state, a, b)
}

#[test]
fn opening_snapshots_the_production_flag_of_each_side() {
    let (state, a, b) = flagged(false, true);
    let (state, _) = act(state, open_action(a, b));
    assert!(!state.compare.is_production(a));
    assert!(state.compare.is_production(b));
}

#[test]
fn editing_a_saved_connection_does_not_change_the_open_compare_session() {
    let (state, a, b) = flagged(false, true);
    let (mut state, _) = act(state, open_action(a, b));
    state.conn.connections[0].production = true;
    state.conn.connections[1].production = false;
    assert!(!state.compare.is_production(a));
    assert!(state.compare.is_production(b));
}

#[test]
fn opening_another_pair_takes_the_new_flags() {
    let (state, a, b) = flagged(false, true);
    let (state, _) = act(state, open_action(a, b));
    let (mut state, _) = update(state, ev(CompareEvent::Opened { session: 1 }));
    state.conn.connections[0].production = true;
    state.conn.connections[1].production = false;
    let (state, _) = act(state, CompareAction::CloseSession);
    let (state, _) = act(state, open_action(a, b));
    assert!(state.compare.is_production(a));
    assert!(!state.compare.is_production(b));
}

#[test]
fn an_unrelated_connection_is_never_production() {
    let (state, a, b) = flagged(true, true);
    let (state, _) = act(state, open_action(a, b));
    assert!(!state.compare.is_production(uuid::Uuid::new_v4()));
}
