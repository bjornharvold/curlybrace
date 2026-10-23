//! A read-only connection never receives a write, whichever side of the compare it is on: an
//! apply, a review or a queued action that writes to a read-only connection (saved flag or the
//! flag the session opened with) is refused, no `Apply` is sent and a localized banner says why.
use super::confirm::{applies, confirmed};
use super::*;
use crate::viewmodel::compare::{ApplyUi, RowAction};

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

/// The queued changes all write to the target, so a read-only source is only read.
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

fn source_of(state: &AppState) -> uuid::Uuid {
    state.compare.connections.expect("an open session").0
}

fn with_saved_read_only(mut state: AppState, id: uuid::Uuid) -> AppState {
    for c in state.conn.connections.iter_mut().filter(|c| c.id == id) {
        c.read_only = true;
    }
    state
}

/// Different rows 0 and 1 queued to copy to the SOURCE, the review open.
fn reviewing_copy_to_source() -> AppState {
    use super::confirm::{pending_for, reviewing};
    reviewing(pending_for(Category::Different, RowAction::CopyToSource))
}

#[test]
fn compare_apply_not_sent_when_ops_go_to_a_read_only_source() {
    // Saved flag ticked after the review opened.
    let state = reviewing_copy_to_source();
    let source = source_of(&state);
    let (state, cmds) = act(
        with_saved_read_only(state, source),
        CompareAction::ConfirmApply,
    );
    assert!(applies(&cmds).is_empty(), "{cmds:?}");
    assert_eq!(state.banner, Some(Banner::CompareApplyReadOnlySource));
    assert_eq!(state.compare.apply, ApplyUi::Idle);

    // Snapshot taken when the session opened (or raised by a lock), flag since unticked.
    let mut state = reviewing_copy_to_source();
    state.compare.connections_read_only.0 = true;
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&cmds).is_empty());
    assert_eq!(state.banner, Some(Banner::CompareApplyReadOnlySource));
}

#[test]
fn review_is_refused_when_the_pending_list_writes_to_a_read_only_source() {
    use super::confirm::{pending_for, review_action};
    let state = pending_for(Category::Different, RowAction::CopyToSource);
    let source = source_of(&state);
    let (state, cmds) = act(with_saved_read_only(state, source), review_action());
    assert!(cmds.is_empty());
    assert!(state.compare.apply.review().is_none(), "no review opens");
    assert_eq!(state.banner, Some(Banner::CompareApplyReadOnlySource));
}

#[test]
fn copy_and_delete_into_a_read_only_source_are_not_queued() {
    use super::confirm::{tab_of, toggled};
    for (cat, action) in [
        (Category::Different, RowAction::CopyToSource),
        (Category::OnlyInTarget, RowAction::CopyToSource),
        (Category::OnlyInSource, RowAction::DeleteFromSource),
    ] {
        let state = toggled(tab_of(cat), &[0, 1]);
        let source = source_of(&state);
        let (state, cmds) = act(
            with_saved_read_only(state, source),
            CompareAction::AddToPending(action),
        );
        assert!(cmds.is_empty());
        assert!(state.compare.pending.is_empty(), "{action:?} not queued");
    }
    // The other direction still queues.
    let state = toggled(tab_of(Category::Different), &[0, 1]);
    let source = source_of(&state);
    let to_target = CompareAction::AddToPending(RowAction::CopyToTarget);
    let (state, _) = act(with_saved_read_only(state, source), to_target);
    assert_eq!(state.compare.pending.items().len(), 1);
}

#[test]
fn copy_selected_fields_to_a_read_only_source_is_not_queued() {
    let state = super::fields::ticked();
    let source = source_of(&state);
    let mut state = with_saved_read_only(state, source);
    state.compare.connections_read_only.0 = true;
    let to_source = CompareAction::AddFieldsToPending(RowAction::CopyFieldsToSource);
    let (state, cmds) = act(state, to_source);
    assert!(cmds.is_empty());
    assert!(state.compare.pending.is_empty());
    let to_target = CompareAction::AddFieldsToPending(RowAction::CopyFieldsToTarget);
    let (state, _) = act(state, to_target);
    assert_eq!(state.compare.pending.items().len(), 1);
}
