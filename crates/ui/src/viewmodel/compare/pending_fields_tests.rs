use super::super::pending_fields::FieldsQueued;
use super::*;
use bson::Bson;
use curlybrace_core::compare::diff::PathSegment;

const DIFF: Category = Category::Different;
const TO_T: RowAction = RowAction::CopyFieldsToTarget;
const TO_S: RowAction = RowAction::CopyFieldsToSource;

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn nan() -> KeyTuple {
    KeyTuple(vec![Bson::Double(f64::NAN)])
}

fn p(name: &str) -> Vec<PathSegment> {
    vec![PathSegment::Field(name.into())]
}

fn ps(names: &[&str]) -> Vec<Vec<PathSegment>> {
    names.iter().map(|n| p(n)).collect()
}

fn fields_scope(key: KeyTuple, names: &[&str]) -> Scope {
    Scope::Fields {
        key,
        paths: ps(names),
    }
}

fn whole(keys: &[i32]) -> Scope {
    Scope::Keys(keys.iter().map(|n| k(*n)).collect())
}

fn queued(list: &PendingList, key: KeyTuple, action: RowAction, names: &[&str]) -> PendingList {
    list.with_fields(1, &key, action, ps(names)).0
}

#[test]
fn a_first_selection_is_queued_as_a_fields_item() {
    let (l, out) = PendingList::default().with_fields(1, &k(1), TO_T, ps(&["a", "b"]));
    assert_eq!(out, FieldsQueued::Queued);
    assert_eq!(l.items().len(), 1);
    let item = &l.items()[0];
    assert_eq!((item.pair, item.category, item.action), (1, DIFF, TO_T));
    assert_eq!(item.scope, fields_scope(k(1), &["a", "b"]));
}

#[test]
fn duplicate_incoming_paths_are_dropped_keeping_first_seen_order() {
    let (l, out) = PendingList::default().with_fields(1, &k(1), TO_T, ps(&["b", "a", "b"]));
    assert_eq!(out, FieldsQueued::Queued);
    assert_eq!(l.items()[0].scope, fields_scope(k(1), &["b", "a"]));
}

#[test]
fn same_key_and_direction_merges_paths_in_place_without_repeats() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a", "b"]);
    let l = queued(&l, k(2), TO_T, &["x"]);
    let (l, out) = l.with_fields(1, &k(1), TO_T, ps(&["b", "c"]));
    assert_eq!(out, FieldsQueued::Merged);
    assert_eq!(l.items().len(), 2);
    assert_eq!(l.items()[0].scope, fields_scope(k(1), &["a", "b", "c"]));
    assert_eq!(l.items()[1].scope, fields_scope(k(2), &["x"]));
}

#[test]
fn a_union_that_conflicts_is_refused_and_the_list_is_unchanged() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a"]);
    let nested = vec![vec![
        PathSegment::Field("a".into()),
        PathSegment::Field("b".into()),
    ]];
    let (same, out) = l.with_fields(1, &k(1), TO_T, nested);
    assert_eq!(out, FieldsQueued::Conflict);
    assert_eq!(same, l);
}

#[test]
fn incoming_paths_that_conflict_among_themselves_are_refused() {
    let both = vec![
        p("a"),
        vec![
            PathSegment::Field("a".into()),
            PathSegment::Field("b".into()),
        ],
    ];
    let (l, out) = PendingList::default().with_fields(1, &k(1), TO_T, both);
    assert_eq!(out, FieldsQueued::Conflict);
    assert!(l.is_empty());
}

#[test]
fn no_paths_is_refused() {
    let (l, out) = PendingList::default().with_fields(1, &k(1), TO_T, Vec::new());
    assert_eq!(out, FieldsQueued::Empty);
    assert!(l.is_empty());
}

#[test]
fn the_other_direction_replaces_the_item_and_keeps_only_the_new_paths() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a", "b"]);
    let l = queued(&l, k(2), TO_T, &["x"]);
    let (l, out) = l.with_fields(1, &k(1), TO_S, ps(&["c"]));
    assert_eq!(out, FieldsQueued::Queued);
    assert_eq!(l.items().len(), 2);
    assert_eq!(l.items()[0].scope, fields_scope(k(2), &["x"]));
    assert_eq!(l.items()[1].action, TO_S);
    assert_eq!(l.items()[1].scope, fields_scope(k(1), &["c"]));
}

#[test]
fn fields_replace_a_whole_document_change_for_the_same_key_only() {
    let (l, _) =
        PendingList::default().with_action(1, DIFF, RowAction::CopyToTarget, whole(&[1, 2]));
    let (l, out) = l.with_fields(1, &k(1), TO_T, ps(&["a"]));
    assert_eq!(out, FieldsQueued::Queued);
    assert_eq!(l.items().len(), 2);
    assert_eq!(l.items()[0].scope, whole(&[2]));
    assert_eq!(l.items()[1].scope, fields_scope(k(1), &["a"]));
}

#[test]
fn an_emptied_whole_document_item_is_dropped() {
    let (l, _) = PendingList::default().with_action(1, DIFF, RowAction::CopyToSource, whole(&[1]));
    let l = queued(&l, k(1), TO_T, &["a"]);
    assert_eq!(l.items().len(), 1);
    assert_eq!(l.items()[0].scope, fields_scope(k(1), &["a"]));
}

#[test]
fn a_whole_document_change_replaces_fields_for_the_same_key_only() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a"]);
    let l = queued(&l, k(2), TO_T, &["b"]);
    let (l, ignored) = l.with_action(1, DIFF, RowAction::CopyToTarget, whole(&[1, 3]));
    assert_eq!(ignored, 0);
    assert_eq!(l.items().len(), 2);
    assert_eq!(l.items()[0].scope, fields_scope(k(2), &["b"]));
    assert_eq!(l.items()[1].scope, whole(&[1, 3]));
}

#[test]
fn whole_document_changes_of_other_categories_leave_fields_alone() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a"]);
    let (l2, _) = l.with_action(
        1,
        Category::OnlyInSource,
        RowAction::CopyToTarget,
        whole(&[1]),
    );
    assert_eq!(l2.items()[0], l.items()[0]);
    assert_eq!(l2.items().len(), 2);
}

#[test]
fn a_category_item_replaces_fields_items_of_its_tab() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a"]);
    let (l, _) = l.with_action(1, DIFF, RowAction::CopyToSource, Scope::Category);
    assert_eq!(l.items().len(), 1);
    assert_eq!(l.items()[0].scope, Scope::Category);
}

#[test]
fn a_category_item_swallows_a_fields_request() {
    let (l, _) =
        PendingList::default().with_action(1, DIFF, RowAction::CopyToTarget, Scope::Category);
    let (same, out) = l.with_fields(1, &k(1), TO_T, ps(&["a"]));
    assert_eq!(out, FieldsQueued::Swallowed);
    assert_eq!(same, l);
    // another pair is not covered
    let (other, out) = l.with_fields(2, &k(1), TO_T, ps(&["a"]));
    assert_eq!(out, FieldsQueued::Queued);
    assert_eq!(other.items().len(), 2);
}

#[test]
fn a_nan_key_merges_with_its_own_clone() {
    let l = queued(&PendingList::default(), nan(), TO_T, &["a"]);
    let (l, out) = l.with_fields(1, &nan(), TO_T, ps(&["b"]));
    assert_eq!(out, FieldsQueued::Merged);
    assert_eq!(l.items().len(), 1);
    // `==` on a NaN key is false by IEEE rules, so look at the paths.
    let Scope::Fields { paths, .. } = &l.items()[0].scope else {
        panic!("a fields item");
    };
    assert_eq!(*paths, ps(&["a", "b"]));
}

#[test]
fn int32_and_int64_keys_of_one_value_are_one_document() {
    let wide = KeyTuple(vec![Bson::Int64(1)]);
    let l = queued(&PendingList::default(), k(1), TO_T, &["a"]);
    let (l, out) = l.with_fields(1, &wide, TO_T, ps(&["b"]));
    assert_eq!(out, FieldsQueued::Merged);
    assert_eq!(l.items().len(), 1);
}

#[test]
fn same_changes_ignores_nan_identity_and_path_order() {
    let a = queued(&PendingList::default(), nan(), TO_T, &["a", "b"]);
    let b = queued(&PendingList::default(), nan(), TO_T, &["b", "a"]);
    assert!(a.same_changes(&b));
    assert!(a.same_changes(&a.clone()));
}

#[test]
fn same_changes_tells_paths_keys_and_directions_apart() {
    let base = queued(&PendingList::default(), k(1), TO_T, &["a", "b"]);
    assert!(!base.same_changes(&queued(&PendingList::default(), k(1), TO_T, &["a", "c"])));
    assert!(!base.same_changes(&queued(&PendingList::default(), k(1), TO_T, &["a"])));
    assert!(!base.same_changes(&queued(&PendingList::default(), k(2), TO_T, &["a", "b"])));
    assert!(!base.same_changes(&queued(&PendingList::default(), k(1), TO_S, &["a", "b"])));
    let (whole_doc, _) =
        PendingList::default().with_action(1, DIFF, RowAction::CopyToTarget, whole(&[1]));
    assert!(!base.same_changes(&whole_doc));
}

#[test]
fn command_parts_emit_one_fields_op_per_document_and_no_bulk() {
    let l = queued(&PendingList::default(), k(1), TO_S, &["a", "b"]);
    let (l, _) = l.with_action(1, DIFF, RowAction::CopyToTarget, whole(&[2]));
    let (ops, bulk) = l.to_command_parts();
    assert!(bulk.is_empty());
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].kind, ApplyKind::Fields);
    assert_eq!(ops[0].direction, Direction::TargetToSource);
    assert_eq!(ops[0].key, k(1));
    assert_eq!(ops[0].fields, ps(&["a", "b"]));
    assert_eq!(ops[1].kind, ApplyKind::Replace);
    assert!(ops[1].fields.is_empty());
}

#[test]
fn a_fields_item_counts_as_one_write() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a", "b", "c"]);
    let l = queued(&l, k(2), TO_T, &["a"]);
    assert_eq!(l.op_count(&|_, _| 99), 2);
}

#[test]
fn removing_a_pair_removes_its_fields_items() {
    let l = queued(&PendingList::default(), k(1), TO_T, &["a"]);
    assert!(l.without_pair(1).is_empty());
    assert_eq!(l.without_pair(2), l);
}
