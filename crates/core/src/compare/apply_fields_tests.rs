use super::*;
use crate::compare::diff::{diff_normalized, render_path};
use crate::compare::key::KeySpec;
use crate::compare::normalize::normalize;
use crate::compare::rules::CompareRules;
use Direction::{SourceToTarget as ST, TargetToSource as TS};
use bson::doc;

fn f(n: &str) -> PathSegment {
    PathSegment::Field(n.into())
}
fn i(n: usize) -> PathSegment {
    PathSegment::Index(n)
}
fn change(segments: Vec<PathSegment>, kind: ChangeKind) -> FieldChange {
    FieldChange {
        path: render_path(&segments),
        segments,
        kind,
        source: None,
        target: None,
    }
}

fn origin() -> Document {
    doc! { "_id": 1, "a": { "b": 5_i32, "arr": [1, 2, 3] }, "n": 7_i32, "": 1, "x.y": 2, "$z": 3 }
}

/// One row per rule: (segments, kind, direction, expected).
#[allow(clippy::type_complexity)]
fn table() -> Vec<(
    Vec<PathSegment>,
    ChangeKind,
    Direction,
    std::result::Result<&'static str, Unaddressable>,
)> {
    use ChangeKind::{Added, Changed, Removed};
    use Unaddressable::*;
    let n = || vec![f("n")];
    let ab = || vec![f("a"), f("b")];
    let idx = || vec![f("a"), f("arr"), i(1)];
    vec![
        (n(), Changed, ST, Ok("set")),
        (n(), Changed, TS, Ok("set")),
        (n(), Removed, ST, Ok("set")),
        (n(), Removed, TS, Ok("unset")),
        (n(), Added, ST, Ok("unset")),
        (n(), Added, TS, Ok("set")),
        (ab(), Changed, ST, Ok("set")),
        (ab(), Removed, ST, Ok("set")),
        (ab(), Added, TS, Ok("set")),
        (vec![f("_id")], Changed, ST, Err(Id)),
        (vec![f("_id")], Added, TS, Err(Id)),
        (vec![f("_id"), f("k")], Changed, ST, Err(Id)),
        (
            vec![f("a"), PathSegment::AnyElement],
            Changed,
            ST,
            Err(AnyElement),
        ),
        (
            vec![f("a"), PathSegment::AnyElement, f("k")],
            Removed,
            TS,
            Err(AnyElement),
        ),
        (vec![f("")], Changed, ST, Err(BadName)),
        (vec![f("x.y")], Changed, TS, Err(BadName)),
        (vec![f("$z")], Removed, ST, Err(BadName)),
        (vec![f("a"), f("$z")], Added, TS, Err(BadName)),
        (idx(), Changed, ST, Ok("set")),
        (idx(), Changed, TS, Ok("set")),
        (idx(), Added, ST, Err(ArrayElement)),
        (idx(), Added, TS, Err(ArrayElement)),
        (idx(), Removed, ST, Err(ArrayElement)),
        (idx(), Removed, TS, Err(ArrayElement)),
        (
            vec![f("a"), f("arr"), i(0), f("k")],
            Added,
            TS,
            Err(ArrayElement),
        ),
        (
            vec![f("a"), f("arr"), i(0), f("k")],
            Removed,
            ST,
            Err(ArrayElement),
        ),
        (
            vec![f("a"), f("arr"), i(0), f("k")],
            Changed,
            ST,
            Err(NotAChange),
        ),
        (Vec::new(), Changed, ST, Err(NotAChange)),
    ]
}

#[test]
fn can_copy_and_field_write_agree_on_every_table_row() {
    let o = doc! { "_id": 1, "a": { "b": 5_i32, "arr": [1, 2, 3] }, "n": 7_i32, "": 1, "x.y": 2, "$z": 3 };
    for (segments, kind, dir, want) in table() {
        let c = change(segments.clone(), kind);
        let got = field_write(&c, dir, &o);
        let reason = can_copy(&c, dir);
        // The expected can_copy per row: the rule reasons, except that a missing origin
        // value (NotAChange from the value lookup) is not visible without the origin.
        let origin_dependent =
            segments == [f("a"), f("arr"), i(0), f("k")] && kind == ChangeKind::Changed;
        let want_can = match &want {
            _ if origin_dependent => None,
            Ok(_) => None,
            Err(r) => Some(*r),
        };
        assert_eq!(reason, want_can, "can_copy {:?} {kind:?} {dir:?}", c.path);
        let label = format!("{:?} {kind:?} {dir:?}", c.path);
        match (want, got) {
            (Ok("set"), Ok(FieldWrite::Set(s, _))) => assert_eq!(s, segments, "{label}"),
            (Ok("unset"), Ok(FieldWrite::Unset(s))) => assert_eq!(s, segments, "{label}"),
            (Err(Unaddressable::NotAChange), Err(r)) => {
                assert_eq!(r, Unaddressable::NotAChange, "{label}")
            }
            (Err(want), Err(r)) => {
                assert_eq!(r, want, "{label}");
                assert_eq!(reason, Some(want), "{label}");
            }
            (want, got) => panic!("{label}: want {want:?}, got {got:?}"),
        }
        if field_write(&c, dir, &o).is_ok() {
            assert_eq!(reason, None, "{label}");
        }
    }
}

#[test]
fn can_copy_is_origin_free_so_a_missing_value_is_still_copyable() {
    let c = change(vec![f("gone")], ChangeKind::Changed);
    assert_eq!(can_copy(&c, ST), None);
    assert_eq!(
        field_write(&c, ST, &doc! { "other": 1 }),
        Err(Unaddressable::NotAChange)
    );
}

#[test]
fn field_write_reads_the_original_origin_value_not_the_diff_value() {
    let mut c = change(vec![f("n")], ChangeKind::Changed);
    c.source = Some(Bson::Int64(7)); // normalized
    let w = field_write(&c, ST, &origin()).unwrap();
    assert_eq!(w, FieldWrite::Set(vec![f("n")], Bson::Int32(7)));
    let w = field_write(&c, TS, &doc! { "n": 3.5 }).unwrap();
    assert_eq!(w, FieldWrite::Set(vec![f("n")], Bson::Double(3.5)));
}

#[test]
fn a_set_of_an_array_element_takes_the_origin_element() {
    let c = change(vec![f("a"), f("arr"), i(2)], ChangeKind::Changed);
    let w = field_write(&c, ST, &origin()).unwrap();
    assert_eq!(w, FieldWrite::Set(c.segments.clone(), Bson::Int32(3)));
}

#[test]
fn an_unset_needs_no_value_from_the_origin() {
    let c = change(vec![f("n")], ChangeKind::Added);
    let w = field_write(&c, ST, &doc! {}).unwrap();
    assert_eq!(w, FieldWrite::Unset(vec![f("n")]));
}

#[test]
fn i18n_keys_are_distinct_literals() {
    use Unaddressable::*;
    let keys: Vec<_> = [Id, AnyElement, BadName, ArrayElement, NotAChange]
        .map(Unaddressable::i18n_key)
        .to_vec();
    assert_eq!(
        keys,
        [
            "compare.apply.field_reason.id",
            "compare.apply.field_reason.any_element",
            "compare.apply.field_reason.bad_name",
            "compare.apply.field_reason.array_element",
            "compare.apply.field_reason.not_a_change",
        ]
    );
}

#[test]
fn value_at_walks_documents_and_arrays() {
    let d = doc! { "a": { "arr": [ { "k": 1_i32 }, 20_i32 ] }, "n": 7_i32 };
    assert_eq!(value_at(&d, &[f("n")]), Some(&Bson::Int32(7)));
    assert_eq!(
        value_at(&d, &[f("a"), f("arr"), i(0), f("k")]),
        Some(&Bson::Int32(1))
    );
    assert_eq!(
        value_at(&d, &[f("a"), f("arr"), i(1)]),
        Some(&Bson::Int32(20))
    );
    assert!(matches!(value_at(&d, &[f("a")]), Some(Bson::Document(_))));
}

#[test]
fn value_at_is_none_for_absent_or_mistyped_paths() {
    let d = doc! { "a": { "arr": [1] }, "n": 7 };
    assert_eq!(value_at(&d, &[f("zz")]), None);
    assert_eq!(value_at(&d, &[f("a"), f("arr"), i(5)]), None);
    assert_eq!(value_at(&d, &[f("a"), i(0)]), None);
    assert_eq!(value_at(&d, &[f("n"), f("deeper")]), None);
    assert_eq!(
        value_at(&d, &[f("a"), f("arr"), PathSegment::AnyElement]),
        None
    );
    assert_eq!(value_at(&d, &[]), None);
}

#[test]
fn value_at_keeps_the_original_numeric_type() {
    let d = doc! { "n": 5_i32 };
    assert_eq!(value_at(&d, &[f("n")]), Some(&Bson::Int32(5)));
}

#[test]
fn paths_conflict_flags_duplicates_and_prefixes_only() {
    let p = |s: &[PathSegment]| s.to_vec();
    assert!(paths_conflict(&[p(&[f("a")]), p(&[f("a")])]));
    assert!(paths_conflict(&[p(&[f("a")]), p(&[f("a"), f("b")])]));
    assert!(paths_conflict(&[p(&[f("a"), f("b")]), p(&[f("a")])]));
    assert!(paths_conflict(&[p(&[f("a"), i(0)]), p(&[f("a")])]));
    assert!(!paths_conflict(&[p(&[f("a")]), p(&[f("b")])]));
    assert!(!paths_conflict(&[
        p(&[f("a"), f("b")]),
        p(&[f("a"), f("c")])
    ]));
    assert!(!paths_conflict(&[p(&[f("a"), i(0)]), p(&[f("a"), i(1)])]));
    assert!(!paths_conflict(&[p(&[f("ab")]), p(&[f("a")])]));
    assert!(!paths_conflict(&[]));
    assert!(!paths_conflict(&[p(&[f("a")])]));
}

fn changes() -> Vec<FieldChange> {
    vec![
        change(vec![f("n")], ChangeKind::Changed),
        change(vec![f("only_src")], ChangeKind::Removed),
        change(vec![f("only_tgt")], ChangeKind::Added),
        change(vec![f("a"), f("arr"), i(1)], ChangeKind::Changed),
        change(vec![f("_id")], ChangeKind::Changed),
        change(vec![f("a"), f("arr"), i(9)], ChangeKind::Added),
    ]
}

fn src() -> Document {
    doc! { "_id": 1, "n": 7_i32, "only_src": "s", "a": { "arr": [1, 2, 3] } }
}

#[test]
fn plan_fields_builds_set_and_unset_in_request_order() {
    let paths = vec![
        vec![f("only_tgt")],
        vec![f("n")],
        vec![f("a"), f("arr"), i(1)],
        vec![f("only_src")],
    ];
    let op = plan_fields(&paths, &changes(), ST, &src(), &dest(42), &FieldScope::All).unwrap();
    assert_eq!(
        op,
        WriteOp::UpdateFields {
            id: Bson::Int32(42),
            set: vec![
                (vec![f("n")], Bson::Int32(7)),
                (vec![f("a"), f("arr"), i(1)], Bson::Int32(2)),
                (vec![f("only_src")], Bson::String("s".into())),
            ],
            unset: vec![vec![f("only_tgt")]],
        }
    );
}

#[test]
fn plan_fields_pins_the_direction_of_added_and_removed() {
    let tgt = doc! { "_id": 9, "only_tgt": "t" };
    let op = plan_fields(
        &[vec![f("only_tgt")]],
        &changes(),
        TS,
        &tgt,
        &tgt,
        &FieldScope::All,
    )
    .unwrap();
    assert_eq!(
        op,
        WriteOp::UpdateFields {
            id: Bson::Int32(9),
            set: vec![(vec![f("only_tgt")], Bson::String("t".into()))],
            unset: vec![],
        }
    );
    let op = plan_fields(
        &[vec![f("only_src")]],
        &changes(),
        TS,
        &tgt,
        &tgt,
        &FieldScope::All,
    )
    .unwrap();
    assert_eq!(
        op,
        WriteOp::UpdateFields {
            id: Bson::Int32(9),
            set: vec![],
            unset: vec![vec![f("only_src")]],
        }
    );
}

fn invalid() -> CoreError {
    CoreError::InvalidDocument(String::new())
}

#[test]
fn plan_fields_refuses_empty_conflicting_unknown_and_unaddressable_paths() {
    let d = dest(1);
    let plan = |paths: &[Vec<PathSegment>]| {
        plan_fields(paths, &changes(), ST, &src(), &d, &FieldScope::All)
    };
    assert_eq!(plan(&[]), Err(invalid()));
    assert_eq!(plan(&[vec![f("n")], vec![f("n")]]), Err(invalid()));
    assert_eq!(
        plan(&[vec![f("a")], vec![f("a"), f("arr"), i(1)]]),
        Err(invalid())
    );
    assert_eq!(plan(&[vec![f("not_a_change")]]), Err(invalid()));
    assert_eq!(
        plan(&[vec![f("n")], vec![f("not_a_change")]]),
        Err(invalid())
    );
    assert_eq!(plan(&[vec![f("_id")]]), Err(invalid()));
    assert_eq!(plan(&[vec![f("a"), f("arr"), i(9)]]), Err(invalid()));
    assert_eq!(plan(&[Vec::new()]), Err(invalid()));
}

#[test]
fn plan_fields_refuses_a_set_whose_origin_value_is_gone() {
    let origin = doc! { "_id": 1 };
    let r = plan_fields(
        &[vec![f("n")]],
        &changes(),
        ST,
        &origin,
        &dest(1),
        &FieldScope::All,
    );
    assert_eq!(r, Err(invalid()));
}

#[test]
fn plan_fields_refuses_a_path_reported_twice() {
    let mut two = changes();
    two.push(change(vec![f("n")], ChangeKind::Changed));
    let r = plan_fields(
        &[vec![f("n")]],
        &two,
        ST,
        &src(),
        &dest(1),
        &FieldScope::All,
    );
    assert_eq!(r, Err(invalid()));
}

#[test]
fn the_error_text_carries_no_values() {
    let origin = doc! { "_id": 1, "secret": "hunter2-distinctive" };
    let ch = vec![change(vec![f("secret")], ChangeKind::Changed)];
    let scope = FieldScope::Only(vec!["other".into()]);
    for bad in [
        plan_fields(
            &[vec![f("nope")]],
            &ch,
            ST,
            &origin,
            &dest(1),
            &FieldScope::All,
        ),
        plan_fields(&[vec![f("secret")]], &ch, ST, &origin, &dest(1), &scope),
    ] {
        let text = format!("{:?} {}", bad.clone().unwrap_err(), bad.unwrap_err());
        assert!(
            !text.contains("secret") && !text.contains("hunter2"),
            "{text}"
        );
    }
}

fn dest(id: i32) -> Document {
    doc! { "_id": id }
}

/// The fresh diff under `rules`, as the engine computes it.
fn diff_under(src: &Document, tgt: &Document, scope: &FieldScope) -> Vec<FieldChange> {
    let mut rules = CompareRules::for_key(&KeySpec::new(vec!["_id".into()]).unwrap());
    rules.field_scope = scope.clone();
    let (s, t) = (
        normalize(src, &rules).unwrap(),
        normalize(tgt, &rules).unwrap(),
    );
    diff_normalized(&s, &t, false).unwrap()
}

fn plan_with(
    scope: &FieldScope,
    src: &Document,
    tgt: &Document,
    paths: &[Vec<PathSegment>],
) -> Result<WriteOp> {
    let ch = diff_under(src, tgt, scope);
    plan_fields(paths, &ch, ST, src, tgt, scope)
}

#[test]
fn a_scope_that_shifts_array_indexes_is_refused() {
    // Only(items.price) drops the scalar 5, so the diff's items[1] is original items[2].
    let scope = FieldScope::Only(vec!["items.price".into()]);
    let src = doc! { "_id": 1, "items": [5, { "price": 1 }, { "price": 2 }] };
    let tgt = doc! { "_id": 1, "items": [{ "price": 1 }, { "price": 3 }] };
    let path = vec![f("items"), i(1), f("price")];
    assert!(
        diff_under(&src, &tgt, &scope)
            .iter()
            .any(|c| c.segments == path)
    );
    assert_eq!(plan_with(&scope, &src, &tgt, &[path]), Err(invalid()));
}

#[test]
fn all_except_keeps_array_positions_so_an_untouched_leaf_is_planned() {
    let scope = FieldScope::AllExcept(vec!["items.skip".into()]);
    let src = doc! { "_id": 1, "items": [{ "skip": 1, "price": 1 }, { "price": 2 }] };
    let tgt = doc! { "_id": 1, "items": [{ "skip": 9, "price": 5 }, { "price": 2 }] };
    let path = vec![f("items"), i(0), f("price")];
    let op = plan_with(&scope, &src, &tgt, &[path.clone()]).unwrap();
    let WriteOp::UpdateFields { set, .. } = op else {
        panic!("expected UpdateFields")
    };
    assert_eq!(set, vec![(path, Bson::Int32(1))]);
}

#[test]
fn all_except_refuses_a_changed_value_that_hides_an_out_of_scope_field() {
    let scope = FieldScope::AllExcept(vec!["items.skip".into()]);
    let src = doc! { "_id": 1, "items": [5] };
    let tgt = doc! { "_id": 1, "items": [{ "skip": 1, "price": 1 }] };
    let path = vec![f("items"), i(0)];
    assert!(
        diff_under(&src, &tgt, &scope)
            .iter()
            .any(|c| c.segments == path)
    );
    assert_eq!(plan_with(&scope, &src, &tgt, &[path]), Err(invalid()));
}

#[test]
fn a_removed_subtree_with_out_of_scope_fields_is_refused() {
    let scope = FieldScope::Only(vec!["meta.owner".into()]);
    let src = doc! { "_id": 1, "meta": { "owner": "o", "sync": "s" } };
    let tgt = doc! { "_id": 1 };
    let ch = diff_under(&src, &tgt, &scope);
    assert_eq!(ch.len(), 1);
    assert_eq!(ch[0].kind, ChangeKind::Removed);
    assert_eq!(
        plan_with(&scope, &src, &tgt, &[vec![f("meta")]]),
        Err(invalid())
    );
}

#[test]
fn an_unset_of_a_subtree_with_out_of_scope_fields_is_refused() {
    let scope = FieldScope::AllExcept(vec!["meta.sync".into()]);
    let src = doc! { "_id": 1 };
    let tgt = doc! { "_id": 1, "meta": { "owner": "o", "sync": "s" } };
    let ch = diff_under(&src, &tgt, &scope);
    assert_eq!(ch[0].kind, ChangeKind::Added);
    assert_eq!(
        plan_with(&scope, &src, &tgt, &[vec![f("meta")]]),
        Err(invalid())
    );
}

#[test]
fn a_changed_scalar_against_a_scoped_document_is_refused() {
    let scope = FieldScope::Only(vec!["meta.owner".into()]);
    let src = doc! { "_id": 1, "meta": 5 };
    let tgt = doc! { "_id": 1, "meta": { "owner": "o", "sync": "s" } };
    let r = plan_with(&scope, &src, &tgt, &[vec![f("meta")]]);
    assert_eq!(r, Err(invalid()));
}

#[test]
fn a_scope_that_keeps_the_subtree_intact_plans_normally() {
    let src = doc! { "_id": 1, "meta": { "owner": "a" }, "other": 1 };
    let tgt = doc! { "_id": 2, "meta": { "owner": "b" }, "other": 2 };
    for scope in [
        FieldScope::Only(vec!["meta".into()]),
        FieldScope::AllExcept(vec!["other".into()]),
    ] {
        let op = plan_with(&scope, &src, &tgt, &[vec![f("meta"), f("owner")]]).unwrap();
        assert_eq!(
            op,
            WriteOp::UpdateFields {
                id: Bson::Int32(2),
                set: vec![(vec![f("meta"), f("owner")], Bson::String("a".into()))],
                unset: vec![],
            }
        );
    }
}

#[test]
fn a_shifted_array_is_refused_even_when_the_leaf_value_happens_to_match() {
    let scope = FieldScope::Only(vec!["items.price".into()]);
    let src = doc! { "_id": 1, "items": [{ "price": 3 }, { "price": 5 }, { "price": 4 }] };
    let tgt = doc! { "_id": 1, "items": [7, { "price": 3 }, { "price": 3 }, { "price": 4 }] };
    let path = vec![f("items"), i(1), f("price")];
    let ch = diff_under(&src, &tgt, &scope);
    assert!(ch.iter().any(|c| c.segments == path));
    let paths = [path];
    // Destination shifted (source to target), then origin shifted (target to source).
    let to_target = plan_fields(&paths, &ch, ST, &src, &tgt, &scope);
    let to_source = plan_fields(&paths, &ch, TS, &tgt, &src, &scope);
    assert_eq!(to_target, Err(invalid()));
    assert_eq!(to_source, Err(invalid()));
}

#[test]
fn a_nested_array_that_loses_an_inner_array_is_refused() {
    let scope = FieldScope::Only(vec!["a.price".into()]);
    let src = doc! { "_id": 1, "a": [[5], [{ "price": 2 }], [{ "price": 2 }]] };
    let tgt = doc! { "_id": 1, "a": [[{ "price": 2 }], [{ "price": 9 }]] };
    let path = vec![f("a"), i(1), i(0), f("price")];
    let ch = diff_under(&src, &tgt, &scope);
    assert!(ch.iter().any(|c| c.segments == path), "{ch:?}");
    assert_eq!(
        plan_fields(&[path], &ch, ST, &src, &tgt, &scope),
        Err(invalid())
    );
}

#[test]
fn an_intact_array_under_only_still_plans() {
    let scope = FieldScope::Only(vec!["items.price".into()]);
    let src = doc! { "_id": 1, "items": [{ "price": 1, "x": 0 }, { "price": 2 }] };
    let tgt = doc! { "_id": 2, "items": [{ "price": 1 }, { "price": 8 }] };
    let path = vec![f("items"), i(1), f("price")];
    let op = plan_with(&scope, &src, &tgt, &[path.clone()]).unwrap();
    let WriteOp::UpdateFields { set, .. } = op else {
        panic!("expected UpdateFields")
    };
    assert_eq!(set, vec![(path, Bson::Int32(2))]);
}

#[test]
fn scope_all_is_a_no_op() {
    let d = doc! { "_id": 1, "a": [1, { "b": 2 }], "c": { "d": 3 } };
    assert_eq!(apply_scope(&d, &FieldScope::All), d);
    let src = doc! { "_id": 1, "items": [5, { "price": 1 }] };
    let tgt = doc! { "_id": 1, "items": [5, { "price": 2 }] };
    let path = vec![f("items"), i(1), f("price")];
    assert!(plan_with(&FieldScope::All, &src, &tgt, &[path]).is_ok());
}
