use bson::doc;

use super::{SuggestedIndex, suggest};
use crate::store::Query;

fn keys(pairs: &[(&str, i32)]) -> Vec<(String, i32)> {
    pairs.iter().map(|(f, d)| (f.to_string(), *d)).collect()
}

#[test]
fn suggests_equality_then_sort_then_range() {
    let q = Query {
        filter: doc! { "age": { "$gt": 18 }, "status": "open", "region": { "$in": ["eu", "us"] } },
        sort: Some(doc! { "createdAt": -1 }),
        ..Query::default()
    };
    assert_eq!(
        suggest(&q).unwrap().keys,
        keys(&[("status", 1), ("region", 1), ("createdAt", -1), ("age", 1)])
    );
}

#[test]
fn deduplicates_fields() {
    let q = Query {
        filter: doc! { "status": "open", "age": { "$gte": 1 } },
        sort: Some(doc! { "status": 1, "age": -1 }),
        ..Query::default()
    };
    assert_eq!(
        suggest(&q).unwrap().keys,
        keys(&[("status", 1), ("age", -1)])
    );
}

#[test]
fn none_for_empty_query() {
    assert!(suggest(&Query::default()).is_none());
}

#[test]
fn command_renders_create_index() {
    let s = SuggestedIndex {
        keys: keys(&[("status", 1), ("createdAt", -1)]),
    };
    assert_eq!(
        s.command("orders"),
        r#"db.orders.createIndex({ "status": 1, "createdAt": -1 })"#
    );
}
