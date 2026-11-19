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
