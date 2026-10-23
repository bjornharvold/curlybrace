use super::*;
use crate::viewmodel::compare::{FieldBlock, FieldRow, PendingList, RowAction, Scope};
use bson::Bson;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::results::Category;

fn state(read_only: (bool, bool)) -> (CompareState, Uuid, Uuid) {
    let (source, target) = (Uuid::new_v4(), Uuid::new_v4());
    let state = CompareState {
        connections: Some((source, target)),
        connections_read_only: read_only,
        ..CompareState::default()
    };
    (state, source, target)
}

fn queued(state: CompareState, cat: Category, action: RowAction) -> CompareState {
    let keys = Scope::Keys(vec![KeyTuple(vec![Bson::Int32(1)])]);
    let (pending, _) = PendingList::default().with_action(1, cat, action, keys);
    CompareState { pending, ..state }
}

fn saved(id: Uuid, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        id,
        read_only,
        ..ConnectionConfig::new("c", "mongodb://c")
    }
}

#[test]
fn destination_follows_the_direction() {
    let (s, source, target) = state((false, false));
    assert_eq!(
        s.destination_connection(Direction::SourceToTarget),
        Some(target)
    );
    assert_eq!(
        s.destination_connection(Direction::TargetToSource),
        Some(source)
    );
    assert_eq!(
        CompareState::default().destination_connection(Direction::SourceToTarget),
        None
    );
}

#[test]
fn a_read_only_source_is_a_read_only_destination_only_for_writes_to_it() {
    let (s, _, _) = state((true, false));
    let to_target = queued(s.clone(), Category::Different, RowAction::CopyToTarget);
    assert_eq!(to_target.read_only_destination(&[]), None);
    let to_source = queued(s.clone(), Category::Different, RowAction::CopyToSource);
    assert_eq!(to_source.read_only_destination(&[]), Some(Side::Source));
    let delete = queued(s, Category::OnlyInSource, RowAction::DeleteFromSource);
    assert_eq!(delete.read_only_destination(&[]), Some(Side::Source));
}

#[test]
fn the_live_saved_flag_counts_like_the_snapshot() {
    let (s, source, _) = state((false, false));
    let s = queued(s, Category::Different, RowAction::CopyToSource);
    assert_eq!(s.read_only_destination(&[saved(source, false)]), None);
    assert_eq!(
        s.read_only_destination(&[saved(source, true)]),
        Some(Side::Source)
    );
}

#[test]
fn a_read_only_target_is_named_first() {
    let (s, _, _) = state((true, true));
    let s = queued(s, Category::Different, RowAction::CopyToSource);
    let (pending, _) = s.pending.with_action(
        2,
        Category::Different,
        RowAction::CopyToTarget,
        Scope::Category,
    );
    let s = CompareState { pending, ..s };
    assert_eq!(s.read_only_destination(&[]), Some(Side::Target));
}

#[test]
fn guarded_buttons_block_only_writes_to_a_read_only_side() {
    let (s, _, _) = state((true, false));
    let buttons = RowAction::offered_for(Category::Different)
        .iter()
        .map(|&action| ActionButton {
            action,
            block: None,
            read_only: false,
        })
        .collect();
    let guarded = s.guard_buttons(buttons, &[]);
    let marks: Vec<_> = guarded
        .iter()
        .map(|b| (b.action, b.block, b.read_only))
        .collect();
    assert_eq!(
        marks,
        vec![
            (RowAction::CopyToTarget, None, false),
            (RowAction::CopyToSource, None, true),
        ]
    );
}

#[test]
fn a_guarded_button_keeps_its_own_block_beside_the_read_only_mark() {
    let (s, _, _) = state((true, false));
    let button = ActionButton {
        action: RowAction::CopyToSource,
        block: Some(ActionBlock::Running),
        read_only: false,
    };
    let guarded = s.guard_buttons(vec![button], &[]);
    assert_eq!(guarded[0].block, Some(ActionBlock::Running));
    assert!(guarded[0].read_only);
    assert!(!guarded[0].is_enabled());
}

#[test]
fn review_reasons_list_the_block_then_the_read_only_side() {
    let (s, _, _) = state((false, true));
    let s = queued(s, Category::Different, RowAction::CopyToTarget);
    let busy = CompareState {
        apply: crate::viewmodel::compare::ApplyUi::Running {
            run: 9,
            total: 1,
            rows: Vec::new(),
            cancel_requested: false,
        },
        ..s.clone()
    };
    assert_eq!(
        busy.review_reasons(&[]),
        vec![
            ReviewReason::Block(ActionBlock::Busy),
            ReviewReason::ReadOnly(Side::Target)
        ]
    );
}

#[test]
fn review_reasons_name_a_lone_read_only_side_and_nothing_else() {
    let (s, _, _) = state((false, true));
    let s = queued(s, Category::Different, RowAction::CopyToTarget);
    assert_eq!(
        s.review_reasons(&[]).last(),
        Some(&ReviewReason::ReadOnly(Side::Target))
    );
    let (writable, _, _) = state((false, false));
    let writable = queued(writable, Category::Different, RowAction::CopyToTarget);
    assert!(
        !writable
            .review_reasons(&[])
            .iter()
            .any(|r| matches!(r, ReviewReason::ReadOnly(_)))
    );
}

#[test]
fn guarded_field_bar_blocks_copy_to_a_read_only_source() {
    let (s, _, _) = state((true, false));
    let bar = FieldBar {
        rows: vec![FieldRow {
            ticked: true,
            block: None,
        }],
        selected: 1,
        to_target: None,
        to_source: Some(FieldBlock::NoFields),
        to_target_read_only: false,
        to_source_read_only: false,
        clear: None,
    };
    let guarded = s.guard_field_bar(bar, &[]);
    assert_eq!(guarded.to_target, None);
    assert!(!guarded.to_target_read_only);
    // The block stays beside the read-only mark, so both are stated.
    assert_eq!(guarded.to_source, Some(FieldBlock::NoFields));
    assert!(guarded.to_source_read_only);
}
