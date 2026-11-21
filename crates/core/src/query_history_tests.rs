use super::*;
use crate::pipeline_options::PipelineOptions;
use serde_json::json;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

fn ns() -> (Uuid, &'static str, &'static str) {
    (Uuid::from_u128(1), "shop", "orders")
}

fn record(store: &QueryHistory, filter: &str, now: i64) -> QueryHistory {
    let (c, db, coll) = ns();
    store.recorded(c, db, coll, text(filter), now).unwrap()
}

fn star(store: &QueryHistory, name: &str, filter: &str) -> Result<QueryHistory, SavedError> {
    let (c, db, coll) = ns();
    store.starred(c, db, coll, name, text(filter), 7)
}

#[test]
fn a_recorded_query_comes_first_and_the_original_store_is_untouched() {
    let (c, db, coll) = ns();
    let empty = QueryHistory::default();
    let one = record(&empty, "{\"a\":1}", 1);
    let two = record(&one, "{\"b\":2}", 2);
    assert!(empty.history(c, db, coll).is_empty());
    assert_eq!(one.history(c, db, coll).len(), 1);
    let filters: Vec<&str> = two
        .history(c, db, coll)
        .iter()
        .map(|e| e.text.filter.as_str())
        .collect();
    assert_eq!(filters, ["{\"b\":2}", "{\"a\":1}"]);
}

#[test]
fn a_repeat_moves_to_the_front_and_updates_its_time_without_a_second_copy() {
    let (c, db, coll) = ns();
    let s = record(
        &record(&record(&QueryHistory::default(), "a", 1), "b", 2),
        "a",
        9,
    );
    let list = s.history(c, db, coll);
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].text.filter.as_str(), list[0].ran_unix), ("a", 9));
    assert_eq!(list[1].text.filter, "b");
}

#[test]
fn the_same_filter_with_other_options_is_a_different_entry() {
    let (c, db, coll) = ns();
    let mut with_limit = text("a");
    with_limit.limit = "5".into();
    let s = record(&QueryHistory::default(), "a", 1)
        .recorded(c, db, coll, with_limit, 2)
        .unwrap();
    assert_eq!(s.history(c, db, coll).len(), 2);
}

#[test]
fn history_keeps_the_newest_hundred_per_namespace() {
    let (c, db, coll) = ns();
    let mut s = QueryHistory::default();
    for i in 0..(MAX_HISTORY_PER_NAMESPACE + 5) {
        s = record(&s, &format!("{{\"n\":{i}}}"), i as i64);
    }
    let list = s.history(c, db, coll);
    assert_eq!(list.len(), MAX_HISTORY_PER_NAMESPACE);
    assert_eq!(list[0].text.filter, "{\"n\":104}");
    assert_eq!(list[99].text.filter, "{\"n\":5}");
}

#[test]
fn namespaces_and_connections_keep_separate_histories() {
    let (c, _, _) = ns();
    let other = Uuid::from_u128(2);
    let s = record(&QueryHistory::default(), "a", 1)
        .recorded(c, "shop", "users", text("u"), 2)
        .unwrap()
        .recorded(other, "shop", "orders", text("o"), 3)
        .unwrap();
    assert_eq!(s.history(c, "shop", "orders").len(), 1);
    assert_eq!(s.history(c, "shop", "users")[0].text.filter, "u");
    assert_eq!(s.history(other, "shop", "orders")[0].text.filter, "o");
    assert_eq!(s.history_len(), 3);
}

#[test]
fn a_blank_bar_is_not_recorded() {
    let (c, db, coll) = ns();
    let s = QueryHistory::default()
        .recorded(c, db, coll, text("  \n"), 1)
        .unwrap();
    assert_eq!(s, QueryHistory::default());
}

#[test]
fn a_query_over_the_entry_limit_is_refused() {
    let (c, db, coll) = ns();
    let big = text(&"x".repeat(MAX_ENTRY_BYTES + 1));
    assert_eq!(
        QueryHistory::default().recorded(c, db, coll, big, 1),
        Err(SavedError::TooLarge)
    );
    let exact = text(&"x".repeat(MAX_ENTRY_BYTES));
    assert!(
        QueryHistory::default()
            .recorded(c, db, coll, exact, 1)
            .is_ok()
    );
}

#[test]
fn a_favourite_is_a_named_copy_and_history_is_kept() {
    let (c, db, coll) = ns();
    let s = star(
        &record(&QueryHistory::default(), "a", 1),
        "  My query ",
        "a",
    )
    .unwrap();
    let favs = s.favourites(c);
    assert_eq!(favs.len(), 1);
    assert_eq!(
        (favs[0].name.as_str(), favs[0].db.as_str()),
        ("My query", "shop")
    );
    assert_eq!(favs[0].text.filter, "a");
    assert_eq!(s.history(c, db, coll).len(), 1);
}

#[test]
fn favourite_names_are_unique_per_connection_and_namespace_ignoring_case() {
    let (c, _, _) = ns();
    let s = star(&QueryHistory::default(), "Big", "a").unwrap();
    assert_eq!(star(&s, " big ", "b"), Err(SavedError::NameTaken));
    // Another collection of the same connection may reuse the name.
    let other = s.starred(c, "shop", "users", "Big", text("b"), 1).unwrap();
    assert_eq!(other.favourites(c).len(), 2);
}

#[test]
fn favourite_names_follow_the_pipeline_name_rules() {
    let empty = QueryHistory::default();
    assert_eq!(star(&empty, "  ", "a"), Err(SavedError::EmptyName));
    assert_eq!(star(&empty, "a\nb", "a"), Err(SavedError::InvalidName));
    let long = "x".repeat(MAX_NAME_CHARS + 1);
    assert_eq!(star(&empty, &long, "a"), Err(SavedError::NameTooLong));
}

#[test]
fn a_connection_keeps_at_most_a_hundred_favourites() {
    let (c, _, _) = ns();
    let mut s = QueryHistory::default();
    for i in 0..MAX_FAVOURITES_PER_CONNECTION {
        s = star(&s, &format!("q{i}"), "a").unwrap();
    }
    assert_eq!(star(&s, "one more", "a"), Err(SavedError::TooMany));
    assert_eq!(s.favourites(c).len(), MAX_FAVOURITES_PER_CONNECTION);
}

#[test]
fn a_favourite_over_the_entry_limit_is_refused() {
    let big = "x".repeat(MAX_ENTRY_BYTES + 1);
    assert_eq!(
        star(&QueryHistory::default(), "n", &big),
        Err(SavedError::TooLarge)
    );
}

#[test]
fn rename_checks_the_rules_and_keeps_the_place_and_text() {
    let (c, _, _) = ns();
    let s = star(
        &star(&QueryHistory::default(), "one", "a").unwrap(),
        "two",
        "b",
    )
    .unwrap();
    let two = s.favourites(c)[0].id;
    let renamed = s.renamed(c, two, " three ").unwrap();
    assert_eq!(renamed.favourites(c)[0].name, "three");
    assert_eq!(renamed.favourites(c)[0].text.filter, "b");
    assert_eq!(s.renamed(c, two, "ONE"), Err(SavedError::NameTaken));
    assert_eq!(s.renamed(c, two, ""), Err(SavedError::EmptyName));
    // Keeping its own name (another case) is allowed.
    assert!(s.renamed(c, two, "TWO").is_ok());
    // An unknown id changes nothing.
    assert_eq!(s.renamed(c, Uuid::from_u128(99), "x").unwrap(), s);
}

#[test]
fn unstar_removes_only_that_favourite() {
    let (c, _, _) = ns();
    let s = star(
        &star(&QueryHistory::default(), "one", "a").unwrap(),
        "two",
        "b",
    )
    .unwrap();
    let id = s.favourites(c)[0].id;
    let next = s.unstarred(c, id);
    assert_eq!(next.favourites(c).len(), 1);
    assert_eq!(next.favourites(c)[0].name, "one");
    assert_eq!(s.unstarred(c, Uuid::from_u128(99)), s);
}

#[test]
fn clearing_one_namespace_leaves_the_rest_and_all_favourites() {
    let (c, db, coll) = ns();
    let s = record(&QueryHistory::default(), "a", 1)
        .recorded(c, "shop", "users", text("u"), 2)
        .unwrap();
    let s = star(&s, "keep", "a").unwrap();
    let cleared = s.cleared(c, db, coll);
    assert!(cleared.history(c, db, coll).is_empty());
    assert_eq!(cleared.history(c, "shop", "users").len(), 1);
    assert_eq!(cleared.favourites(c).len(), 1);
}

#[test]
fn erasing_history_keeps_favourites() {
    let (c, db, coll) = ns();
    let s = star(&record(&QueryHistory::default(), "a", 1), "keep", "a").unwrap();
    let erased = s.history_erased();
    assert!(erased.history(c, db, coll).is_empty());
    assert_eq!(erased.history_len(), 0);
    assert_eq!(erased.favourites(c).len(), 1);
}

#[test]
fn a_deleted_connection_loses_history_and_favourites() {
    let (c, db, coll) = ns();
    let s = star(&record(&QueryHistory::default(), "a", 1), "n", "a").unwrap();
    let gone = s.without_connection(c);
    assert!(gone.history(c, db, coll).is_empty());
    assert!(gone.favourites(c).is_empty());
    assert_eq!(gone, QueryHistory::default());
}

#[test]
fn a_duplicate_copies_favourites_but_not_history() {
    let (c, db, coll) = ns();
    let copy = Uuid::from_u128(5);
    let s = star(&record(&QueryHistory::default(), "a", 1), "n", "a").unwrap();
    let dup = s.with_connection_copied(c, copy);
    assert_eq!(dup.favourites(copy), s.favourites(c));
    assert!(dup.history(copy, db, coll).is_empty());
    assert_eq!(dup.history(c, db, coll).len(), 1);
    // A source with no favourites changes nothing.
    assert_eq!(
        dup.with_connection_copied(Uuid::from_u128(77), Uuid::from_u128(78)),
        dup
    );
}

#[test]
fn dropping_the_oldest_history_entry_goes_across_namespaces() {
    let (c, db, coll) = ns();
    let s = record(&QueryHistory::default(), "old", 1)
        .recorded(c, "shop", "users", text("mid"), 5)
        .unwrap();
    let s = record(&s, "new", 9);
    let trimmed = s.oldest_history_dropped().unwrap();
    assert_eq!(trimmed.history_len(), 2);
    assert_eq!(trimmed.history(c, db, coll)[0].text.filter, "new");
    assert_eq!(trimmed.history(c, db, coll).len(), 1);
    assert_eq!(QueryHistory::default().oldest_history_dropped(), None);
    // Favourites are never dropped for room.
    let only_fav = star(&QueryHistory::default(), "n", "a").unwrap();
    assert_eq!(only_fav.oldest_history_dropped(), None);
}

#[test]
fn the_file_round_trips() {
    let (c, db, coll) = ns();
    let mut with_opts = text("a");
    with_opts.options = PipelineOptions {
        comment: Some("c".into()),
        ..PipelineOptions::default()
    };
    let s = QueryHistory::default()
        .recorded(c, db, coll, with_opts, 3)
        .unwrap();
    let s = star(&s, "fav", "z").unwrap();
    let json = serde_json::to_string(&s).unwrap();
    let back: QueryHistory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn a_newer_version_is_refused_on_load() {
    let r: Result<QueryHistory, _> =
        serde_json::from_value(json!({"version": VERSION + 1, "history": [], "favourites": []}));
    assert!(r.is_err());
}

fn group(c: Uuid, n: usize) -> serde_json::Value {
    let entries: Vec<_> = (0..n)
        .map(|i| {
            json!({"text": {"filter": format!("{i}"), "sort": "", "projection": "",
                "skip": "", "limit": ""}, "ran_unix": i})
        })
        .collect();
    json!({"connection": c, "db": "d", "collection": "c", "entries": entries})
}

#[test]
fn a_file_over_the_history_cap_is_refused_whole() {
    let c = Uuid::from_u128(1);
    let ok = json!({"version": 1, "history": [group(c, MAX_HISTORY_PER_NAMESPACE)],
        "favourites": []});
    assert!(serde_json::from_value::<QueryHistory>(ok).is_ok());
    let bad = json!({"version": 1, "history": [group(c, MAX_HISTORY_PER_NAMESPACE + 1)],
        "favourites": []});
    assert!(serde_json::from_value::<QueryHistory>(bad).is_err());
}

fn fav(name: &str, id: u128) -> serde_json::Value {
    json!({"id": Uuid::from_u128(id), "name": name, "db": "d", "collection": "c",
        "text": {"filter": "a", "sort": "", "projection": "", "skip": "", "limit": ""},
        "saved_unix": 1})
}

#[test]
fn a_file_with_a_repeated_favourite_name_or_id_or_a_bad_name_is_refused() {
    let c = Uuid::from_u128(1);
    let load = |favs: Vec<serde_json::Value>| {
        serde_json::from_value::<QueryHistory>(json!({"version": 1, "history": [],
            "favourites": [{"connection": c, "items": favs}]}))
    };
    assert!(load(vec![fav("a", 1), fav("b", 2)]).is_ok());
    assert!(load(vec![fav("a", 1), fav("A", 2)]).is_err());
    assert!(load(vec![fav("a", 1), fav("b", 1)]).is_err());
    assert!(load(vec![fav("", 1)]).is_err());
    assert!(load(vec![fav("a\nb", 1)]).is_err());
}

#[test]
fn a_file_with_an_oversized_entry_or_bad_options_is_refused() {
    let c = Uuid::from_u128(1);
    let big = "x".repeat(MAX_ENTRY_BYTES + 1);
    let bad = json!({"version": 1, "history": [{"connection": c, "db": "d", "collection": "c",
        "entries": [{"text": {"filter": big, "sort": "", "projection": "", "skip": "",
        "limit": ""}, "ran_unix": 1}]}], "favourites": []});
    assert!(serde_json::from_value::<QueryHistory>(bad).is_err());
    let opts = json!({"version": 1, "history": [{"connection": c, "db": "d", "collection": "c",
        "entries": [{"text": {"filter": "a", "sort": "", "projection": "", "skip": "",
        "limit": "", "options": {"maxTimeMS": 0}}, "ran_unix": 1}]}], "favourites": []});
    assert!(serde_json::from_value::<QueryHistory>(opts).is_err());
}

#[test]
fn debug_output_never_contains_query_text_or_names() {
    let s = star(
        &record(&QueryHistory::default(), "SECRET-FILTER", 1),
        "SECRET-NAME",
        "SECRET-FILTER",
    )
    .unwrap();
    let shown = format!(
        "{s:?} {:?} {:?}",
        s.favourites(ns().0),
        s.history(ns().0, "shop", "orders")
    );
    for needle in ["SECRET", "shop", "orders"] {
        assert!(!shown.contains(needle), "{needle} leaked: {shown}");
    }
}
