//! What a file may hold: loading enforces the limits the API enforces, and a file that breaks
//! them is refused whole (the app then treats it like a corrupt file) instead of being trimmed.
use super::*;
use serde_json::{Value, json};

fn entry(id: Uuid, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "saved_unix": 1,
        "stages": [{"text": "{\"$match\": {}}", "enabled": true}],
    })
}

fn file(conn: Uuid, entries: Vec<Value>) -> String {
    json!({"version": 1, "connections": {conn.to_string(): entries}}).to_string()
}

fn load(text: &str) -> Result<SavedPipelines, serde_json::Error> {
    serde_json::from_str(text)
}

#[test]
fn a_file_within_the_limits_loads_as_written() {
    let conn = Uuid::new_v4();
    let entries = (0..MAX_PER_CONNECTION)
        .map(|i| entry(Uuid::new_v4(), &format!("p{i}")))
        .collect();
    let store = load(&file(conn, entries)).unwrap();
    assert_eq!(store.list(conn).len(), MAX_PER_CONNECTION);
}

#[test]
fn the_same_id_in_two_connections_is_fine() {
    let (a, b, id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let text = json!({"version": 1, "connections": {
        a.to_string(): [entry(id, "x")], b.to_string(): [entry(id, "x")],
    }})
    .to_string();
    assert!(load(&text).is_ok());
}

#[test]
fn names_are_trimmed_on_load() {
    let conn = Uuid::new_v4();
    let store = load(&file(conn, vec![entry(Uuid::new_v4(), "  totals \t")])).unwrap();
    assert_eq!(store.list(conn)[0].name, "totals");
}

#[test]
fn a_duplicate_id_in_one_connection_is_refused() {
    let (conn, id) = (Uuid::new_v4(), Uuid::new_v4());
    let text = file(conn, vec![entry(id, "a"), entry(id, "b")]);
    assert!(load(&text).is_err());
}

#[test]
fn names_that_differ_only_in_case_or_spaces_are_refused() {
    let conn = Uuid::new_v4();
    let text = file(
        conn,
        vec![
            entry(Uuid::new_v4(), "Totals"),
            entry(Uuid::new_v4(), " totals "),
        ],
    );
    assert!(load(&text).is_err());
}

#[test]
fn a_name_that_could_not_be_typed_is_refused() {
    let conn = Uuid::new_v4();
    for bad in [
        "".to_string(),
        "   ".to_string(),
        "x".repeat(10_000),
        "x".repeat(MAX_NAME_CHARS + 1),
        "two\nlines".to_string(),
        "nul\0".to_string(),
    ] {
        let text = file(conn, vec![entry(Uuid::new_v4(), &bad)]);
        assert!(load(&text).is_err(), "{} chars", bad.len());
    }
}

#[test]
fn more_than_the_most_per_connection_is_refused() {
    let conn = Uuid::new_v4();
    let entries: Vec<Value> = (0..=MAX_PER_CONNECTION)
        .map(|i| entry(Uuid::new_v4(), &format!("p{i}")))
        .collect();
    assert_eq!(entries.len(), 201);
    assert!(load(&file(conn, entries)).is_err());
}

#[test]
fn stage_text_over_the_limit_is_refused() {
    let conn = Uuid::new_v4();
    let mut big = entry(Uuid::new_v4(), "big");
    big["stages"] = json!([{"text": "x".repeat(MAX_STAGE_BYTES + 1), "enabled": true}]);
    assert!(load(&file(conn, vec![big])).is_err());
}

#[test]
fn a_refusal_is_content_free() {
    let conn = Uuid::new_v4();
    let text = file(conn, vec![entry(Uuid::new_v4(), "secret\nname")]);
    let message = load(&text).unwrap_err().to_string();
    assert!(message.contains("invalid_name"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}
