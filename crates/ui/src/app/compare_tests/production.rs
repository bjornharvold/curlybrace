//! A compare session counts a connection as production when the flag it opened with OR the saved
//! connection's live flag says so: ticking applies at once, unticking on the next open.
use super::*;

fn prod(state: &AppState, id: uuid::Uuid) -> bool {
    state.compare.is_production(id, &state.conn.connections)
}

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
    assert!(!prod(&state, a));
    assert!(prod(&state, b));
}

#[test]
fn ticking_a_saved_connection_applies_to_the_open_compare_session_at_once() {
    let (state, a, b) = flagged(false, false);
    let (mut state, _) = act(state, open_action(a, b));
    state.conn.connections[0].production = true;
    assert!(prod(&state, a));
    assert!(!prod(&state, b));
}

#[test]
fn reopening_the_same_pair_after_ticking_is_still_flagged() {
    let (state, a, b) = flagged(false, false);
    let (mut state, _) = act(state, open_action(a, b));
    state.conn.connections[1].production = true;
    let (state, _) = act(state, open_action(a, b));
    assert!(
        prod(&state, b),
        "reopening returns early; the live flag still counts"
    );
}

#[test]
fn unticking_keeps_the_open_compare_session_production_until_reopened() {
    let (state, a, b) = flagged(false, true);
    let (mut state, _) = act(state, open_action(a, b));
    state.conn.connections[1].production = false;
    assert!(prod(&state, b), "the snapshot still says production");
    let (state, _) = update(state, ev(CompareEvent::Opened { session: 1 }));
    let (state, _) = act(state, CompareAction::CloseSession);
    let (state, _) = act(state, open_action(a, b));
    assert!(!prod(&state, b), "the next open takes the new flag");
}

#[test]
fn a_connection_deleted_while_compare_is_open_uses_its_snapshot_alone() {
    let (state, a, b) = flagged(false, true);
    let (mut state, _) = act(state, open_action(a, b));
    state.conn.connections.clear();
    assert!(prod(&state, b));
    assert!(!prod(&state, a));
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
    assert!(prod(&state, a));
    assert!(!prod(&state, b));
}

#[test]
fn an_unrelated_connection_is_never_production() {
    let (state, a, b) = flagged(true, true);
    let (state, _) = act(state, open_action(a, b));
    assert!(!prod(&state, uuid::Uuid::new_v4()));
}
