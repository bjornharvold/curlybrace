use super::*;
use bson::doc;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;

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

fn original() -> IndexSpec {
    IndexSpec {
        options: IndexOptions {
            unique: true,
            expire_after_seconds: None,
            partial_filter: Some(doc! { "status": "open" }),
            ..IndexOptions::default()
        },
        raw: doc! {
            "key": { "loc": "2d", "kind": -1 }, "name": "loc_2d_kind_-1", "v": 2,
            "bits": 26, "min": -180.0, "max": 180.0,
        },
        ..mock_index_spec_named(
            "loc_2d_kind_-1",
            &[("loc", KeyKind::Geo2d), ("kind", KeyKind::Desc)],
        )
    }
}

#[test]
fn a_copy_starts_as_the_original_under_a_new_name() {
    let spec = original();
    let copy = IndexBuilderState::from_copy(ns(), &spec, "loc_2d_kind_-1_v2".into());
    assert_eq!(copy.keys, spec.keys);
    assert_eq!(copy.name, "loc_2d_kind_-1_v2");
    assert!(copy.name_edited, "the name does not follow the keys");
    assert_eq!(copy.replaces(), Some("loc_2d_kind_-1"));
    assert!(copy.options.unique);
    assert!(!copy.partial_filter_text.is_empty());
    let built = copy.spec().expect("a valid copy");
    // Keys, options and the unmodelled fields come out as they went in; only the name differs.
    assert_eq!(built.keys, spec.keys);
    assert_eq!(built.options, spec.options);
    assert_eq!(built.raw, spec.raw, "raw is carried along");
    let index = built.create_command("c").get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap()
        .clone();
    assert_eq!(index.get_i32("bits"), Ok(26));
    assert_eq!(index.get_f64("min"), Ok(-180.0));
    assert_eq!(index.get_str("name"), Ok("loc_2d_kind_-1_v2"));
}

#[test]
fn hashed_and_2d_rows_cannot_be_retyped_but_can_be_removed() {
    let copy = IndexBuilderState::from_copy(ns(), &original(), "c".into());
    let retyped = copy.clone().with_kind(0, KeyKind::Asc);
    assert_eq!(retyped, copy, "a 2d key stays 2d");
    let renamed = copy.clone().with_field(0, "other");
    assert_eq!(renamed, copy, "and keeps its field");
    // An ordinary key of the same copy is edited as usual.
    assert_eq!(
        copy.clone().with_kind(1, KeyKind::Asc).keys[1].kind,
        KeyKind::Asc
    );
    assert_eq!(copy.removing_key(0).keys.len(), 1);
    // Nothing the picker offers is hashed or 2d.
    assert!(!BUILDER_KINDS.contains(&KeyKind::Geo2d));
}

#[test]
fn a_copy_is_untouched_until_something_changes() {
    let copy = IndexBuilderState::from_copy(ns(), &original(), "c".into());
    assert!(copy.is_untouched());
    assert!(copy.clone().with_estimate(5).is_untouched());
    assert!(!copy.clone().with_unique(false).is_untouched());
    assert!(!copy.with_name("d").is_untouched());
}

#[test]
fn a_copy_keeps_the_original_filter_document_exactly_until_it_is_edited() {
    let spec = IndexSpec {
        options: IndexOptions {
            partial_filter: Some(doc! { "n": { "$gt": 5_i64 } }),
            ..IndexOptions::default()
        },
        ..mock_index_spec_named("n_1", &[("n", KeyKind::Asc)])
    };
    let copy = IndexBuilderState::from_copy(ns(), &spec, "n_1_v2".into());
    assert_eq!(
        copy.spec().unwrap().options.partial_filter,
        spec.options.partial_filter
    );
}

fn text_original() -> IndexSpec {
    IndexSpec::from_document(&doc! {
        "key": { "_fts": "text", "_ftsx": 1 }, "name": "title_text_body_text", "v": 2,
        "weights": { "title": 5, "body": 1 }, "default_language": "english",
        "language_override": "language", "textIndexVersion": 3,
    })
    .unwrap()
}

fn index_of(spec: &IndexSpec) -> bson::Document {
    spec.create_command("c").get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap()
        .clone()
}

#[test]
fn an_unedited_copy_of_a_text_index_still_round_trips_all_its_options() {
    let original = text_original();
    let copy = IndexBuilderState::from_copy(ns(), &original, "t_v2".into());
    let built = index_of(&copy.spec().unwrap());
    let first = index_of(&original);
    for field in [
        "weights",
        "default_language",
        "language_override",
        "textIndexVersion",
    ] {
        assert_eq!(built.get(field), first.get(field), "{field}");
    }
}

#[test]
fn removing_a_text_key_from_a_copy_removes_it_from_the_weights() {
    let copy = IndexBuilderState::from_copy(ns(), &text_original(), "t_v2".into()).removing_key(1);
    let spec = copy.spec().unwrap();
    let index = index_of(&spec);
    assert_eq!(index.get_document("weights").unwrap(), &doc! { "title": 5 });
    assert_eq!(index.get_str("default_language"), Ok("english"));
    // The preview is still exactly the command.
    assert_eq!(
        copy.preview_json(),
        doc_to_view_json(&spec.create_command("orders"))
    );
    // A text field added to the copy gets the default weight.
    let more = IndexBuilderState::from_copy(ns(), &text_original(), "t_v2".into())
        .adding_key()
        .with_field(2, "tags")
        .with_kind(2, KeyKind::Text);
    let weights = index_of(&more.spec().unwrap())
        .get_document("weights")
        .unwrap()
        .clone();
    assert_eq!(weights, doc! { "title": 5, "body": 1, "tags": 1 });
}

#[test]
fn a_copy_without_text_keys_carries_no_text_options() {
    let copy = IndexBuilderState::from_copy(ns(), &text_original(), "t_v2".into())
        .with_kind(0, KeyKind::Asc)
        .with_kind(1, KeyKind::Desc);
    let index = index_of(&copy.spec().unwrap());
    for field in [
        "weights",
        "default_language",
        "language_override",
        "textIndexVersion",
    ] {
        assert!(index.get(field).is_none(), "{field}");
    }
    let geo = IndexSpec::from_document(&doc! {
        "key": { "loc": "2dsphere" }, "name": "loc_2dsphere", "v": 2, "2dsphereIndexVersion": 3,
    })
    .unwrap();
    let kept = IndexBuilderState::from_copy(ns(), &geo, "g".into());
    assert_eq!(
        index_of(&kept.spec().unwrap()).get_i32("2dsphereIndexVersion"),
        Ok(3)
    );
    let plain = kept.with_kind(0, KeyKind::Asc);
    assert!(
        index_of(&plain.spec().unwrap())
            .get("2dsphereIndexVersion")
            .is_none()
    );
}

#[test]
fn a_copy_of_a_compound_text_index_keeps_its_non_text_keys_and_weights() {
    let original = IndexSpec::from_document(&doc! {
        "key": { "a": 1, "_fts": "text", "_ftsx": 1, "z": -1 }, "name": "a_1_t_text_z_-1", "v": 2,
        "weights": { "t": 3 }, "default_language": "english",
        "language_override": "language", "textIndexVersion": 3,
    })
    .unwrap();
    let copy = IndexBuilderState::from_copy(ns(), &original, "c_v2".into());
    let spec = copy.spec().unwrap();
    let index = index_of(&spec);
    assert_eq!(
        index.get_document("key").unwrap(),
        &doc! { "a": 1, "t": "text", "z": -1 }
    );
    assert_eq!(index.get_document("weights").unwrap(), &doc! { "t": 3 });
    assert_eq!(
        copy.preview_json(),
        doc_to_view_json(&spec.create_command("orders"))
    );
    assert!(copy.preview_json().contains("\"a\""));
    assert!(copy.preview_json().contains("\"z\""));
}

#[test]
fn a_key_type_the_builder_does_not_model_is_fixed_and_keeps_its_name_part() {
    let builder = on("loc").with_kind(0, KeyKind::Other("geoHaystack".into()));
    assert!(builder.is_fixed(0));
    assert_eq!(
        kind_label_key(&KeyKind::Other("geoHaystack".into())),
        "indexes.builder.kind_other"
    );
    assert_eq!(default_name(&builder.keys), "loc_geoHaystack");
}

#[test]
fn a_text_index_listed_without_weights_is_refused_not_rebuilt_on_a_literal_field() {
    let raw_key = doc! { "_fts": "text", "_ftsx": 1_i32 };
    let spec =
        IndexSpec::from_document(&doc! { "key": raw_key.clone(), "name": "t_text" }).unwrap();
    let copy = IndexBuilderState::from_copy(ns(), &spec, "t_text_v2".into());
    // Either the copy rebuilds the very same key document, or Create is refused.
    let same_key = copy.spec().ok().map(|built| {
        built.create_command("c").get_array("indexes").unwrap()[0]
            .as_document()
            .unwrap()
            .get_document("key")
            .unwrap()
            .clone()
    });
    assert!(
        same_key.is_none_or(|key| key == raw_key),
        "a copy must never alter the key"
    );
    assert_eq!(copy.problem(), Some(BuilderProblem::KeyNotRebuildable));
    assert_eq!(
        BuilderProblem::KeyNotRebuildable.message_key(),
        "indexes.builder.problem_unrebuildable"
    );
}
