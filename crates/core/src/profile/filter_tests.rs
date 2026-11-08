use bson::{Document, doc};

use super::entry_has_filter;
use super::entry_tests::{entry, fixture};
use super::parse_entry;

fn with(op: &str, command: Document) -> super::ProfileEntry {
    let mut d = fixture("collscan");
    d.insert("op", op);
    d.insert("command", command);
    parse_entry(&d).unwrap()
}

#[test]
fn has_filter_for_find_update_delete_aggregate_count() {
    assert!(entry_has_filter(&entry("collscan")));
    assert!(entry_has_filter(&entry("aggregate_match")));
    let update =
        doc! { "update": "orders", "updates": [{ "q": { "a": 1 }, "u": { "$set": { "b": 2 } } }] };
    assert!(entry_has_filter(&with("update", update)));
    let delete = doc! { "delete": "orders", "deletes": [{ "q": { "a": 1 }, "limit": 0 }] };
    assert!(entry_has_filter(&with("remove", delete)));
    let count = doc! { "count": "orders", "query": { "a": 1 } };
    assert!(entry_has_filter(&with("command", count)));
    let find = doc! { "find": "orders", "filter": { "a": 1 } };
    assert!(entry_has_filter(&with("command", find)));
}

#[test]
fn no_filter_for_empty_filter_and_unsupported_ops() {
    let empty = doc! { "find": "orders", "filter": {} };
    assert!(!entry_has_filter(&with("query", empty)));
    let no_key = doc! { "find": "orders" };
    assert!(!entry_has_filter(&with("query", no_key)));
    let update = doc! { "update": "orders", "updates": [{ "q": {}, "u": { "$set": { "b": 2 } } }] };
    assert!(!entry_has_filter(&with("update", update)));
    assert!(!entry_has_filter(&entry("insert")));
    assert!(!entry_has_filter(&entry("getmore")));
    assert!(!entry_has_filter(&entry("index_stats_aggregation")));
}
