use super::*;
use bson::{Bson, doc};
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
            },
            ChangeLine {
                path: "b.c".into(),
                kind: "compare.detail.added",
                source_text: None,
                target_text: Some("true".into()),
            },
            ChangeLine {
                path: "d".into(),
                kind: "compare.detail.removed",
                source_text: Some("null".into()),
                target_text: None,
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
