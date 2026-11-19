use super::super::index_builder::BuilderEdit;
use super::*;
use curlybrace_core::explain::SuggestedIndex;
use curlybrace_core::pipeline_options::{CaseFirst, CollationOption, Strength};
use curlybrace_core::store::Namespace;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn index() -> SuggestedIndex {
    SuggestedIndex {
        keys: vec![("status".into(), 1), ("total".into(), -1)],
    }
}

fn collation() -> CollationOption {
    CollationOption {
        locale: "fr".into(),
        strength: Some(Strength::Secondary),
        case_level: None,
        case_first: Some(CaseFirst::Upper),
        numeric_ordering: None,
        alternate: None,
    }
}

#[test]
fn keys_and_directions_fill_the_rows() {
    let builder = IndexBuilderState::from_suggested(ns(), &index(), None);
    let rows: Vec<(String, KeyKind)> = builder
        .keys
        .iter()
        .map(|k| (k.field.clone(), k.kind.clone()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("status".to_string(), KeyKind::Asc),
            ("total".to_string(), KeyKind::Desc)
        ]
    );
    assert_eq!(builder.ns, ns());
    assert!(builder.collation_text.is_empty());
    assert!(builder.editing.is_none());
}

#[test]
fn the_name_follows_the_keys() {
    let builder = IndexBuilderState::from_suggested(ns(), &index(), None);
    assert_eq!(builder.name, "status_1_total_-1");
    assert!(!builder.name_edited);
    // Editing a key still renames it: the name was never typed.
    let renamed = builder.with_field(1, "amount");
    assert_eq!(renamed.name, "status_1_amount_-1");
}

#[test]
fn a_collation_fills_its_text_and_parses_in_spec() {
    let builder = IndexBuilderState::from_suggested(ns(), &index(), Some(&collation()));
    assert!(
        builder.collation_text.contains("\"locale\""),
        "{}",
        builder.collation_text
    );
    let spec = builder.spec().expect("a valid spec");
    let parsed = spec.options.collation.expect("the collation is parsed");
    assert_eq!(parsed.get_str("locale"), Ok("fr"));
    assert_eq!(parsed.get_i32("strength"), Ok(2));
    assert_eq!(parsed.get_str("caseFirst"), Ok("upper"));
}

#[test]
fn the_builder_spec_is_the_suggested_index() {
    let spec = IndexBuilderState::from_suggested(ns(), &index(), None)
        .spec()
        .expect("a valid spec");
    let fields: Vec<&str> = spec.keys.iter().map(|k| k.field.as_str()).collect();
    assert_eq!(fields, ["status", "total"]);
    assert_eq!(spec.options.collation, None);
}

#[test]
fn a_suggested_builder_does_not_claim_an_unrecorded_collation_by_default() {
    let builder = IndexBuilderState::from_suggested(ns(), &index(), None);
    assert!(!builder.collation_unrecorded);
}

#[test]
fn marking_the_collation_unrecorded_changes_only_that_flag() {
    let plain = IndexBuilderState::from_suggested(ns(), &index(), None);
    let marked = plain.clone().with_collation_unrecorded();
    assert!(marked.collation_unrecorded);
    let cleared = IndexBuilderState {
        collation_unrecorded: false,
        ..marked
    };
    assert_eq!(cleared, plain);
}

#[test]
fn typing_a_collation_keeps_the_flag_for_the_view_to_hide_the_note() {
    // The note shows while the collation box is empty; the state itself is not rewritten.
    let marked =
        IndexBuilderState::from_suggested(ns(), &index(), None).with_collation_unrecorded();
    let typed = marked.edited(BuilderEdit::SetCollationText("{\"locale\":\"en\"}".into()));
    assert!(typed.collation_unrecorded);
    assert!(!typed.collation_text.is_empty());
}
