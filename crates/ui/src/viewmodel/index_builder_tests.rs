use super::*;
use bson::doc;
use curlybrace_core::ejson::editor_json_to_doc;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

/// A builder with one valid ascending key on `field`.
fn on(field: &str) -> IndexBuilderState {
    IndexBuilderState::new(ns()).with_field(0, field)
}

#[test]
fn create_is_disabled_with_no_keys_or_duplicate_fields() {
    assert_eq!(
        IndexBuilderState::new(ns()).problem(),
        Some(BuilderProblem::Index(IndexProblem::EmptyField))
    );
    let none = on("a").removing_key(0);
    assert_eq!(
        none.problem(),
        Some(BuilderProblem::Index(IndexProblem::NoKeys))
    );
    assert!(none.spec().is_err());

    let twice = on("a").adding_key().with_field(1, "a");
    assert_eq!(
        twice.problem(),
        Some(BuilderProblem::Index(IndexProblem::DuplicateField(
            "a".into()
        )))
    );
    // Whitespace around a field does not make it a different field.
    let padded = on("a").adding_key().with_field(1, "  a ");
    assert_eq!(padded.problem(), twice.problem());

    assert_eq!(on("a").problem(), None);
    assert!(on("a").spec().is_ok());
}

#[test]
fn the_preview_is_the_exact_command_the_store_receives() {
    let builder = on("customerId")
        .adding_key()
        .with_field(1, "createdAt")
        .with_kind(1, KeyKind::Desc)
        .with_unique(true)
        .with_partial_filter_text(r#"{ "status": "open" }"#)
        .with_collation_text(r#"{ "locale": "en" }"#);
    let spec = builder.spec().expect("a valid builder");
    let command = spec.create_command("orders");
    assert_eq!(builder.preview_json(), doc_to_view_json(&command));
    // And it reads back as that command (no number of the command is an int64 here).
    assert_eq!(
        editor_json_to_doc(&builder.preview_json()).unwrap(),
        command
    );
    assert_eq!(
        command.get_str("createIndexes"),
        Ok("orders"),
        "the collection is the builder's"
    );
}

#[test]
fn the_preview_of_an_invalid_builder_still_renders_the_command_shape() {
    let builder = on("a").with_partial_filter_text("{ nope");
    assert!(builder.spec().is_err());
    let preview = editor_json_to_doc(&builder.preview_json()).unwrap();
    assert_eq!(preview.get_str("createIndexes"), Ok("orders"));
    let index = preview.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap()
        .clone();
    assert!(!index.contains_key("partialFilterExpression"));
}

#[test]
fn a_ttl_with_a_compound_key_is_rejected() {
    let compound = on("a").adding_key().with_field(1, "b").with_ttl(Some(60));
    assert_eq!(
        compound.problem(),
        Some(BuilderProblem::Index(IndexProblem::TtlNeedsOneOrderedKey))
    );
    let text = on("a").with_kind(0, KeyKind::Text).with_ttl(Some(60));
    assert_eq!(
        text.problem(),
        Some(BuilderProblem::Index(IndexProblem::TtlNeedsOneOrderedKey))
    );
    let zero = on("a").with_ttl(Some(0));
    assert_eq!(
        zero.problem(),
        Some(BuilderProblem::Index(IndexProblem::TtlNotPositive))
    );
    let fine = on("a").with_ttl(Some(3_600));
    assert_eq!(
        fine.spec().unwrap().options.expire_after_seconds,
        Some(3_600)
    );
}

#[test]
fn a_partial_filter_that_is_not_json_is_rejected() {
    for text in ["{ nope", "[1, 2]", "42", "\"text\""] {
        let builder = on("a").with_partial_filter_text(text);
        assert_eq!(
            builder.problem(),
            Some(BuilderProblem::PartialFilterInvalid),
            "{text}"
        );
    }
    let ok = on("a").with_partial_filter_text(r#"{ "qty": { "$gt": 5 } }"#);
    assert_eq!(
        ok.spec().unwrap().options.partial_filter,
        Some(doc! { "qty": { "$gt": 5_i32 } })
    );
    // Blank means no filter.
    assert_eq!(
        on("a")
            .with_partial_filter_text("   ")
            .spec()
            .unwrap()
            .options
            .partial_filter,
        None
    );
    let collation = on("a").with_collation_text("not json");
    assert_eq!(collation.problem(), Some(BuilderProblem::CollationInvalid));
}

#[test]
fn field_names_with_dots_and_unicode_are_kept_verbatim() {
    for field in [
        "address.city",
        "naïve.日本語",
        "weird field.with space",
        "a.b.c.d",
    ] {
        let spec = on(field).spec().unwrap();
        assert_eq!(spec.keys[0].field, field);
    }
    // Only the ends are trimmed.
    let spec = on("  address . city\t").spec().unwrap();
    assert_eq!(spec.keys[0].field, "address . city");
    // The state keeps what was typed, so the text box does not jump under the cursor.
    assert_eq!(on("  a ").keys[0].field, "  a ");
}

#[test]
fn the_default_name_follows_mongodb_naming_until_the_user_edits_it() {
    let builder = on("a");
    assert_eq!(builder.name, "a_1");
    let builder = builder
        .adding_key()
        .with_field(1, "other")
        .with_kind(1, KeyKind::Desc);
    assert_eq!(builder.name, "a_1_other_-1");
    let builder = builder
        .adding_key()
        .with_field(2, "body")
        .with_kind(2, KeyKind::Text);
    assert_eq!(builder.name, "a_1_other_-1_body_text");
    let geo = on("loc").with_kind(0, KeyKind::Geo2dSphere);
    assert_eq!(geo.name, "loc_2dsphere");
    let wild = on("").with_kind(0, KeyKind::Wildcard);
    assert_eq!(wild.name, "$**_1");
    assert_eq!(wild.spec().unwrap().keys[0].field, "$**");
    let scoped = on("attrs").with_kind(0, KeyKind::Wildcard);
    assert_eq!(scoped.name, "attrs.$**_1");

    // Once the person types a name it stays, whatever the keys do.
    let named = builder.with_name("my_index");
    assert!(named.name_edited);
    let named = named.adding_key().with_field(3, "z");
    assert_eq!(named.name, "my_index");
    assert_eq!(named.spec().unwrap().name, "my_index");
    // Emptying the box hands the name back to the keys.
    let reset = named.with_name("  ");
    assert!(!reset.name_edited);
    assert_eq!(reset.name, "a_1_other_-1_body_text_z_1");
}

#[test]
fn removing_and_moving_keys_keep_the_others_in_order() {
    let b = on("a")
        .adding_key()
        .with_field(1, "b")
        .adding_key()
        .with_field(2, "c");
    let fields =
        |b: &IndexBuilderState| -> Vec<String> { b.keys.iter().map(|k| k.field.clone()).collect() };
    assert_eq!(fields(&b.clone().moving_key(0, 2)), ["b", "c", "a"]);
    assert_eq!(fields(&b.clone().moving_key(2, 0)), ["c", "a", "b"]);
    assert_eq!(fields(&b.clone().moving_key(1, 1)), ["a", "b", "c"]);
    assert_eq!(fields(&b.clone().moving_key(0, 9)), ["a", "b", "c"]);
    assert_eq!(fields(&b.clone().removing_key(1)), ["a", "c"]);
    assert_eq!(fields(&b.clone().removing_key(9)), ["a", "b", "c"]);
    assert_eq!(b.clone().moving_key(0, 2).name, "b_1_c_1_a_1");
}

#[test]
fn only_the_builder_kinds_are_offered() {
    assert_eq!(
        BUILDER_KINDS,
        [
            KeyKind::Asc,
            KeyKind::Desc,
            KeyKind::Text,
            KeyKind::Geo2dSphere,
            KeyKind::Wildcard
        ]
    );
    assert!(!BUILDER_KINDS.contains(&KeyKind::Hashed));
    assert!(!BUILDER_KINDS.contains(&KeyKind::Geo2d));
}

#[test]
fn the_estimate_warns_only_above_the_threshold() {
    let at = on("a").with_estimate(BUILD_WARNING_THRESHOLD);
    assert_eq!(at.warning, None);
    assert_eq!(at.estimated_documents, Some(BUILD_WARNING_THRESHOLD));
    let over = on("a").with_estimate(BUILD_WARNING_THRESHOLD + 1);
    assert_eq!(
        over.warning,
        Some(BuildWarning::LargeCollection {
            documents: BUILD_WARNING_THRESHOLD + 1
        })
    );
    // The warning never touches validity.
    assert!(over.spec().is_ok());
}

#[test]
fn edits_apply_the_matching_updater() {
    let b = on("a")
        .edited(BuilderEdit::AddKey)
        .edited(BuilderEdit::SetField(1, "b".into()))
        .edited(BuilderEdit::SetKind(1, KeyKind::Desc))
        .edited(BuilderEdit::MoveKey { from: 1, to: 0 })
        .edited(BuilderEdit::SetUnique(true))
        .edited(BuilderEdit::SetSparse(true))
        .edited(BuilderEdit::SetHidden(true))
        .edited(BuilderEdit::SetName("n".into()));
    assert_eq!(b.name, "n");
    assert_eq!(b.keys[0].field, "b");
    assert!(b.options.unique && b.options.sparse && b.options.hidden);
    let b = b
        .edited(BuilderEdit::SetTtl(Some(5)))
        .edited(BuilderEdit::SetPartialFilterText("{}".into()))
        .edited(BuilderEdit::SetCollationText("{}".into()))
        .edited(BuilderEdit::RemoveKey(0));
    assert_eq!(b.options.expire_after_seconds, Some(5));
    assert_eq!(
        (b.partial_filter_text.as_str(), b.collation_text.as_str()),
        ("{}", "{}")
    );
    assert_eq!(b.keys.len(), 1);
}

#[test]
fn a_builder_is_untouched_until_something_changes() {
    assert!(IndexBuilderState::new(ns()).is_untouched());
    assert!(
        IndexBuilderState::new(ns())
            .with_estimate(5_000_000)
            .is_untouched()
    );
    assert!(!on("a").is_untouched());
    assert!(
        !IndexBuilderState::new(ns())
            .with_unique(true)
            .is_untouched()
    );
}

#[test]
fn updaters_return_new_values_and_leave_the_original() {
    let original = on("a");
    let before = original.clone();
    let _ = original
        .clone()
        .with_unique(true)
        .adding_key()
        .with_name("x");
    assert_eq!(original, before);
}
