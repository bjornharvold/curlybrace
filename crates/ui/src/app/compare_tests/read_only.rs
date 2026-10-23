//! A compare apply is refused when its target connection is read-only (saved flag or the flag the
//! session opened with): no `Apply` is sent and a localized banner says why.
use super::confirm::{applies, confirmed};
use super::*;
use crate::viewmodel::compare::ApplyUi;

fn target_of(state: &AppState) -> uuid::Uuid {
    state.compare.connections.expect("an open session").1
}

#[test]
fn compare_apply_not_sent_when_target_read_only() {
    // Saved flag ticked after the session opened.
    let mut state = confirmed();
    let target = target_of(&state);
    for c in state.conn.connections.iter_mut().filter(|c| c.id == target) {
        c.read_only = true;
    }
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&cmds).is_empty(), "{cmds:?}");
    assert_eq!(state.banner, Some(Banner::CompareApplyReadOnly));
    assert_eq!(state.compare.apply, ApplyUi::Idle);

    // Snapshot taken when the session opened, flag since unticked.
    let mut state = confirmed();
    state.compare.connections_read_only.1 = true;
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&cmds).is_empty());
    assert_eq!(state.banner, Some(Banner::CompareApplyReadOnly));
}

#[test]
fn a_read_only_source_does_not_block_the_apply() {
    let mut state = confirmed();
    let source = state.compare.connections.unwrap().0;
    for c in state.conn.connections.iter_mut().filter(|c| c.id == source) {
        c.read_only = true;
    }
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    assert_eq!(applies(&cmds).len(), 1);
}

#[test]
fn opening_snapshots_the_read_only_flag_of_each_side() {
    let (mut state, a, b) = two_connections();
    state.conn.connections[1].read_only = true;
    let (state, _) = act(state, open_action(a, b));
    assert_eq!(state.compare.connections_read_only, (false, true));
}
