//! The fixtures captured from a real server (mongod 9.0.4) by `tests/local_profiler.rs` with
//! `CURLYBRACE_CAPTURE=1`: the profile code must read, judge and shape them as it does the
//! hand-written fixtures. Refresh them with that mode; never edit them by hand.

use bson::{Bson, Document};

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
        "update_many" => include_str!("fixtures/real_update_many.json"),
        "delete_many" => include_str!("fixtures/real_delete_many.json"),
        other => panic!("unknown real fixture {other}"),
    };
    editor_json_to_doc(text).expect("real fixture parses")
}

const ALL: [&str; 20] = [
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
    "update_many",
    "delete_many",
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
        assert_scrubbed(name, &Bson::Document(raw(name)));
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
    // A distinct reads every key of its range to return few values: no index could do better,
    // so it is not judged by the examined/returned ratio (only documents-matching ops are).
    assert_eq!(verdict("distinct"), VerdictKind::IndexUsed);
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

#[test]
fn a_ratio_that_would_be_inefficient_is_not_for_distinct_count_or_aggregate() {
    for name in ["distinct", "count", "aggregate"] {
        let mut e = entry(name);
        (e.keys_examined, e.docs_examined, e.returned) = (6000, 6000, 1);
        assert_eq!(classify_entry(&e, true), VerdictKind::IndexUsed, "{name}");
    }
    let mut find = entry("find_ixscan");
    (find.keys_examined, find.docs_examined, find.returned) = (6000, 6000, 1);
    assert_eq!(classify_entry(&find, true), VerdictKind::InefficientIndex);
}

#[test]
fn an_aggregate_that_scans_the_collection_is_still_a_collection_scan() {
    let text = include_str!("fixtures/aggregate_match.json");
    let e = parse_entry(&editor_json_to_doc(text).unwrap()).unwrap();
    assert_eq!(
        classify_entry(&e, entry_has_filter(&e)),
        VerdictKind::CollectionScan
    );
}

/// A real write entry with its counters replaced: `keys` examined through the index and
/// `done` documents matched (update) or deleted (remove).
fn write_entry(name: &str, keys: i32, done: i32) -> ProfileEntry {
    let mut d = raw(name);
    d.insert("keysExamined", keys);
    d.insert("docsExamined", keys);
    d.remove("nreturned");
    d.insert("planSummary", "IXSCAN { a: 1 }");
    for key in ["nMatched", "ndeleted"] {
        if d.contains_key(key) {
            d.insert(key, done);
        }
    }
    parse_entry(&d).unwrap()
}

#[test]
fn a_write_counts_its_matched_or_deleted_documents_as_returned() {
    assert_eq!(write_entry("update", 6000, 6000).returned, 6000);
    assert_eq!(write_entry("delete", 6000, 6000).returned, 6000);
    assert_eq!(
        entry("find_ixscan").returned,
        1,
        "nreturned still wins for reads"
    );
}

#[test]
fn a_multi_document_write_through_an_index_is_not_inefficient() {
    for name in ["update", "delete"] {
        let e = write_entry(name, 6000, 6000);
        assert_eq!(classify_entry(&e, true), VerdictKind::IndexUsed, "{name}");
    }
    // The same keys for a handful of matches is wasteful.
    let e = write_entry("update", 6000, 3);
    assert_eq!(classify_entry(&e, true), VerdictKind::InefficientIndex);
    let mut find = entry("find_ixscan");
    (find.keys_examined, find.docs_examined, find.returned) = (6000, 6000, 3);
    assert_eq!(classify_entry(&find, true), VerdictKind::InefficientIndex);
}

/// No session, host or user field and no throwaway database name at any depth.
fn assert_scrubbed(name: &str, value: &Bson) {
    match value {
        Bson::Document(d) => {
            for (key, v) in d {
                assert!(
                    !["lsid", "$clusterTime", "txnNumber"].contains(&key.as_str()),
                    "{name}: {key} was not scrubbed"
                );
                match (key.as_str(), v) {
                    ("me" | "host", _) => panic!("{name}: {key} was not scrubbed"),
                    ("user", Bson::String(u)) => assert!(u.is_empty(), "{name}: user {u}"),
                    _ => assert_scrubbed(name, v),
                }
            }
        }
        Bson::Array(items) => items.iter().for_each(|v| assert_scrubbed(name, v)),
        Bson::String(s) => assert!(!s.contains("cbit_"), "{name}: {s}"),
        _ => {}
    }
}

#[test]
fn real_multi_document_writes_are_index_used_with_matched_and_deleted_as_returned() {
    for name in ["update_many", "delete_many"] {
        let e = entry(name);
        assert!(e.plan_summary.contains("IXSCAN"), "{name}: {e:?}");
        assert!(e.keys_examined >= 600 && e.returned == 600, "{name}: {e:?}");
        assert!(
            raw(name).get("nreturned").is_none(),
            "{name} has no nreturned"
        );
        assert_eq!(verdict(name), VerdictKind::IndexUsed, "{name}");
    }
    assert_eq!(entry("update_many").op, "update");
    assert_eq!(entry("delete_many").op, "remove");
}
