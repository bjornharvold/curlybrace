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
fn validate_new_rejects_a_ttl_that_is_not_positive() {
    for seconds in [0, -5] {
        let spec = IndexSpec {
            options: IndexOptions {
                expire_after_seconds: Some(seconds),
                ..IndexOptions::default()
            },
            ..mock_index_spec_named("t", &[("at", KeyKind::Asc)])
        };
        assert_eq!(validate_new(&spec), Err(IndexProblem::TtlNotPositive));
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
