use super::*;
use bson::{Bson, doc};
use curlybrace_core::compare::apply::Direction;
use curlybrace_core::compare::apply_fields::{Unaddressable, can_copy};
use curlybrace_core::compare::diff::PathSegment;

fn change(path: &str, kind: ChangeKind, s: Option<Bson>, t: Option<Bson>) -> FieldChange {
    FieldChange {
        path: path.into(),
        segments: vec![PathSegment::Field(path.into())],
        kind,
        source: s,
        target: t,
    }
}

#[test]
fn canonical_json_keeps_int32_and_int64_apart() {
    let small = canonical_json(&doc! {"n": 1_i32});
    let long = canonical_json(&doc! {"n": 1_i64});
    assert_ne!(small, long);
    assert!(small.contains("$numberInt"), "{small}");
    assert!(long.contains("$numberLong"), "{long}");
    assert!(small.contains('\n'), "pretty printed");
}

#[test]
fn a_new_detail_is_loading_until_the_answer_arrives() {
    let d = DetailView::loading(KeyTuple(vec![Bson::Int32(1)]));
    assert!(d.loading);
    assert!(d.changes.is_empty());
}

#[test]
fn changes_become_lines_with_kind_keys_and_formatted_values() {
    let key = KeyTuple(vec![Bson::Int32(1)]);
    let changes = vec![
        change(
            "a",
            ChangeKind::Changed,
            Some(Bson::Int32(1)),
            Some(Bson::String("x".into())),
        ),
        change("b.c", ChangeKind::Added, None, Some(Bson::Boolean(true))),
        change("d", ChangeKind::Removed, Some(Bson::Null), None),
    ];
    let d = DetailView::loading(key).loaded(Some(doc! {"a": 1}), Some(doc! {"a": "x"}), &changes);
    assert!(!d.loading);
    assert_eq!(
        d.changes,
        vec![
            ChangeLine {
                path: "a".into(),
                kind: "compare.detail.changed",
                source_text: Some("1".into()),
                target_text: Some("\"x\"".into()),
                segments: vec![PathSegment::Field("a".into())],
                change: ChangeKind::Changed,
            },
            ChangeLine {
                path: "b.c".into(),
                kind: "compare.detail.added",
                source_text: None,
                target_text: Some("true".into()),
                segments: vec![PathSegment::Field("b.c".into())],
                change: ChangeKind::Added,
            },
            ChangeLine {
                path: "d".into(),
                kind: "compare.detail.removed",
                source_text: Some("null".into()),
                target_text: None,
                segments: vec![PathSegment::Field("d".into())],
                change: ChangeKind::Removed,
            },
        ]
    );
}

#[test]
fn rich_types_use_relaxed_extended_json() {
    let oid = bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap();
    assert_eq!(
        value_text(&Bson::ObjectId(oid)),
        r#"{"$oid":"507f1f77bcf86cd799439011"}"#
    );
}

#[test]
fn documents_are_kept_for_the_side_by_side_view() {
    let d =
        DetailView::loading(KeyTuple(vec![Bson::Int32(1)])).loaded(Some(doc! {"a": 1}), None, &[]);
    assert_eq!(d.source, Some(doc! {"a": 1}));
    assert_eq!(d.target, None);
}

#[test]
fn is_for_matches_only_the_requested_key() {
    let d = DetailView::loading(KeyTuple(vec![Bson::Int32(1)]));
    assert!(d.is_for(&KeyTuple(vec![Bson::Int32(1)])));
    assert!(!d.is_for(&KeyTuple(vec![Bson::Int32(2)])));
}

fn line_at(segments: Vec<PathSegment>, change: ChangeKind) -> ChangeLine {
    ChangeLine {
        path: String::new(),
        kind: kind_key(change),
        source_text: None,
        target_text: None,
        segments,
        change,
    }
}

fn fld(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

#[test]
fn copy_block_names_every_reason_in_both_directions() {
    use ChangeKind::{Added, Changed, Removed};
    use Unaddressable as U;
    let idx = PathSegment::Index(0);
    let table = [
        (vec![fld("_id")], Changed, Some(U::Id)),
        (vec![fld("_id"), fld("x")], Changed, Some(U::Id)),
        (vec![], Changed, Some(U::NotAChange)),
        (
            vec![fld("a"), PathSegment::AnyElement],
            Changed,
            Some(U::AnyElement),
        ),
        (vec![fld("a.b")], Changed, Some(U::BadName)),
        (vec![fld("$x")], Added, Some(U::BadName)),
        (vec![fld("")], Removed, Some(U::BadName)),
        (vec![fld("a"), idx.clone()], Added, Some(U::ArrayElement)),
        (vec![fld("a"), idx.clone()], Removed, Some(U::ArrayElement)),
        (
            vec![fld("a"), idx.clone(), fld("b")],
            Added,
            Some(U::ArrayElement),
        ),
        (vec![fld("a"), idx], Changed, None),
        (vec![fld("a")], Changed, None),
        (vec![fld("a")], Added, None),
        (vec![fld("a")], Removed, None),
        (vec![fld("a"), fld("b")], Added, None),
    ];
    for (segments, change, expected) in table {
        for direction in [Direction::SourceToTarget, Direction::TargetToSource] {
            let l = line_at(segments.clone(), change);
            assert_eq!(
                l.copy_block(direction),
                expected,
                "{segments:?} {change:?} {direction:?}"
            );
        }
    }
}

#[test]
fn copy_block_agrees_with_the_core_rules_for_a_loaded_change() {
    let c = change(
        "a",
        ChangeKind::Changed,
        Some(Bson::Int32(1)),
        Some(Bson::Int32(2)),
    );
    let d = DetailView::loading(KeyTuple(vec![Bson::Int32(1)])).loaded(None, None, &[c.clone()]);
    for direction in [Direction::SourceToTarget, Direction::TargetToSource] {
        assert_eq!(d.changes[0].copy_block(direction), can_copy(&c, direction));
    }
}
