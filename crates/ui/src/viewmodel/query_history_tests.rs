use super::*;

fn conn() -> Uuid {
    Uuid::from_u128(1)
}

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

fn store() -> QueryHistory {
    let s = QueryHistory::default()
        .recorded(
            conn(),
            "shop",
            "orders",
            text("{ \"status\": \"open\" }"),
            100,
        )
        .unwrap()
        .recorded(
            conn(),
            "shop",
            "orders",
            text("{ \"total\": { \"$gt\": 5 } }"),
            200,
        )
        .unwrap()
        .recorded(conn(), "shop", "users", text("{ \"name\": \"Ann\" }"), 300)
        .unwrap();
    s.starred(
        conn(),
        "shop",
        "orders",
        "Open Orders",
        text("{ \"status\": \"open\" }"),
        5,
    )
    .unwrap()
    .starred(
        conn(),
        "shop",
        "users",
        "Annas",
        text("{ \"name\": \"Ann\" }"),
        6,
    )
    .unwrap()
}

const ORDERS: (Uuid, &str, &str) = (Uuid::from_u128(1), "shop", "orders");

#[test]
fn an_empty_search_lists_the_collections_own_entries_only() {
    let s = store();
    let list = listing(&s, ORDERS, "");
    assert_eq!(list.favourites.len(), 1);
    assert_eq!(list.favourites[0].name, "Open Orders");
    assert_eq!(list.recent.len(), 2);
    assert_eq!(
        list.recent[0].1.text.filter,
        "{ \"total\": { \"$gt\": 5 } }"
    );
    assert_eq!((list.favourites_total, list.recent_total), (1, 2));
    assert_eq!(list.matches(), 3);
}

#[test]
fn recent_rows_carry_their_position_in_the_whole_list() {
    let s = store();
    let list = listing(&s, ORDERS, "status");
    assert_eq!(list.recent.len(), 1);
    assert_eq!(list.recent[0].0, 1, "the older entry is second in the list");
}

#[test]
fn search_matches_names_and_text_ignoring_case() {
    let s = store();
    assert_eq!(
        listing(&s, ORDERS, "open orders").favourites.len(),
        1,
        "name"
    );
    assert_eq!(listing(&s, ORDERS, "OPEN").recent.len(), 1, "text");
    assert_eq!(listing(&s, ORDERS, "  $GT ").recent.len(), 1, "trimmed");
    assert_eq!(listing(&s, ORDERS, "zzz").matches(), 0);
    // The other collection's entries are never listed, whatever the search.
    assert_eq!(listing(&s, ORDERS, "ann").matches(), 0);
}

#[test]
fn search_also_looks_in_sort_and_projection() {
    let mut t = text("{}");
    t.sort = "{ \"createdAt\": -1 }".into();
    t.projection = "{ \"secretField\": 1 }".into();
    let s = QueryHistory::default()
        .recorded(conn(), "shop", "orders", t, 1)
        .unwrap();
    assert_eq!(listing(&s, ORDERS, "createdat").recent.len(), 1);
    assert_eq!(listing(&s, ORDERS, "secretfield").recent.len(), 1);
}

#[test]
fn a_row_shows_the_filter_on_one_line() {
    let t = text("{\n  \"a\": 1,\n\t\"b\": 2\n}");
    assert_eq!(one_line(&t, "none"), "{ \"a\": 1, \"b\": 2 }");
    let only_sort = FindText {
        sort: "{ \"a\": 1 }".into(),
        ..FindText::default()
    };
    assert_eq!(one_line(&only_sort, "none"), "none");
}

#[test]
fn when_it_ran_uses_the_shared_age_phrases() {
    let i18n = I18n::english();
    assert_eq!(ran_phrase(1000, 1030, &i18n), "just now");
    assert_eq!(ran_phrase(1000, 1000 + 5 * 60, &i18n), "5 min ago");
    assert_eq!(ran_phrase(1000, 1000 + 3 * 3600, &i18n), "3 h ago");
    assert_eq!(
        ran_phrase(5000, 1000, &i18n),
        "just now",
        "never in the future"
    );
}

#[test]
fn the_match_count_is_a_sentence() {
    let i18n = I18n::english();
    assert_eq!(matches_phrase(0, &i18n), "No matches");
    assert_eq!(matches_phrase(1, &i18n), "1 match");
    assert_eq!(matches_phrase(4, &i18n), "4 matches");
}

#[test]
fn a_name_problem_is_told_before_confirming() {
    let s = store();
    let ns = ORDERS;
    assert_eq!(
        name_problem(&s, ns, "  ", None),
        Some(SavedError::EmptyName)
    );
    assert_eq!(
        name_problem(&s, ns, "open ORDERS", None),
        Some(SavedError::NameTaken)
    );
    assert_eq!(name_problem(&s, ns, "fresh", None), None);
    let own = s.favourites(conn())[1].id;
    assert_eq!(s.favourites(conn())[1].name, "Open Orders");
    assert_eq!(name_problem(&s, ns, "open orders", Some(own)), None);
    // Another collection's favourite name is free here.
    assert_eq!(name_problem(&s, ns, "Annas", None), None);
}

#[test]
fn every_refusal_has_a_message_key() {
    for e in [
        SavedError::EmptyName,
        SavedError::InvalidName,
        SavedError::NameTooLong,
        SavedError::NameTaken,
        SavedError::TooMany,
        SavedError::TooLarge,
        SavedError::Unsupported,
        SavedError::InvalidOptions,
    ] {
        assert!(error_key(e).starts_with("history.error."));
    }
}

#[test]
fn debug_output_never_shows_text_or_names() {
    let ui = HistoryUi {
        search: "SECRET".into(),
        dialog: Some(Dialog::Star {
            text: Box::new(text("SECRET")),
            name: "SECRET".into(),
        }),
        error: None,
    };
    let shown = format!(
        "{ui:?} {:?} {:?}",
        HistoryAction::Search("SECRET".into()),
        HistoryAction::NameEdited("SECRET".into())
    );
    assert!(!shown.contains("SECRET"), "{shown}");
}
