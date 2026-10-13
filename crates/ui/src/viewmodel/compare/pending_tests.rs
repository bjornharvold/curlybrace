use super::*;
use bson::Bson;

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

const SRC: Category = Category::OnlyInSource;

fn keys(ns: &[i32]) -> Scope {
    Scope::Keys(ns.iter().map(|n| k(*n)).collect())
}

#[test]
fn adds_keys_in_order() {
    let (l, ignored) =
        PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1, 2]));
    assert_eq!(ignored, 0);
    let (l, _) = l.with_action(2, SRC, RowAction::DeleteFromSource, keys(&[3]));
    assert_eq!(l.items().len(), 2);
    assert_eq!(l.items()[0].pair, 1);
    assert_eq!(l.items()[1].pair, 2);
    assert!(!l.is_empty());
}

#[test]
fn category_replaces_keys_and_other_category_items() {
    let (l, _) = PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1]));
    let (l, _) = l.with_action(1, SRC, RowAction::DeleteFromSource, Scope::Category);
    let (l, _) = l.with_action(2, SRC, RowAction::CopyToTarget, keys(&[7]));
    let (l, ignored) = l.with_action(1, SRC, RowAction::CopyToTarget, Scope::Category);
    assert_eq!(ignored, 0);
    assert_eq!(l.items().len(), 2);
    assert_eq!(l.items()[0].pair, 2, "other pair untouched");
    assert_eq!(l.items()[1].scope, Scope::Category);
    assert_eq!(l.items()[1].action, RowAction::CopyToTarget);
}

#[test]
fn keys_replace_in_other_key_items_and_drop_emptied() {
    let (l, _) = PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1, 2]));
    let (l, _) = l.with_action(1, SRC, RowAction::DeleteFromSource, keys(&[2, 3]));
    assert_eq!(l.items()[0].scope, keys(&[1]));
    assert_eq!(l.items()[1].scope, keys(&[2, 3]));
    let (l, _) = l.with_action(1, SRC, RowAction::CopyToTarget, keys(&[1]));
    assert_eq!(l.items().len(), 2, "emptied item dropped");
    assert_eq!(l.items()[0].scope, keys(&[2, 3]));
}

#[test]
fn category_item_swallows_keys() {
    let (l, _) =
        PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, Scope::Category);
    let (l2, ignored) = l.with_action(1, SRC, RowAction::DeleteFromSource, keys(&[1, 2, 3]));
    assert_eq!(ignored, 3);
    assert_eq!(l2, l);
    let (l3, ignored) = l.with_action(1, Category::Different, RowAction::CopyToSource, keys(&[1]));
    assert_eq!(ignored, 0);
    assert_eq!(l3.items().len(), 2);
}

#[test]
fn without_removes_by_index_pair_and_all() {
    let (l, _) = PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1]));
    let (l, _) = l.with_action(2, SRC, RowAction::CopyToTarget, keys(&[1]));
    let (l, _) = l.with_action(
        1,
        Category::OnlyInTarget,
        RowAction::CopyToSource,
        keys(&[5]),
    );
    assert_eq!(l.without(1).items().len(), 2);
    assert_eq!(l.without(1).items()[1].category, Category::OnlyInTarget);
    assert_eq!(l.without(99), l);
    assert_eq!(l.without_pair(1).items().len(), 1);
    assert!(l.cleared().is_empty());
}

#[test]
fn command_parts() {
    let (l, _) =
        PendingList::default().with_action(1, SRC, RowAction::DeleteFromSource, keys(&[1, 2]));
    let (l, _) = l.with_action(
        2,
        Category::Different,
        RowAction::CopyToSource,
        Scope::Category,
    );
    let (ops, bulk) = l.to_command_parts();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].pair, 1);
    assert_eq!(ops[0].key, k(1));
    assert_eq!(ops[0].kind, ApplyKind::Delete);
    assert_eq!(ops[0].direction, Direction::TargetToSource);
    assert_eq!(bulk.len(), 1);
    assert_eq!(bulk[0].pair, 2);
    assert_eq!(bulk[0].category, Category::Different);
    assert_eq!(bulk[0].kind, ApplyKind::Replace);
    assert_eq!(bulk[0].direction, Direction::TargetToSource);
}

#[test]
fn op_count_uses_totals_and_lengths() {
    let (l, _) = PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1, 2]));
    let (l, _) = l.with_action(
        2,
        Category::Different,
        RowAction::CopyToTarget,
        Scope::Category,
    );
    let totals = |pair: PairId, cat: Category| {
        assert_eq!((pair, cat), (2, Category::Different));
        500
    };
    assert_eq!(l.op_count(&totals), 502);
}

#[test]
fn compare_state_drops_selection_and_pending_with_results_and_session() {
    use crate::viewmodel::compare::{CompareState, Selection};
    let (pending, _) =
        PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1]));
    let state = CompareState {
        selection: Selection::default().toggled(1, SRC, &k(1), &[]),
        pending,
        ..CompareState::default()
    };
    let cut = state.clone().without_results(&[1]);
    assert!(cut.pending.is_empty() && cut.selection.is_empty_for(1, SRC));
    let kept = state.clone().without_results(&[2]);
    assert!(!kept.pending.is_empty() && !kept.selection.is_empty_for(1, SRC));
    let dropped = state.session_dropped();
    assert!(dropped.pending.is_empty() && dropped.selection.is_empty_for(1, SRC));
}

fn i64k(n: i64) -> KeyTuple {
    KeyTuple(vec![Bson::Int64(n)])
}

fn nan() -> KeyTuple {
    KeyTuple(vec![Bson::Double(f64::NAN)])
}

#[test]
fn compare_equal_but_structurally_different_keys_are_displaced() {
    let (l, _) = PendingList::default().with_action(
        1,
        SRC,
        RowAction::CopyToTarget,
        Scope::Keys(vec![k(1), nan()]),
    );
    let (l, _) = l.with_action(
        1,
        SRC,
        RowAction::DeleteFromSource,
        Scope::Keys(vec![i64k(1), nan()]),
    );
    assert_eq!(l.items().len(), 1, "first item emptied and dropped");
    assert_eq!(l.items()[0].action, RowAction::DeleteFromSource);
}

#[test]
fn keys_are_deduplicated_on_entry_and_ignored_counts_distinct() {
    let (l, _) = PendingList::default().with_action(
        1,
        SRC,
        RowAction::CopyToTarget,
        Scope::Keys(vec![k(1), i64k(1), k(2), k(1)]),
    );
    assert_eq!(l.items()[0].scope, Scope::Keys(vec![k(1), k(2)]));
    assert_eq!(l.op_count(&|_, _| 0), 2);
    let (c, _) =
        PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, Scope::Category);
    let (_, ignored) = c.with_action(
        1,
        SRC,
        RowAction::DeleteFromSource,
        Scope::Keys(vec![k(1), i64k(1), k(2)]),
    );
    assert_eq!(ignored, 2);
}

#[test]
fn ten_thousand_by_ten_thousand_overlap() {
    let a: Vec<KeyTuple> = (0..10_000).map(k).collect();
    let b: Vec<KeyTuple> = (5_000..15_000).map(i64k).collect();
    let (l, _) =
        PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, Scope::Keys(a));
    let (l, _) = l.with_action(1, SRC, RowAction::DeleteFromSource, Scope::Keys(b));
    assert_eq!(l.op_count(&|_, _| 0), 15_000);
}

#[test]
fn command_parts_insert_keys_and_category_delete() {
    let (l, _) = PendingList::default().with_action(1, SRC, RowAction::CopyToTarget, keys(&[1]));
    let (l, _) = l.with_action(
        2,
        Category::OnlyInTarget,
        RowAction::DeleteFromTarget,
        Scope::Category,
    );
    let (ops, bulk) = l.to_command_parts();
    assert_eq!(
        (ops[0].kind, ops[0].direction),
        (ApplyKind::Insert, Direction::SourceToTarget)
    );
    assert_eq!(bulk[0].category, Category::OnlyInTarget);
    assert_eq!(
        (bulk[0].kind, bulk[0].direction),
        (ApplyKind::Delete, Direction::SourceToTarget)
    );
}
