//! The fixtures captured from a real server (mongod 9.0.4) by `tests/local_profiler.rs` with
//! `CURLYBRACE_CAPTURE=1`: the profile code must read, judge and shape them as it does the
//! hand-written fixtures. Refresh them with that mode; never edit them by hand.

use bson::Document;

use super::{ProfileEntry, classify_entry, entry_has_filter, is_own_traffic, parse_entry};
use crate::ejson::editor_json_to_doc;
use crate::explain::VerdictKind;
use crate::shape::from_profile_command;

fn raw(name: &str) -> Document {
    let text = match name {
        "aggregate" => include_str!("fixtures/real_aggregate.json"),
        "app_find" => include_str!("fixtures/real_app_find.json"),
        "count" => include_str!("fixtures/real_count.json"),
        "delete" => include_str!("fixtures/real_delete.json"),
        "distinct" => include_str!("fixtures/real_distinct.json"),
        "explain" => include_str!("fixtures/real_explain.json"),
        "find_and_modify" => include_str!("fixtures/real_find_and_modify.json"),
        "find_collscan" => include_str!("fixtures/real_find_collscan.json"),
        "find_inefficient" => include_str!("fixtures/real_find_inefficient.json"),
        "find_ixscan" => include_str!("fixtures/real_find_ixscan.json"),
        "find_no_filter" => include_str!("fixtures/real_find_no_filter.json"),
        "find_sort" => include_str!("fixtures/real_find_sort.json"),
        "getmore" => include_str!("fixtures/real_getmore.json"),
        "index_stats" => include_str!("fixtures/real_index_stats.json"),
        "own_traffic" => include_str!("fixtures/real_own_traffic.json"),
        "own_traffic_prefixed" => include_str!("fixtures/real_own_traffic_prefixed.json"),
        "system_profile_read" => include_str!("fixtures/real_system_profile_read.json"),
        "update" => include_str!("fixtures/real_update.json"),
        other => panic!("unknown real fixture {other}"),
    };
    editor_json_to_doc(text).expect("real fixture parses")
}

const ALL: [&str; 18] = [
    "aggregate",
    "app_find",
    "count",
    "delete",
    "distinct",
    "explain",
    "find_and_modify",
    "find_collscan",
    "find_inefficient",
    "find_ixscan",
    "find_no_filter",
    "find_sort",
    "getmore",
    "index_stats",
    "own_traffic",
    "own_traffic_prefixed",
    "system_profile_read",
    "update",
];

fn entry(name: &str) -> ProfileEntry {
    parse_entry(&raw(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

fn verdict(name: &str) -> VerdictKind {
    let e = entry(name);
    classify_entry(&e, entry_has_filter(&e))
}

#[test]
fn every_real_fixture_parses_and_is_scrubbed() {
    for name in ALL {
        let e = entry(name);
        assert!(
            e.ts_millis > 0 && e.ns.starts_with("test."),
            "{name}: {e:?}"
        );
        let text = format!("{:?}", raw(name));
        assert!(!text.contains("cbit_"), "{name} names a throwaway database");
        assert_eq!(raw(name).get_str("client"), Ok("127.0.0.1"), "{name}");
    }
}

#[test]
fn real_ops_are_the_ones_the_shape_code_expects() {
    for (name, op, verb) in [
        ("find_collscan", "query", "find"),
        ("count", "command", "count"),
        ("distinct", "command", "distinct"),
        ("aggregate", "command", "aggregate"),
        ("find_and_modify", "command", "findAndModify"),
        // Update and delete name the op; `command` is the statement itself.
        ("update", "update", "q"),
        ("delete", "remove", "q"),
        ("getmore", "getmore", "getMore"),
    ] {
        let e = entry(name);
        assert_eq!(e.op, op, "{name}");
        assert_eq!(
            e.command.keys().next().map(String::as_str),
            Some(verb),
            "{name}"
        );
    }
}

#[test]
fn real_find_entries_classify_as_the_explain_classifier_would() {
    assert_eq!(verdict("find_collscan"), VerdictKind::CollectionScan);
    assert_eq!(verdict("find_ixscan"), VerdictKind::IndexUsed);
    assert_eq!(verdict("find_sort"), VerdictKind::SortNotCovered);
    assert_eq!(verdict("find_inefficient"), VerdictKind::InefficientIndex);
    assert_eq!(verdict("find_no_filter"), VerdictKind::NoFilter);
    let sort = entry("find_sort");
    assert!(sort.has_sort_stage && sort.plan_summary == "IXSCAN { a: 1 }");
    let ix = entry("find_ixscan");
    assert_eq!((ix.keys_examined, ix.docs_examined, ix.returned), (1, 1, 1));
}

#[test]
fn real_non_find_entries_classify() {
    assert_eq!(verdict("count"), VerdictKind::IndexUsed);
    // Pinned as observed, and arguably a false positive: a distinct reads every key of its
    // range to return few values (here 300 keys for 10 values), which the examined/returned
    // ratio reads as an inefficient index although no index could do better.
    assert_eq!(verdict("distinct"), VerdictKind::InefficientIndex);
    assert_eq!(verdict("aggregate"), VerdictKind::IndexUsed);
    assert_eq!(verdict("update"), VerdictKind::IndexUsed);
    assert_eq!(verdict("delete"), VerdictKind::CollectionScan);
    assert_eq!(verdict("find_and_modify"), VerdictKind::CollectionScan);
    assert_eq!(entry("count").plan_summary, "COUNT_SCAN { a: 1 }");
}

#[test]
fn every_real_operation_with_a_filter_has_a_shape() {
    for name in [
        "find_collscan",
        "find_ixscan",
        "find_sort",
        "find_inefficient",
        "count",
        "distinct",
        "aggregate",
        "update",
        "delete",
        "find_and_modify",
    ] {
        let e = entry(name);
        let shape = from_profile_command(&e.op, &e.command);
        assert!(shape.is_some_and(|s| s.has_filter()), "{name}");
    }
    let e = entry("getmore");
    assert!(from_profile_command(&e.op, &e.command).is_none());
}

#[test]
fn real_shapes_hold_no_query_value() {
    // The captured queries filter on b: 5, c: 5, a: 3/4 and a range 0..6000.
    for name in ["find_collscan", "find_ixscan", "find_inefficient", "update"] {
        let e = entry(name);
        let json = from_profile_command(&e.op, &e.command).unwrap().json();
        assert!(!json.contains("6000"), "{name}: {json}");
        assert!(json.contains('"'), "{name}: {json}");
    }
    let (a, b) = (entry("find_collscan"), entry("find_ixscan"));
    let hash = |e: &ProfileEntry| from_profile_command(&e.op, &e.command).unwrap().hash();
    assert_eq!(
        hash(&a),
        hash(&b),
        "the same filter on two collections is one shape"
    );
}

#[test]
fn real_own_traffic_is_recognised() {
    for name in [
        "own_traffic",
        "own_traffic_prefixed",
        "explain",
        "index_stats",
        "system_profile_read",
    ] {
        assert!(is_own_traffic(&entry(name)), "{name}");
    }
    assert_eq!(
        entry("own_traffic_prefixed").app_name.as_deref(),
        Some("shopapp curlybrace")
    );
    for name in ["app_find", "find_collscan", "update", "getmore"] {
        assert!(!is_own_traffic(&entry(name)), "{name}");
    }
}

#[test]
fn a_real_getmore_is_a_continuation_judged_neutral() {
    let more = entry("getmore");
    assert!(more.is_continuation());
    assert_eq!(more.op, "getmore");
    assert_eq!(classify_entry(&more, false), VerdictKind::NoFilter);
    assert!(!entry("find_collscan").is_continuation());
}
