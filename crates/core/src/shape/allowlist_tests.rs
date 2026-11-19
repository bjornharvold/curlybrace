//! Only known query operators survive in a shape: a `$`-key nobody knows is data in disguise, so
//! the whole document collapses to the placeholder and its key is never stored.

use bson::{Document, doc};

use super::{Shape, from_query};
use crate::store::Query;

fn shape_of(filter: Document) -> Shape {
    from_query(&Query {
        filter,
        ..Query::default()
    })
}

#[test]
fn unknown_dollar_keys_collapse_to_a_placeholder() {
    let shape = shape_of(doc! { "a": { "$secret": 1 } });
    assert_eq!(shape.json(), r#"{"filter":{"a":"?"}}"#);
    assert!(!shape.json().contains("secret"));
}

#[test]
fn known_operators_are_kept() {
    let shape = shape_of(doc! {
        "age": { "$gte": 1, "$lt": 9, "$ne": 5 },
        "tags": { "$all": ["x"], "$size": 2 },
        "name": { "$exists": true, "$type": "string" },
        "code": { "$mod": [4, 0], "$nin": [1] },
    });
    assert_eq!(
        shape.json(),
        concat!(
            r#"{"filter":{"age":{"$gte":"?","$lt":"?","$ne":"?"},"#,
            r#""code":{"$mod":"?","$nin":["?"]},"#,
            r#""name":{"$exists":"?","$type":"?"},"#,
            r#""tags":{"$all":["?"],"$size":"?"}}}"#
        )
    );
}

/// Every operator a field's operator document accepts on mongod 9.0.8, found by probing the
/// server with each `$`-word of its binary (and a hand list for the geo companions).
const SERVER_FIELD_OPERATORS: [&str; 28] = [
    "$all",
    "$bitsAllClear",
    "$bitsAllSet",
    "$bitsAnyClear",
    "$bitsAnySet",
    "$elemMatch",
    "$eq",
    "$exists",
    "$geoIntersects",
    "$geoWithin",
    "$gt",
    "$gte",
    "$in",
    "$lt",
    "$lte",
    "$maxDistance",
    "$minDistance",
    "$mod",
    "$ne",
    "$near",
    "$nearSphere",
    "$nin",
    "$not",
    "$options",
    "$regex",
    "$size",
    "$type",
    "$within",
];

/// What the server rejects at field level (operands or root-only words): data in disguise here.
const NOT_FIELD_OPERATORS: [&str; 12] = [
    "$box",
    "$center",
    "$centerSphere",
    "$polygon",
    "$geometry",
    "$uniqueDocs",
    "$comment",
    "$text",
    "$expr",
    "$slice",
    "$meta",
    "$natural",
];

#[test]
fn every_operator_the_server_accepts_at_field_level_is_kept() {
    for op in SERVER_FIELD_OPERATORS {
        let shape = shape_of(doc! { "a": { op: 1 } });
        assert_eq!(
            shape.json(),
            format!(r#"{{"filter":{{"a":{{"{op}":"?"}}}}}}"#)
        );
        assert!(Shape::from_json(&shape.json()).is_ok(), "{op}");
    }
    assert_eq!(SERVER_FIELD_OPERATORS.len(), 28);
}

#[test]
fn words_the_server_rejects_at_field_level_collapse() {
    for op in NOT_FIELD_OPERATORS {
        let shape = shape_of(doc! { "a": { op: 1 } });
        assert_eq!(shape.json(), r#"{"filter":{"a":"?"}}"#, "{op}");
    }
}

#[test]
fn a_within_shape_survives_the_normalizer() {
    let shape = shape_of(doc! { "loc": { "$within": { "$box": [[0, 0], [1, 1]] } } });
    assert_eq!(shape.json(), r#"{"filter":{"loc":{"$within":"?"}}}"#);
    assert!(Shape::from_json(&shape.json()).is_ok());
}

#[test]
fn the_regex_marker_forms_are_kept() {
    let shape = shape_of(
        doc! { "a": bson::Regex { pattern: "x".into(), options: "i".into() },
        "b": { "$in": [bson::Regex { pattern: "y".into(), options: String::new() }] } },
    );
    assert_eq!(
        shape.json(),
        r#"{"filter":{"a":{"$regex":"?"},"b":{"$in":["?","$regex"]}}}"#
    );
    assert!(Shape::from_json(&shape.json()).is_ok());
}

#[test]
fn a_mixed_known_and_unknown_operator_document_collapses() {
    let shape = shape_of(doc! { "a": { "$gt": 1, "$secret": 2 } });
    assert_eq!(shape.json(), r#"{"filter":{"a":"?"}}"#);
}

#[test]
fn elem_match_and_not_still_recurse() {
    let shape = shape_of(doc! {
        "a": { "$elemMatch": { "$gt": 1, "$lt": 5 } },
        "b": { "$not": { "$gt": 1 } },
        "c": { "$elemMatch": { "k": 1, "v": { "$gte": 2 } } },
    });
    assert_eq!(
        shape.json(),
        concat!(
            r#"{"filter":{"a":{"$elemMatch":{"$gt":"?","$lt":"?"}},"#,
            r#""b":{"$not":{"$gt":"?"}},"#,
            r#""c":{"$elemMatch":{"k":"?","v":{"$gte":"?"}}}}}"#
        )
    );
}

/// The words that are valid at the root of a query (not in a field's operator document).
const ROOT_ONLY: [&str; 6] = [
    "$comment",
    "$expr",
    "$jsonSchema",
    "$sampleRate",
    "$text",
    "$where",
];

#[test]
fn root_only_keys_in_an_elem_match_operand_keep_the_field_names() {
    for key in ROOT_ONLY {
        let shape = shape_of(doc! { "a": { "$elemMatch": { "qty": { "$gt": 1 }, key: "x" } } });
        let expected =
            format!(r#"{{"filter":{{"a":{{"$elemMatch":{{"{key}":"?","qty":{{"$gt":"?"}}}}}}}}}}"#);
        assert_eq!(shape.json(), expected, "{key}");
        assert!(Shape::from_json(&shape.json()).is_ok(), "{key}");
    }
}

#[test]
fn root_only_keys_next_to_operators_in_an_elem_match_operand_read_as_before() {
    for key in ROOT_ONLY {
        let shape = shape_of(doc! { "a": { "$elemMatch": { "$in": [1], key: "x" } } });
        // An operator document prints its keys sorted.
        let (key_part, in_part) = (format!(r#""{key}":"?""#), r#""$in":["?"]"#);
        let body = if key < "$in" {
            format!("{key_part},{in_part}")
        } else {
            format!("{in_part},{key_part}")
        };
        let expected = format!(r#"{{"filter":{{"a":{{"$elemMatch":{{{body}}}}}}}}}"#);
        assert_eq!(shape.json(), expected, "{key}");
        let alone = shape_of(doc! { "a": { "$elemMatch": { key: "x" } } });
        assert_eq!(
            alone.json(),
            format!(r#"{{"filter":{{"a":{{"$elemMatch":{{"{key}":"?"}}}}}}}}"#),
            "{key}"
        );
    }
}

#[test]
fn an_unknown_operator_beside_root_only_keys_still_collapses() {
    let shape = shape_of(doc! { "a": { "$elemMatch": { "$comment": "x", "$secret": 1 } } });
    assert_eq!(shape.json(), r#"{"filter":{"a":{"$elemMatch":"?"}}}"#);
}

#[test]
fn logical_and_nested_operands_keep_root_only_keys() {
    let shape = shape_of(doc! {
        "$and": [ { "a": { "$gt": 1 }, "$comment": "x" } ],
        "b": { "$elemMatch": { "$or": [ { "c": 1, "$comment": "y" } ], "d": { "$not": { "$gt": 2 } } } },
    });
    assert_eq!(
        shape.json(),
        concat!(
            r#"{"filter":{"$and":[{"$comment":"?","a":{"$gt":"?"}}],"#,
            r#""b":{"$elemMatch":{"$or":[{"$comment":"?","c":"?"}],"d":{"$not":{"$gt":"?"}}}}}}"#
        )
    );
    assert!(Shape::from_json(&shape.json()).is_ok());
}

#[test]
fn an_unknown_operator_inside_elem_match_or_not_collapses() {
    let shape = shape_of(doc! {
        "a": { "$elemMatch": { "$gt": 1, "$secret": 2 } },
        "b": { "$not": { "$secret": 1 } },
    });
    assert_eq!(
        shape.json(),
        r#"{"filter":{"a":{"$elemMatch":"?"},"b":{"$not":"?"}}}"#
    );
}

#[test]
fn from_json_rejects_the_old_form_with_an_unknown_operator() {
    assert!(Shape::from_json(r#"{"filter":{"a":{"$secret":"?"}}}"#).is_err());
    assert!(Shape::from_json(r#"{"filter":{"a":{"$gt":"?","$secret":"?"}}}"#).is_err());
    assert!(Shape::from_json(r#"{"filter":{"a":{"$gt":"?"}}}"#).is_ok());
}
