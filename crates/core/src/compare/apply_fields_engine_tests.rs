//! Tests for `ApplyKind::Fields` in the apply engine (real `run_pair` results, in-memory stores).

use super::tests::{
    Fixture, PAIR, all, assert_no_differences, email, k, op, src_ns, tgt_ns, total,
};
use super::*;
use crate::compare::apply::{ApplyKind, Direction};
use crate::compare::diff::{FieldChange, PathSegment, diff_normalized};
use crate::compare::key::KeySpec;
use crate::compare::normalize::normalize;
use crate::compare::rules::FieldScope;
use bson::{Bson, doc};

use ApplyKind::{Delete, Fields, Insert, Replace};
use Direction::{SourceToTarget as ST, TargetToSource as TS};

fn fld(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

fn i(n: usize) -> PathSegment {
    PathSegment::Index(n)
}

fn fields_op(key: KeyTuple, direction: Direction, paths: &[Vec<PathSegment>]) -> ApplyOp {
    ApplyOp {
        pair: PAIR,
        key,
        kind: Fields,
        direction,
        fields: paths.to_vec(),
    }
}

/// A fixture keyed on `_id` with `tweak` applied to the default rules.
async fn fixture(
    source: Vec<Document>,
    target: Vec<Document>,
    tweak: impl FnOnce(&mut CompareRules),
) -> Fixture {
    let key = KeySpec::id();
    let mut rules = CompareRules::for_key(&key);
    tweak(&mut rules);
    Fixture::build(source, target, key, rules, doc! {}).await
}

async fn plain(source: Vec<Document>, target: Vec<Document>) -> Fixture {
    fixture(source, target, |_| {}).await
}

fn changes(source: &Document, target: &Document, rules: &CompareRules) -> Vec<FieldChange> {
    let (s, t) = (
        normalize(source, rules).unwrap(),
        normalize(target, rules).unwrap(),
    );
    diff_normalized(&s, &t, rules.ignore_array_order).unwrap()
}

/// The rendered paths still differing between the first document of each side.
fn remaining(f: &Fixture) -> Vec<String> {
    changes(&f.source_docs()[0], &f.target_docs()[0], &f.spec.rules)
        .into_iter()
        .map(|c| c.path)
        .collect()
}

fn no_writes(f: &Fixture) {
    assert!(f.target.calls.lock().unwrap().is_empty());
    assert!(f.source.calls.lock().unwrap().is_empty());
}

/// One Fields op on key 1 must be refused and leave both sides untouched.
async fn assert_refused(f: &Fixture, direction: Direction, path: Vec<PathSegment>) {
    let (source, target) = (f.source_docs(), f.target_docs());
    let r = f.apply(&[fields_op(k(1), direction, &[path])]).await;
    assert_eq!((r.not_planned, r.applied, total(&r)), (1, 0, 1));
    assert_eq!((f.source_docs(), f.target_docs()), (source, target));
    no_writes(f);
}

#[tokio::test(flavor = "multi_thread")]
async fn copies_one_changed_field_and_leaves_the_rest_of_the_target_document() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "b": 1}],
        vec![doc! {"_id": 1, "a": 2, "b": 2}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("a")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1, "a": 1, "b": 2}]);
    assert_eq!(f.source_docs(), vec![doc! {"_id": 1, "a": 1, "b": 1}]);
    assert_eq!(remaining(&f), vec!["b".to_string()]);
    assert_eq!(f.recompare().await.count(Category::Different), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn copies_several_fields_of_one_document_in_one_write() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "b": 1, "c": 1}],
        vec![doc! {"_id": 1, "a": 2, "b": 2, "c": 2}],
    )
    .await;
    let r = f
        .apply(&[fields_op(k(1), ST, &[vec![fld("a")], vec![fld("b")]])])
        .await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(*f.target.calls.lock().unwrap(), vec![1]);
    assert_eq!(
        f.target_docs(),
        vec![doc! {"_id": 1, "a": 1, "b": 1, "c": 2}]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn copies_target_to_source_by_writing_the_source() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "b": 1}],
        vec![doc! {"_id": 1, "a": 2, "b": 2}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), TS, &[vec![fld("a")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.source_docs(), vec![doc! {"_id": 1, "a": 2, "b": 1}]);
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1, "a": 2, "b": 2}]);
    assert!(f.target.calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn unsets_a_field_that_exists_only_in_the_target() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1}],
        vec![doc! {"_id": 1, "a": 1, "extra": 5}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("extra")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1, "a": 1}]);
    assert_no_differences(&f.recompare().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsets_a_field_that_exists_only_in_the_source_when_copying_back() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "extra": 5}],
        vec![doc! {"_id": 1, "a": 1}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), TS, &[vec![fld("extra")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.source_docs(), vec![doc! {"_id": 1, "a": 1}]);
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1, "a": 1}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn adds_a_field_that_exists_only_in_the_source() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "n": 2}],
        vec![doc! {"_id": 1, "a": 1}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("n")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1, "a": 1, "n": 2}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn adds_a_field_that_exists_only_in_the_target_when_copying_back() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1}],
        vec![doc! {"_id": 1, "a": 1, "n": 2}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), TS, &[vec![fld("n")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.source_docs(), vec![doc! {"_id": 1, "a": 1, "n": 2}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_field_and_an_array_element_by_index() {
    let f = plain(
        vec![doc! {"_id": 1, "a": {"b": 1, "c": 1}, "arr": [1, 2, 3]}],
        vec![doc! {"_id": 1, "a": {"b": 2, "c": 2}, "arr": [1, 9, 3]}],
    )
    .await;
    let paths = [vec![fld("a"), fld("b")], vec![fld("arr"), i(1)]];
    let r = f.apply(&[fields_op(k(1), ST, &paths)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(
        f.target_docs(),
        vec![doc! {"_id": 1, "a": {"b": 1, "c": 2}, "arr": [1, 2, 3]}]
    );
    assert_eq!(remaining(&f), vec!["a.c".to_string()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_field_named_with_a_dot_is_not_planned() {
    let f = plain(
        vec![doc! {"_id": 1, "a.b": 1}],
        vec![doc! {"_id": 1, "a.b": 2}],
    )
    .await;
    assert_refused(&f, ST, vec![fld("a.b")]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_field_named_with_a_dollar_is_not_planned() {
    let f = plain(
        vec![doc! {"_id": 1, "$x": 1}],
        vec![doc! {"_id": 1, "$x": 2}],
    )
    .await;
    assert_refused(&f, ST, vec![fld("$x")]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_id_is_not_planned() {
    let key = KeySpec::new(vec!["email".into()]).unwrap();
    let mut rules = CompareRules::for_key(&key);
    rules.include_id = true;
    let f = Fixture::build(
        vec![doc! {"_id": 1, "email": "a"}],
        vec![doc! {"_id": 2, "email": "a"}],
        key,
        rules,
        doc! {},
    )
    .await;
    let found = changes(&f.source_docs()[0], &f.target_docs()[0], &f.spec.rules);
    assert!(found.iter().any(|c| c.segments == [fld("_id")]));
    let (source, target) = (f.source_docs(), f.target_docs());
    let r = f
        .apply(&[fields_op(email("a"), ST, &[vec![fld("_id")]])])
        .await;
    assert_eq!((r.not_planned, r.applied, total(&r)), (1, 0, 1));
    assert_eq!((f.source_docs(), f.target_docs()), (source, target));
    no_writes(&f);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_added_array_element_is_not_planned() {
    let f = plain(
        vec![doc! {"_id": 1, "arr": [1]}],
        vec![doc! {"_id": 1, "arr": [1, 2]}],
    )
    .await;
    let path = vec![fld("arr"), i(1)];
    let found = changes(&f.source_docs()[0], &f.target_docs()[0], &f.spec.rules);
    assert!(found.iter().any(|c| c.segments == path));
    assert_refused(&f, ST, path.clone()).await;
    assert_refused(&f, TS, path).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unordered_array_element_is_not_planned() {
    let f = fixture(
        vec![doc! {"_id": 1, "arr": [1, 2]}],
        vec![doc! {"_id": 1, "arr": [1, 3]}],
        |r| r.ignore_array_order = true,
    )
    .await;
    let path = vec![fld("arr"), PathSegment::AnyElement];
    let found = changes(&f.source_docs()[0], &f.target_docs()[0], &f.spec.rules);
    assert!(found.iter().any(|c| c.segments == path), "{found:?}");
    assert_refused(&f, ST, path).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_is_not_a_real_change_is_not_planned() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "b": 1}],
        vec![doc! {"_id": 1, "a": 1, "b": 2}],
    )
    .await;
    assert_refused(&f, ST, vec![fld("a")]).await;
    assert_refused(&f, ST, vec![fld("nowhere")]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn conflicting_paths_are_not_planned() {
    let f = plain(
        vec![doc! {"_id": 1, "a": {"b": 1}}],
        vec![doc! {"_id": 1, "a": {"b": 2}}],
    )
    .await;
    let (source, target) = (f.source_docs(), f.target_docs());
    for paths in [
        vec![vec![fld("a")], vec![fld("a"), fld("b")]],
        vec![vec![fld("a"), fld("b")], vec![fld("a"), fld("b")]],
    ] {
        let r = f.apply(&[fields_op(k(1), ST, &paths)]).await;
        assert_eq!((r.not_planned, r.applied, total(&r)), (1, 0, 1));
    }
    assert_eq!((f.source_docs(), f.target_docs()), (source, target));
    no_writes(&f);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_changed_since_the_compare_is_stale_for_fields_too() {
    for on_target in [true, false] {
        let f = plain(
            vec![doc! {"_id": 1, "a": 1, "b": 1}],
            vec![doc! {"_id": 1, "a": 2, "b": 2}],
        )
        .await;
        let changed = doc! {"_id": 1, "a": 2, "b": 99};
        if on_target {
            f.target_inner
                .replace(&tgt_ns(), &Bson::Int32(1), changed.clone())
                .await
                .unwrap();
        } else {
            f.source_inner
                .replace(&src_ns(), &Bson::Int32(1), changed.clone())
                .await
                .unwrap();
        }
        let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("a")]])]).await;
        assert_eq!((r.stale, r.applied, total(&r)), (1, 0, 1), "{on_target}");
        no_writes(&f);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fields_on_a_row_that_exists_on_one_side_only_are_not_planned() {
    let f = plain(vec![doc! {"_id": 1, "a": 1}], vec![doc! {"_id": 2, "a": 1}]).await;
    for (key, direction) in [(k(1), ST), (k(1), TS), (k(2), ST), (k(2), TS)] {
        let r = f
            .apply(&[fields_op(key, direction, &[vec![fld("a")]])])
            .await;
        assert_eq!((r.not_planned, r.applied, total(&r)), (1, 0, 1));
    }
    no_writes(&f);
    assert_eq!(f.target_docs(), vec![doc! {"_id": 2, "a": 1}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn values_come_from_the_original_document_not_the_normalized_one() {
    let f = fixture(
        vec![doc! {"_id": 1, "z": 5_i32}],
        vec![doc! {"_id": 1, "z": 6_i64}],
        |r| r.numbers_equal_across_types = true,
    )
    .await;
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("z")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.target_docs()[0].get("z"), Some(&Bson::Int32(5)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copied_subtree_keeps_the_origin_field_order() {
    let f = fixture(
        vec![doc! {"_id": 1, "d": {"b": 1, "a": 2}}],
        vec![doc! {"_id": 1, "d": 7}],
        |r| r.ignore_field_order = true,
    )
    .await;
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("d")]])]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    let d = f.target_docs()[0].get_document("d").unwrap().clone();
    assert_eq!(d.keys().collect::<Vec<_>>(), ["b", "a"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn field_scope_only_never_writes_outside_the_scope() {
    let f = fixture(
        vec![doc! {"_id": 1, "meta": {"owner": "o", "sync": "s"}, "out": 1}],
        vec![doc! {"_id": 1, "meta": {"owner": "x", "sync": "t"}, "out": 2}],
        |r| r.field_scope = FieldScope::Only(vec!["meta.owner".into()]),
    )
    .await;
    assert_refused(&f, ST, vec![fld("out")]).await;
    assert_refused(&f, ST, vec![fld("meta")]).await;
    let r = f
        .apply(&[fields_op(k(1), ST, &[vec![fld("meta"), fld("owner")]])])
        .await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(
        f.target_docs(),
        vec![doc! {"_id": 1, "meta": {"owner": "o", "sync": "t"}, "out": 2}]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_that_shifts_array_indexes_is_not_planned() {
    let path = vec![fld("items"), i(1), fld("price")];
    let only = |r: &mut CompareRules| r.field_scope = FieldScope::Only(vec!["items.price".into()]);
    let cases = [
        (
            doc! {"_id": 1, "items": [5, {"price": 1}, {"price": 2}]},
            doc! {"_id": 1, "items": [{"price": 1}, {"price": 3}]},
        ),
        (
            doc! {"_id": 1, "items": [{"price": 3}, {"price": 5}, {"price": 4}]},
            doc! {"_id": 1, "items": [7, {"price": 3}, {"price": 3}, {"price": 4}]},
        ),
    ];
    for (source, target) in cases {
        for direction in [ST, TS] {
            let f = fixture(vec![source.clone()], vec![target.clone()], only).await;
            let found = changes(&f.source_docs()[0], &f.target_docs()[0], &f.spec.rules);
            assert!(found.iter().any(|c| c.segments == path), "{found:?}");
            assert_refused(&f, direction, path.clone()).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subtree_missing_on_the_destination_is_not_planned_under_a_narrow_scope() {
    let f = fixture(
        vec![doc! {"_id": 1, "meta": {"owner": "o", "sync": "s"}}],
        vec![doc! {"_id": 1}],
        |r| r.field_scope = FieldScope::Only(vec!["meta.owner".into()]),
    )
    .await;
    assert_refused(&f, ST, vec![fld("meta")]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn applying_the_fields_then_comparing_again_shows_only_the_unchosen_differences() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "b": 1, "c": 1}],
        vec![doc! {"_id": 1, "a": 2, "b": 2, "c": 2}],
    )
    .await;
    let r = f
        .apply(&[fields_op(k(1), ST, &[vec![fld("a")], vec![fld("c")]])])
        .await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(remaining(&f), vec!["b".to_string()]);
    let again = f.recompare().await;
    assert_eq!(again.count(Category::Different), 1);
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("b")]])]).await;
    assert_eq!(r.applied, 0, "the stale results no longer match");
    assert_eq!(r.stale, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_unchosen_field_applied_on_fresh_results_leaves_no_difference() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1, "b": 1}],
        vec![doc! {"_id": 1, "a": 1, "b": 2}],
    )
    .await;
    let r = f.apply(&[fields_op(k(1), ST, &[vec![fld("b")]])]).await;
    assert_eq!(r.applied, 1);
    assert_no_differences(&f.recompare().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fields_op_without_fields_and_a_replace_with_fields_are_not_planned() {
    let f = plain(
        vec![doc! {"_id": 1, "a": 1}, doc! {"_id": 2, "a": 1}],
        vec![doc! {"_id": 1, "a": 2}, doc! {"_id": 2, "a": 2}],
    )
    .await;
    let mut replace_with_fields = op(k(2), Replace, ST);
    replace_with_fields.fields = vec![vec![fld("a")]];
    let ops = [fields_op(k(1), ST, &[]), replace_with_fields];
    let r = f.apply(&ops).await;
    assert_eq!((r.not_planned, r.applied, total(&r)), (2, 0, 2));
    no_writes(&f);
}

#[tokio::test(flavor = "multi_thread")]
async fn mixed_whole_document_and_field_ops_keep_the_accounting() {
    let f = plain(
        vec![
            doc! {"_id": 1, "v": 1},
            doc! {"_id": 2, "a": 1, "b": 1},
            doc! {"_id": 3},
            doc! {"_id": 4, "a": 1, "b": 1},
            doc! {"_id": 5, "a": 1, "b": 1},
        ],
        vec![
            doc! {"_id": 1, "v": 2},
            doc! {"_id": 2, "a": 2, "b": 2},
            doc! {"_id": 4, "a": 2, "b": 2},
            doc! {"_id": 5, "a": 2, "b": 2},
            doc! {"_id": 6},
        ],
    )
    .await;
    f.target_inner
        .replace(&tgt_ns(), &Bson::Int32(4), doc! {"_id": 4, "a": 9})
        .await
        .unwrap();
    let ops = [
        op(k(1), Replace, ST),
        fields_op(k(2), ST, &[vec![fld("a")]]),
        op(k(3), Insert, ST),
        fields_op(k(4), ST, &[vec![fld("a")]]),
        fields_op(k(5), ST, &[vec![fld("nowhere")]]),
        op(k(6), Delete, ST),
    ];
    let r = f.apply(&ops).await;
    assert_eq!(total(&r), ops.len() as u64);
    assert_eq!(
        (r.applied, r.stale, r.not_planned, r.failed, r.not_run),
        (4, 1, 1, 0, 0)
    );
    let target = all(&f.target_inner, &tgt_ns());
    assert!(target.contains(&doc! {"_id": 2, "a": 1, "b": 2}));
    assert!(target.contains(&doc! {"_id": 5, "a": 2, "b": 2}));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fields_op_next_to_a_replace_on_the_same_key_runs_only_the_first() {
    for fields_first in [true, false] {
        let f = plain(
            vec![doc! {"_id": 1, "a": 1, "b": 1}],
            vec![doc! {"_id": 1, "a": 2, "b": 2}],
        )
        .await;
        let fields = fields_op(k(1), ST, &[vec![fld("a")]]);
        let replace = op(k(1), Replace, ST);
        let ops = if fields_first {
            [fields, replace]
        } else {
            [replace, fields]
        };
        let r = f.apply(&ops).await;
        assert_eq!((r.applied, r.not_planned, total(&r)), (1, 1, 2));
        let want = if fields_first {
            doc! {"_id": 1, "a": 1, "b": 2}
        } else {
            doc! {"_id": 1, "a": 1, "b": 1}
        };
        assert_eq!(f.target_docs(), vec![want]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_between_batches_with_fields_ops_reports_not_run() {
    let source = (0..1201).map(|n| doc! {"_id": n, "a": 1}).collect();
    let target = (0..1201).map(|n| doc! {"_id": n, "a": 2}).collect();
    let f = plain(source, target).await;
    let ops: Vec<_> = (0..1201)
        .map(|n| fields_op(k(n), ST, &[vec![fld("a")]]))
        .collect();
    let cancel = CancelFlag::default();
    let c = cancel.clone();
    let r = apply_pair(f.input(&ops), &cancel, &mut |_| c.cancel())
        .await
        .unwrap();
    assert_eq!((r.applied, r.not_run, total(&r)), (500, 701, 1201));
    assert_eq!(*f.target.calls.lock().unwrap(), vec![500]);
}
