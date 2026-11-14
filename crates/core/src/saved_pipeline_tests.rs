use super::*;

fn stage(text: &str) -> SavedStage {
    SavedStage {
        text: text.to_string(),
        enabled: true,
    }
}

fn one(text: &str) -> Vec<SavedStage> {
    vec![stage(text)]
}

fn with_name(store: &SavedPipelines, conn: Uuid, name: &str, now: i64) -> SavedPipelines {
    store
        .saved(conn, name, None, one("{\"$match\": {}}"), now, false)
        .unwrap()
}

#[test]
fn a_name_is_trimmed_and_must_not_be_empty() {
    assert_eq!(validate_name("  daily totals  "), Ok("daily totals".into()));
    assert_eq!(validate_name(""), Err(SavedError::EmptyName));
    assert_eq!(validate_name(" \t\n "), Err(SavedError::EmptyName));
}

#[test]
fn a_name_may_have_eighty_characters_not_eighty_one() {
    assert!(validate_name(&"a".repeat(80)).is_ok());
    assert_eq!(validate_name(&"a".repeat(81)), Err(SavedError::NameTooLong));
    // Characters, not bytes: 80 multi-byte characters fit.
    assert!(validate_name(&"é".repeat(80)).is_ok());
    assert!(validate_name(&"日本語".repeat(27)).is_err());
}

#[test]
fn a_name_that_only_differs_in_case_or_spaces_is_taken() {
    let conn = Uuid::new_v4();
    let store = with_name(&SavedPipelines::default(), conn, "Daily Totals", 1);
    assert!(store.name_taken(conn, "daily totals", None));
    assert!(store.name_taken(conn, "  DAILY TOTALS ", None));
    assert!(!store.name_taken(conn, "daily", None));
    assert!(!store.name_taken(Uuid::new_v4(), "daily totals", None));
    assert_eq!(
        store.saved(conn, " daily TOTALS", None, one("x"), 2, false),
        Err(SavedError::NameTaken)
    );
}

#[test]
fn unicode_names_compare_case_insensitively() {
    let conn = Uuid::new_v4();
    let store = with_name(&SavedPipelines::default(), conn, "Ünïcode ÅNGSTRÖM", 1);
    assert!(store.name_taken(conn, "ünïcode ångström", None));
}

#[test]
fn replacing_keeps_the_id_and_updates_the_content() {
    let conn = Uuid::new_v4();
    let first = with_name(&SavedPipelines::default(), conn, "totals", 10);
    let id = first.list(conn)[0].id;
    let next = first
        .saved(
            conn,
            "TOTALS",
            Some("shop.orders".into()),
            one("new"),
            20,
            true,
        )
        .unwrap();
    assert_eq!(next.list(conn).len(), 1);
    let kept = &next.list(conn)[0];
    assert_eq!(kept.id, id);
    assert_eq!(kept.name, "totals", "the stored spelling stays");
    assert_eq!(kept.ns_hint.as_deref(), Some("shop.orders"));
    assert_eq!(kept.saved_unix, 20);
    assert_eq!(kept.stages, one("new"));
    // The original is untouched.
    assert_eq!(first.list(conn)[0].saved_unix, 10);
}

#[test]
fn replace_true_with_a_free_name_adds_a_new_pipeline() {
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(conn, "fresh", None, one("x"), 1, true)
        .unwrap();
    assert_eq!(store.list(conn).len(), 1);
}

#[test]
fn renaming_to_the_same_name_or_its_case_is_allowed() {
    let conn = Uuid::new_v4();
    let store = with_name(&SavedPipelines::default(), conn, "totals", 1);
    let id = store.list(conn)[0].id;
    assert!(store.renamed(conn, id, "totals").is_ok());
    let upper = store.renamed(conn, id, "  TOTALS ").unwrap();
    assert_eq!(upper.list(conn)[0].name, "TOTALS");
    assert_eq!(upper.list(conn)[0].saved_unix, 1);
}

#[test]
fn renaming_onto_another_pipelines_name_is_refused() {
    let conn = Uuid::new_v4();
    let store = with_name(
        &with_name(&SavedPipelines::default(), conn, "a", 1),
        conn,
        "b",
        2,
    );
    let id = store.find_by_name(conn, "a").unwrap().id;
    assert_eq!(store.renamed(conn, id, "B"), Err(SavedError::NameTaken));
    assert_eq!(store.renamed(conn, id, ""), Err(SavedError::EmptyName));
    assert_eq!(
        store.renamed(conn, id, &"x".repeat(81)),
        Err(SavedError::NameTooLong)
    );
}

#[test]
fn the_newest_pipeline_comes_first() {
    let conn = Uuid::new_v4();
    let store = with_name(
        &with_name(&SavedPipelines::default(), conn, "old", 1),
        conn,
        "new",
        2,
    );
    let names: Vec<&str> = store.list(conn).iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["new", "old"]);
    let resaved = store.saved(conn, "old", None, one("y"), 3, true).unwrap();
    let names: Vec<&str> = resaved.list(conn).iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["old", "new"]);
}

#[test]
fn at_most_two_hundred_pipelines_per_connection() {
    let conn = Uuid::new_v4();
    let mut store = SavedPipelines::default();
    for n in 0..MAX_PER_CONNECTION {
        store = with_name(&store, conn, &format!("p{n}"), n as i64);
    }
    assert_eq!(
        store.saved(conn, "one too many", None, one("x"), 999, false),
        Err(SavedError::TooMany)
    );
    // Replacing one is still fine, and another connection has its own room.
    assert!(store.saved(conn, "p0", None, one("x"), 999, true).is_ok());
    assert!(with_name(&store, Uuid::new_v4(), "p0", 1).list(conn).len() == MAX_PER_CONNECTION);
}

#[test]
fn a_pipeline_over_one_megabyte_of_stage_text_is_refused() {
    let conn = Uuid::new_v4();
    let big = vec![stage(&"a".repeat(MAX_STAGE_BYTES / 2 + 1)); 2];
    assert_eq!(
        SavedPipelines::default().saved(conn, "big", None, big, 1, false),
        Err(SavedError::TooLarge)
    );
    let fits = vec![stage(&"a".repeat(MAX_STAGE_BYTES))];
    assert!(
        SavedPipelines::default()
            .saved(conn, "fits", None, fits, 1, false)
            .is_ok()
    );
}

#[test]
fn deleting_removes_one_pipeline_and_leaves_the_original_alone() {
    let conn = Uuid::new_v4();
    let store = with_name(
        &with_name(&SavedPipelines::default(), conn, "a", 1),
        conn,
        "b",
        2,
    );
    let id = store.find_by_name(conn, "a").unwrap().id;
    let next = store.deleted(conn, id);
    assert_eq!(next.list(conn).len(), 1);
    assert!(next.find(conn, id).is_none());
    assert_eq!(store.list(conn).len(), 2);
    // Deleting an unknown id or from an unknown connection changes nothing.
    assert_eq!(next.deleted(conn, Uuid::new_v4()), next);
    assert_eq!(next.deleted(Uuid::new_v4(), id), next);
}

#[test]
fn a_connection_never_sees_another_connections_pipelines() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let store = with_name(&SavedPipelines::default(), a, "mine", 1);
    assert!(store.list(b).is_empty());
    let id = store.list(a)[0].id;
    assert!(store.find(b, id).is_none());
}

#[test]
fn removing_a_connection_drops_its_pipelines_only() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let store = with_name(&with_name(&SavedPipelines::default(), a, "x", 1), b, "y", 2);
    let next = store.without_connection(a);
    assert!(next.list(a).is_empty());
    assert_eq!(next.list(b).len(), 1);
    assert_eq!(store.list(a).len(), 1);
}

#[test]
fn copying_a_connection_copies_its_pipelines() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let store = with_name(&SavedPipelines::default(), a, "x", 1);
    let next = store.with_connection_copied(a, b);
    assert_eq!(next.list(a), next.list(b));
    // Unchanged when the source has none.
    assert_eq!(store.with_connection_copied(Uuid::new_v4(), b), store);
}

#[test]
fn the_store_round_trips_through_json() {
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(
            conn,
            "ünï",
            Some("db.coll".into()),
            vec![
                stage("{\"$match\": {}}"),
                SavedStage {
                    text: "{\"$limit\": 5}".into(),
                    enabled: false,
                },
            ],
            42,
            false,
        )
        .unwrap();
    let json = serde_json::to_string(&store).unwrap();
    assert!(json.contains("\"version\":1"));
    let back: SavedPipelines = serde_json::from_str(&json).unwrap();
    assert_eq!(back, store);
}

#[test]
fn an_empty_store_is_version_one_with_no_connections() {
    let json = serde_json::to_value(SavedPipelines::default()).unwrap();
    assert_eq!(json, serde_json::json!({"version": 1, "connections": {}}));
}

#[test]
fn a_newer_version_is_refused_and_unknown_fields_are_tolerated() {
    let newer = r#"{"version": 2, "connections": {}}"#;
    assert!(serde_json::from_str::<SavedPipelines>(newer).is_err());
    let extra = r#"{"version": 1, "connections": {}, "future": true}"#;
    assert!(serde_json::from_str::<SavedPipelines>(extra).is_ok());
    let missing_version = r#"{"connections": {}}"#;
    assert!(serde_json::from_str::<SavedPipelines>(missing_version).is_ok());
}

#[test]
fn error_variants_are_stable_names() {
    let names: Vec<&str> = [
        SavedError::EmptyName,
        SavedError::NameTooLong,
        SavedError::NameTaken,
        SavedError::TooMany,
        SavedError::TooLarge,
        SavedError::Unsupported,
    ]
    .iter()
    .map(SavedError::error_variant)
    .collect();
    assert_eq!(
        names,
        [
            "empty_name",
            "name_too_long",
            "name_taken",
            "too_many",
            "too_large",
            "unsupported"
        ]
    );
}
