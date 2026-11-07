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

fn command_of(collection: &str, fields: &[(&str, i32)]) -> String {
    SuggestedIndex { keys: keys(fields) }.command(collection)
}

/// Splits `db.getCollection(<lit>).createIndex({ <body> })` into the two parts.
fn parts(cmd: &str) -> (&str, &str) {
    let rest = cmd.strip_prefix("db.getCollection(").expect("prefix");
    let rest = rest.strip_suffix(" })").expect("suffix");
    rest.split_once(").createIndex({ ").expect("middle")
}

fn collection_literal(cmd: &str) -> String {
    serde_json::from_str(parts(cmd).0).expect("collection is one JSON string")
}

fn field_names(cmd: &str) -> Vec<String> {
    let doc: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&format!("{{ {} }}", parts(cmd).1)).expect("keys parse as JSON");
    doc.keys().cloned().collect()
}

#[test]
fn command_quotes_ordinary_names() {
    assert_eq!(
        command_of("orders", &[("status", 1), ("created", -1)]),
        r#"db.getCollection("orders").createIndex({ "status": 1, "created": -1 })"#
    );
}

#[test]
fn command_escapes_quotes_and_backslashes_in_collection() {
    let cmd = command_of(r#"a"b\c"#, &[("f", 1)]);
    assert_eq!(
        cmd,
        r#"db.getCollection("a\"b\\c").createIndex({ "f": 1 })"#
    );
    assert_eq!(collection_literal(&cmd), r#"a"b\c"#);
}

#[test]
fn command_escapes_quotes_in_field_names() {
    let cmd = command_of("c", &[(r#"a"b"#, 1), ("x.y", -1)]);
    assert_eq!(field_names(&cmd), vec![r#"a"b"#, "x.y"]);
}

#[test]
fn command_neutralises_injection_attempts() {
    let collection = r#"x");db.dropDatabase();//"#;
    let field = "a: 1 }); db.dropDatabase(); ({";
    let cmd = command_of(collection, &[(field, 1), ("ok", 1)]);
    assert_eq!(collection_literal(&cmd), collection);
    assert_eq!(field_names(&cmd), vec![field, "ok"]);
    assert!(cmd.ends_with(" })"));
}

#[test]
fn command_escapes_line_separators() {
    let cmd = command_of("a\u{2028}b", &[("c\u{2029}d\n", 1)]);
    assert!(!cmd.contains('\u{2028}') && !cmd.contains('\u{2029}') && !cmd.contains('\n'));
    assert_eq!(collection_literal(&cmd), "a\u{2028}b");
    assert_eq!(field_names(&cmd), vec!["c\u{2029}d\n"]);
}
