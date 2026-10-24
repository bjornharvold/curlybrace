use bson::{Bson, doc};

use super::*;
use crate::test_support::mock_objects::{mock_index_spec, mock_index_spec_named};

fn key(field: &str, kind: KeyKind) -> IndexKey {
    IndexKey {
        field: field.into(),
        kind,
    }
}

fn with_options(options: IndexOptions) -> IndexSpec {
    IndexSpec {
        options,
        ..mock_index_spec()
    }
}

#[test]
fn from_document_reads_keys_in_order_and_options() {
    let spec = IndexSpec::from_document(&doc! {
        "key": { "a": 1_i32, "b": -1_i32 },
        "name": "a_1_b_-1",
        "unique": true,
        "expireAfterSeconds": 60_i32,
    })
    .unwrap();
    assert_eq!(spec.name, "a_1_b_-1");
    assert_eq!(
        spec.keys,
        vec![key("a", KeyKind::Asc), key("b", KeyKind::Desc)]
    );
    assert!(spec.options.unique);
    assert!(!spec.options.sparse);
    assert_eq!(spec.options.expire_after_seconds, Some(60));
}

#[test]
fn from_document_keeps_hashed_and_2d_keys() {
    let hashed = IndexSpec::from_document(&doc! { "key": { "h": "hashed" }, "name": "h" }).unwrap();
    assert_eq!(hashed.keys, vec![key("h", KeyKind::Hashed)]);
    let flat = IndexSpec::from_document(&doc! { "key": { "p": "2d" }, "name": "p" }).unwrap();
    assert_eq!(flat.keys, vec![key("p", KeyKind::Geo2d)]);
}

#[test]
fn from_document_reads_text_geo_and_wildcard_keys() {
    let spec = IndexSpec::from_document(&doc! {
        "key": { "t": "text", "g": "2dsphere", "$**": 1_i32, "a.$**": 1_i32 },
        "name": "mixed",
    })
    .unwrap();
    let kinds: Vec<_> = spec.keys.iter().map(|k| k.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            KeyKind::Text,
            KeyKind::Geo2dSphere,
            KeyKind::Wildcard,
            KeyKind::Wildcard
        ]
    );
}

#[test]
fn a_non_numeric_wildcard_marker_is_not_a_wildcard_key() {
    let doc = doc! { "key": { "$**": "columnstore" }, "name": "cs" };
    let spec = IndexSpec::from_document(&doc).unwrap();
    assert_eq!(
        spec.keys,
        vec![key("$**", KeyKind::Other("columnstore".into()))]
    );
    // It goes back out as the server wrote it, not as `{ "$**": 1 }`.
    assert_eq!(
        spec.create_command("c").get_array("indexes").unwrap()[0]
            .as_document()
            .unwrap()
            .get_document("key"),
        Ok(&doc! { "$**": "columnstore" })
    );
    let nested = IndexSpec::from_document(&doc! { "key": { "a.$**": -1_i32 }, "name": "w" });
    assert_eq!(nested.unwrap().keys, vec![key("a.$**", KeyKind::Wildcard)]);
}

#[test]
fn from_document_rejects_a_document_without_key_or_name() {
    assert!(IndexSpec::from_document(&doc! { "name": "x" }).is_err());
    assert!(IndexSpec::from_document(&doc! { "key": { "a": 1_i32 } }).is_err());
    assert!(IndexSpec::from_document(&doc! { "key": { "a": [1_i32] }, "name": "x" }).is_err());
}

#[test]
fn create_command_matches_the_server_shape() {
    let expected = doc! {
        "createIndexes": "orders",
        "indexes": [{
            "key": { "customerId": 1_i32, "createdAt": -1_i32 },
            "name": "customerId_1_createdAt_-1",
        }],
    };
    assert_eq!(mock_index_spec().create_command("orders"), expected);
}

#[test]
fn create_command_adds_options_only_when_set() {
    let spec = with_options(IndexOptions {
        unique: true,
        sparse: true,
        hidden: true,
        expire_after_seconds: Some(30),
        partial_filter: Some(doc! { "n": { "$gt": 1_i32 } }),
        collation: Some(doc! { "locale": "en" }),
        wildcard_projection: Some(doc! { "a": 1_i32 }),
    });
    let command = spec.create_command("orders");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap()
        .clone();
    assert_eq!(index.get_bool("unique"), Ok(true));
    assert_eq!(index.get_bool("sparse"), Ok(true));
    assert_eq!(index.get_bool("hidden"), Ok(true));
    assert_eq!(index.get_i64("expireAfterSeconds"), Ok(30));
    assert_eq!(
        index.get_document("partialFilterExpression"),
        Ok(&doc! { "n": { "$gt": 1_i32 } })
    );
    assert_eq!(
        index.get_document("collation"),
        Ok(&doc! { "locale": "en" })
    );
    assert_eq!(
        index.get_document("wildcardProjection"),
        Ok(&doc! { "a": 1_i32 })
    );
}

#[test]
fn create_command_writes_every_key_kind() {
    let spec = mock_index_spec_named(
        "k",
        &[
            ("a", KeyKind::Asc),
            ("b", KeyKind::Desc),
            ("c", KeyKind::Text),
            ("d", KeyKind::Geo2dSphere),
            ("e.$**", KeyKind::Wildcard),
            ("f", KeyKind::Hashed),
            ("g", KeyKind::Geo2d),
        ],
    );
    let command = spec.create_command("c");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap();
    assert_eq!(
        index.get_document("key"),
        Ok(
            &doc! { "a": 1_i32, "b": -1_i32, "c": "text", "d": "2dsphere",
            "e.$**": 1_i32, "f": "hashed", "g": "2d" }
        )
    );
}

#[test]
fn definition_hash_ignores_name_and_hidden() {
    let base = mock_index_spec();
    let renamed = IndexSpec {
        name: "other".into(),
        options: IndexOptions {
            hidden: true,
            ..base.options.clone()
        },
        ..base.clone()
    };
    assert_eq!(base.definition_hash(), renamed.definition_hash());
    assert_eq!(base.definition_hash().len(), 64);
}

#[test]
fn definition_hash_changes_with_keys_order_or_options() {
    let base = mock_index_spec();
    let reordered = mock_index_spec_named(
        &base.name,
        &[("createdAt", KeyKind::Desc), ("customerId", KeyKind::Asc)],
    );
    assert_ne!(base.definition_hash(), reordered.definition_hash());
    let unique = with_options(IndexOptions {
        unique: true,
        ..IndexOptions::default()
    });
    assert_ne!(base.definition_hash(), unique.definition_hash());
}

#[test]
fn to_json_round_trips_through_from_document() {
    let spec = IndexSpec {
        options: IndexOptions {
            unique: true,
            expire_after_seconds: Some(5),
            partial_filter: Some(doc! { "n": 1_i32 }),
            ..IndexOptions::default()
        },
        ..mock_index_spec()
    };
    let value: serde_json::Value = serde_json::from_str(&spec.to_json()).unwrap();
    let Bson::Document(parsed) = Bson::try_from(value).unwrap() else {
        panic!("expected a document");
    };
    let back = IndexSpec::from_document(&parsed).unwrap();
    assert_eq!(back.name, spec.name);
    assert_eq!(back.keys, spec.keys);
    assert_eq!(back.options, spec.options);
    assert_eq!(back.definition_hash(), spec.definition_hash());
}

#[test]
fn validate_new_accepts_a_plain_spec() {
    assert_eq!(validate_new(&mock_index_spec()), Ok(()));
}

#[test]
fn validate_new_rejects_no_keys() {
    let spec = mock_index_spec_named("x", &[]);
    assert_eq!(validate_new(&spec), Err(IndexProblem::NoKeys));
}

#[test]
fn validate_new_rejects_an_empty_field() {
    let spec = mock_index_spec_named("x", &[("  ", KeyKind::Asc)]);
    assert_eq!(validate_new(&spec), Err(IndexProblem::EmptyField));
}

#[test]
fn validate_new_rejects_a_duplicate_field() {
    let spec = mock_index_spec_named("x", &[("a", KeyKind::Asc), ("a", KeyKind::Desc)]);
    assert_eq!(
        validate_new(&spec),
        Err(IndexProblem::DuplicateField("a".into()))
    );
}

#[test]
fn validate_new_rejects_a_ttl_on_a_compound_or_unordered_key() {
    let ttl = IndexOptions {
        expire_after_seconds: Some(10),
        ..IndexOptions::default()
    };
    let compound = IndexSpec {
        options: ttl.clone(),
        ..mock_index_spec()
    };
    assert_eq!(
        validate_new(&compound),
        Err(IndexProblem::TtlNeedsOneOrderedKey)
    );
    let hashed = IndexSpec {
        options: ttl,
        ..mock_index_spec_named("h", &[("h", KeyKind::Hashed)])
    };
    assert_eq!(
        validate_new(&hashed),
        Err(IndexProblem::TtlNeedsOneOrderedKey)
    );
}

#[test]
fn validate_new_rejects_a_negative_ttl_and_accepts_zero() {
    let zero = IndexSpec {
        options: IndexOptions {
            expire_after_seconds: Some(0),
            ..IndexOptions::default()
        },
        ..mock_index_spec_named("t", &[("at", KeyKind::Asc)])
    };
    assert_eq!(validate_new(&zero), Ok(()), "the server accepts 0");
    for seconds in [-1, -5] {
        let spec = IndexSpec {
            options: IndexOptions {
                expire_after_seconds: Some(seconds),
                ..IndexOptions::default()
            },
            ..mock_index_spec_named("t", &[("at", KeyKind::Asc)])
        };
        assert_eq!(validate_new(&spec), Err(IndexProblem::TtlNegative));
    }
}

#[test]
fn validate_new_accepts_a_ttl_on_one_ordered_key() {
    let spec = IndexSpec {
        options: IndexOptions {
            expire_after_seconds: Some(3600),
            ..IndexOptions::default()
        },
        ..mock_index_spec_named("t", &[("at", KeyKind::Desc)])
    };
    assert_eq!(validate_new(&spec), Ok(()));
}

#[test]
fn validate_new_accepts_a_wildcard_with_projection() {
    let spec = IndexSpec {
        options: IndexOptions {
            wildcard_projection: Some(doc! { "a": 1_i32 }),
            ..IndexOptions::default()
        },
        ..mock_index_spec_named("w", &[("$**", KeyKind::Wildcard)])
    };
    assert_eq!(validate_new(&spec), Ok(()));
}

fn server_text_index() -> bson::Document {
    doc! {
        "v": 2_i32,
        "key": { "_fts": "text", "_ftsx": 1_i32 },
        "name": "title_text_body_text",
        "weights": { "title": 1_i32, "body": 1_i32 },
        "default_language": "english",
        "language_override": "language",
        "textIndexVersion": 3_i32,
    }
}

#[test]
fn from_document_rebuilds_a_server_text_index_from_its_weights() {
    let spec = IndexSpec::from_document(&server_text_index()).unwrap();
    assert_eq!(
        spec.keys,
        vec![key("title", KeyKind::Text), key("body", KeyKind::Text)]
    );
}

#[test]
fn a_parsed_text_index_round_trips_through_create_command() {
    let spec = IndexSpec::from_document(&server_text_index()).unwrap();
    let command = spec.create_command("posts");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap();
    assert_eq!(
        index.get_document("key"),
        Ok(&doc! { "title": "text", "body": "text" })
    );
    assert_eq!(
        index.get_document("weights"),
        Ok(&doc! { "title": 1_i32, "body": 1_i32 })
    );
    assert_eq!(index.get_str("default_language"), Ok("english"));
    assert_eq!(index.get_str("language_override"), Ok("language"));
    assert_eq!(index.get_i32("textIndexVersion"), Ok(3));
    assert!(!index.contains_key("v") && !index.contains_key("_fts"));
}

#[test]
fn a_user_built_text_index_has_no_fts_keys() {
    let spec = mock_index_spec_named("t", &[("title", KeyKind::Text)]);
    let command = spec.create_command("posts");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap();
    assert_eq!(index.get_document("key"), Ok(&doc! { "title": "text" }));
    assert!(!index.contains_key("weights"));
}

#[test]
fn text_indexes_over_different_fields_hash_differently() {
    let mut other = server_text_index();
    other.insert("weights", doc! { "title": 1_i32, "summary": 1_i32 });
    let a = IndexSpec::from_document(&server_text_index()).unwrap();
    let b = IndexSpec::from_document(&other).unwrap();
    assert_ne!(a.definition_hash(), b.definition_hash());
}

#[test]
fn create_command_keeps_unmodelled_fields_of_a_2d_index() {
    let spec = IndexSpec::from_document(&doc! {
        "v": 2_i32,
        "key": { "loc": "2d" },
        "name": "loc_2d",
        "bits": 20_i32,
        "min": -100.0,
        "max": 100.0,
    })
    .unwrap();
    let command = spec.create_command("places");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap();
    assert_eq!(index.get_i32("bits"), Ok(20));
    assert_eq!(index.get_f64("min"), Ok(-100.0));
    assert_eq!(index.get_f64("max"), Ok(100.0));
    assert!(!index.contains_key("v"));
    assert_eq!(index.keys().filter(|k| *k == "key").count(), 1);
}

fn compound_text_index() -> bson::Document {
    doc! {
        "v": 2_i32,
        "key": { "a": 1_i32, "_fts": "text", "_ftsx": 1_i32, "z": -1_i32 },
        "name": "a_1_t_text_z_-1",
        "weights": { "t": 1_i32 },
        "default_language": "english",
        "language_override": "language",
        "textIndexVersion": 3_i32,
    }
}

#[test]
fn a_compound_text_index_keeps_its_non_text_keys_in_order() {
    let spec = IndexSpec::from_document(&compound_text_index()).unwrap();
    assert_eq!(
        spec.keys,
        vec![
            key("a", KeyKind::Asc),
            key("t", KeyKind::Text),
            key("z", KeyKind::Desc)
        ]
    );
}

#[test]
fn a_compound_text_index_round_trips_through_create_command() {
    let spec = IndexSpec::from_document(&compound_text_index()).unwrap();
    let command = spec.create_command("posts");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap();
    let key_doc = index.get_document("key").unwrap();
    assert_eq!(key_doc, &doc! { "a": 1_i32, "t": "text", "z": -1_i32 });
    assert_eq!(
        key_doc.keys().collect::<Vec<_>>(),
        vec!["a", "t", "z"],
        "key order is preserved"
    );
    assert_eq!(index.get_document("weights"), Ok(&doc! { "t": 1_i32 }));
    assert_eq!(index.get_str("default_language"), Ok("english"));
}

#[test]
fn compound_text_indexes_with_a_different_prefix_hash_differently() {
    let mut other = compound_text_index();
    other.insert(
        "key",
        doc! { "b": 1_i32, "_fts": "text", "_ftsx": 1_i32, "z": -1_i32 },
    );
    let a = IndexSpec::from_document(&compound_text_index()).unwrap();
    let b = IndexSpec::from_document(&other).unwrap();
    assert_ne!(a.definition_hash(), b.definition_hash());
}

#[test]
fn numeric_option_flags_read_as_true() {
    let spec = IndexSpec::from_document(&doc! {
        "key": { "a": 1_i32 }, "name": "a_1",
        "unique": 1_i32, "sparse": 1_i64, "hidden": 1.0_f64,
    })
    .unwrap();
    assert!(spec.options.unique && spec.options.sparse && spec.options.hidden);
}

#[test]
fn false_zero_and_absent_option_flags_read_as_false() {
    let spec = IndexSpec::from_document(&doc! {
        "key": { "a": 1_i32 }, "name": "a_1",
        "unique": false, "sparse": 0_i32, "hidden": "yes",
    })
    .unwrap();
    assert!(!spec.options.unique && !spec.options.sparse && !spec.options.hidden);
    let plain = IndexSpec::from_document(&doc! { "key": { "a": 1_i32 }, "name": "a_1" }).unwrap();
    assert!(!plain.options.unique && !plain.options.sparse && !plain.options.hidden);
}

#[test]
fn definition_hash_ignores_the_ttl() {
    let ttl = |seconds| {
        with_options(IndexOptions {
            expire_after_seconds: Some(seconds),
            ..IndexOptions::default()
        })
    };
    assert_eq!(ttl(60).definition_hash(), ttl(7200).definition_hash());
    assert_eq!(
        ttl(60).definition_hash(),
        mock_index_spec().definition_hash()
    );
}

#[test]
fn an_unknown_key_type_is_kept_as_other_and_round_trips() {
    let spec = IndexSpec::from_document(&doc! {
        "key": { "loc": "geoHaystack", "t": 1_i32 },
        "name": "loc_hay",
        "bucketSize": 1_i32,
    })
    .unwrap();
    assert_eq!(
        spec.keys,
        vec![
            key("loc", KeyKind::Other("geoHaystack".into())),
            key("t", KeyKind::Asc)
        ]
    );
    let command = spec.create_command("c");
    let index = command.get_array("indexes").unwrap()[0]
        .as_document()
        .unwrap();
    assert_eq!(
        index.get_document("key"),
        Ok(&doc! { "loc": "geoHaystack", "t": 1_i32 })
    );
    assert_eq!(index.get_i32("bucketSize"), Ok(1));
}

#[test]
fn a_text_index_without_weights_is_kept_as_an_unmodelled_key() {
    let spec = IndexSpec::from_document(&doc! {
        "key": { "_fts": "text", "_ftsx": 1_i32 },
        "name": "t_text",
    })
    .unwrap();
    assert_eq!(spec.keys, vec![key("_fts", KeyKind::Other("text".into()))]);
}

#[test]
fn a_key_value_that_is_not_a_type_is_still_an_error() {
    assert!(
        IndexSpec::from_document(&doc! { "key": { "a": { "x": 1_i32 } }, "name": "a" }).is_err()
    );
}
