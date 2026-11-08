use bson::{Bson, Document, doc};

use super::{OWN_APP_NAME, ProfileEntry, is_own_traffic, parse_entry};
use crate::ejson::editor_json_to_doc;
use crate::error::CoreError;

pub(super) fn fixture(name: &str) -> Document {
    let text = match name {
        "collscan" => include_str!("fixtures/collscan.json"),
        "ixscan" => include_str!("fixtures/ixscan.json"),
        "sort" => include_str!("fixtures/sort.json"),
        "inefficient" => include_str!("fixtures/inefficient.json"),
        "own_traffic" => include_str!("fixtures/own_traffic.json"),
        "own_traffic_prefixed" => include_str!("fixtures/own_traffic_prefixed.json"),
        "users_app_lookalike" => include_str!("fixtures/users_app_lookalike.json"),
        "getmore" => include_str!("fixtures/getmore.json"),
        "system_profile_read" => include_str!("fixtures/system_profile_read.json"),
        "index_stats_aggregation" => include_str!("fixtures/index_stats_aggregation.json"),
        "explain_command" => include_str!("fixtures/explain_command.json"),
        "aggregate_match" => include_str!("fixtures/aggregate_match.json"),
        "insert" => include_str!("fixtures/insert.json"),
        other => panic!("unknown fixture {other}"),
    };
    editor_json_to_doc(text).expect("fixture parses")
}

pub(super) fn entry(name: &str) -> ProfileEntry {
    parse_entry(&fixture(name)).expect("fixture is a profile entry")
}

fn minimal() -> Document {
    doc! { "ns": "shop.orders", "ts": bson::DateTime::from_millis(5_999) }
}

#[test]
fn parses_a_collscan_fixture() {
    let e = entry("collscan");
    assert_eq!(e.ts_secs, 1_762_500_000);
    assert_eq!(e.ts_millis.div_euclid(1000), e.ts_secs);
    assert_eq!(e.ns, "shop.orders");
    assert_eq!(e.op, "query");
    assert_eq!(e.plan_summary, "COLLSCAN");
    assert_eq!(
        (e.keys_examined, e.docs_examined, e.returned),
        (0, 5000, 12)
    );
    assert_eq!(e.millis, 41);
    assert!(!e.has_sort_stage);
    assert_eq!(e.app_name.as_deref(), Some("orders-service"));
    assert_eq!(e.command.get_str("find"), Ok("orders"));
}

#[test]
fn parses_numbers_of_every_width() {
    let int32 = parse_entry(&doc! {
        "ns": "a.b", "ts": bson::DateTime::from_millis(1_000),
        "keysExamined": 7_i32, "docsExamined": 8_i32, "nreturned": 9_i32, "millis": 10_i32,
    })
    .unwrap();
    let int64 = parse_entry(&doc! {
        "ns": "a.b", "ts": bson::DateTime::from_millis(1_000),
        "keysExamined": 7_i64, "docsExamined": 8_i64, "nreturned": 9_i64, "millis": 10_i64,
    })
    .unwrap();
    let double = parse_entry(&doc! {
        "ns": "a.b", "ts": bson::DateTime::from_millis(1_000),
        "keysExamined": 7.0_f64, "docsExamined": 8.0_f64, "nreturned": 9.0_f64, "millis": 10.0_f64,
    })
    .unwrap();
    assert_eq!(int32, int64);
    assert_eq!(int64, double);
    assert_eq!(
        (
            int32.keys_examined,
            int32.docs_examined,
            int32.returned,
            int32.millis
        ),
        (7, 8, 9, 10)
    );
}

#[test]
fn negative_and_nonsense_numbers_read_as_zero() {
    let mut d = minimal();
    d.insert("docsExamined", -4_i32);
    d.insert("nreturned", "many");
    d.insert("millis", f64::NAN);
    let e = parse_entry(&d).unwrap();
    assert_eq!((e.docs_examined, e.returned, e.millis), (0, 0, 0));
}

#[test]
fn optional_fields_default() {
    let e = parse_entry(&minimal()).unwrap();
    assert_eq!((e.ts_secs, e.ts_millis), (5, 5_999));
    assert_eq!(e.op, "");
    assert_eq!(e.plan_summary, "");
    assert!(!e.has_sort_stage);
    assert_eq!(e.app_name, None);
    assert!(e.command.is_empty());
}

#[test]
fn insert_has_no_plan_summary() {
    let e = entry("insert");
    assert_eq!(e.plan_summary, "");
    assert_eq!(e.op, "insert");
    assert_eq!(e.millis, 2);
}

#[test]
fn missing_ts_is_an_error() {
    let err = parse_entry(&doc! { "ns": "a.b" }).unwrap_err();
    assert!(matches!(err, CoreError::Database(_)), "{err:?}");
    let wrong = parse_entry(&doc! { "ns": "a.b", "ts": "yesterday" }).unwrap_err();
    assert!(matches!(wrong, CoreError::Database(_)));
}

#[test]
fn missing_ns_is_an_error() {
    let err = parse_entry(&doc! { "ts": bson::DateTime::from_millis(1) }).unwrap_err();
    assert!(matches!(err, CoreError::Database(_)), "{err:?}");
}

#[test]
fn a_non_document_command_is_tolerated() {
    let mut d = minimal();
    d.insert("command", Bson::String("odd".into()));
    assert!(parse_entry(&d).unwrap().command.is_empty());
}

#[test]
fn own_traffic_by_app_name_suffix() {
    assert_eq!(OWN_APP_NAME, "curlybrace");
    assert!(is_own_traffic(&entry("own_traffic")));
}

#[test]
fn own_traffic_with_user_app_name_suffix() {
    assert!(is_own_traffic(&entry("own_traffic_prefixed")));
}

#[test]
fn users_own_app_is_not_own_traffic() {
    assert!(!is_own_traffic(&entry("users_app_lookalike")));
    assert!(!is_own_traffic(&entry("collscan")));
    assert!(!is_own_traffic(&entry("insert")));
    assert!(!is_own_traffic(&entry("aggregate_match")));
    assert!(!is_own_traffic(&entry("getmore")));
}

#[test]
fn system_profile_reads_are_own_traffic() {
    assert!(is_own_traffic(&entry("system_profile_read")));
}

#[test]
fn index_stats_aggregation_is_own_traffic() {
    assert!(is_own_traffic(&entry("index_stats_aggregation")));
}

#[test]
fn explain_commands_are_own_traffic() {
    assert!(is_own_traffic(&entry("explain_command")));
}

#[test]
fn an_index_stats_stage_after_a_match_is_still_own_traffic() {
    let mut e = entry("aggregate_match");
    e.command = doc! { "aggregate": "orders", "pipeline": [{ "$match": { "a": 1 } }, { "$indexStats": {} }] };
    assert!(is_own_traffic(&e));
}

#[test]
fn a_ns_that_merely_mentions_system_profile_is_not_own_traffic() {
    let mut e = entry("collscan");
    e.ns = "shop.system.profile_archive".into();
    assert!(!is_own_traffic(&e));
}

fn named(app: Option<&str>) -> ProfileEntry {
    let mut e = entry("collscan");
    e.app_name = app.map(str::to_owned);
    e
}

#[test]
fn app_name_boundaries() {
    assert!(is_own_traffic(&named(Some("curlybrace"))));
    assert!(is_own_traffic(&named(Some("myapp curlybrace"))));
    for not_ours in [
        None,
        Some(""),
        Some("curlybrace-2"),
        // Trailing space: the name does not end with the token, so it is the user's.
        Some("x curlybrace "),
        Some("Curlybrace"),
        Some("mycurlybrace"),
    ] {
        assert!(!is_own_traffic(&named(not_ours)), "{not_ours:?}");
    }
}

#[test]
fn continuations_are_detected_by_op_and_by_command_key() {
    assert!(entry("getmore").is_continuation());
    let mut e = entry("collscan");
    assert!(!e.is_continuation());
    for op in ["getmore", "getMore", "GETMORE"] {
        e.op = op.into();
        assert!(e.is_continuation(), "{op}");
    }
    e.op = "command".into();
    e.command = doc! { "getMore": 7_i64, "collection": "orders" };
    assert!(e.is_continuation());
    e.command = doc! { "find": "orders", "getMore": 1 };
    assert!(!e.is_continuation());
    assert!(!entry("insert").is_continuation());
}

#[test]
fn debug_output_never_contains_command_values() {
    let mut e = entry("collscan");
    e.command = doc! { "find": "orders", "filter": { "email": "zebra-secret-7731@example.com" } };
    let shown = format!("{e:?}");
    assert!(!shown.contains("zebra-secret-7731"), "{shown}");
    assert!(!shown.contains("email"), "{shown}");
    assert!(shown.contains("shop.orders"));
    assert!(shown.contains("COLLSCAN"));
    assert!(shown.contains("<redacted 2 keys>"), "{shown}");
}
