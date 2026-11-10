//! Name rules for databases, collections and views, and the typed refusals they produce.

use crate::admin::{parse_pipeline, validate_collection_name, validate_database_name};
use crate::error::CoreError;

#[test]
fn ordinary_names_are_accepted() {
    assert_eq!(validate_collection_name("orders"), Ok(()));
    assert_eq!(validate_collection_name("order items.2024"), Ok(()));
    assert_eq!(validate_collection_name("café"), Ok(()));
    assert_eq!(validate_database_name("shop"), Ok(()));
}

#[test]
fn a_blank_collection_name_is_refused() {
    assert_eq!(
        validate_collection_name(""),
        Err(CoreError::InvalidNamespace)
    );
    assert_eq!(
        validate_collection_name("   "),
        Err(CoreError::InvalidNamespace)
    );
}

#[test]
fn a_collection_name_with_a_dollar_or_null_is_refused() {
    assert_eq!(
        validate_collection_name("a$b"),
        Err(CoreError::InvalidNamespace)
    );
    assert_eq!(
        validate_collection_name("a\0b"),
        Err(CoreError::InvalidNamespace)
    );
}

#[test]
fn system_collections_are_refused() {
    assert_eq!(
        validate_collection_name("system.views"),
        Err(CoreError::InvalidNamespace)
    );
}

#[test]
fn a_database_name_with_a_forbidden_character_is_refused() {
    for bad in [
        "", "a/b", "a\\b", "a.b", "a b", "a\"b", "a$b", "a*b", "a<b", "a>b", "a:b", "a|b", "a?b",
        "a\0b",
    ] {
        assert_eq!(
            validate_database_name(bad),
            Err(CoreError::InvalidNamespace),
            "{bad:?}"
        );
    }
}

#[test]
fn a_database_name_of_64_bytes_or_more_is_refused() {
    assert_eq!(validate_database_name(&"a".repeat(63)), Ok(()));
    assert_eq!(
        validate_database_name(&"a".repeat(64)),
        Err(CoreError::InvalidNamespace)
    );
}

#[test]
fn reserved_databases_are_refused() {
    for reserved in ["admin", "local", "config"] {
        assert_eq!(
            validate_database_name(reserved),
            Err(CoreError::ReservedDatabase),
            "{reserved}"
        );
    }
}

#[test]
fn a_pipeline_is_a_json_array_of_stage_documents() {
    let stages = parse_pipeline(r#"[{"$match": {"a": 1}}, {"$limit": 5}]"#).unwrap();
    assert_eq!(stages.len(), 2);
    assert_eq!(parse_pipeline("[]").unwrap().len(), 0);
}

#[test]
fn a_malformed_pipeline_is_refused_with_a_typed_error() {
    assert!(matches!(
        parse_pipeline("{oops"),
        Err(CoreError::InvalidJson(_))
    ));
    assert!(matches!(
        parse_pipeline(r#"{"a": 1}"#),
        Err(CoreError::InvalidDocument(_))
    ));
    assert!(matches!(
        parse_pipeline("[1, 2]"),
        Err(CoreError::InvalidDocument(_))
    ));
}
