use bson::doc;

use super::*;
use crate::compare::diff::PathSegment::{Field, Index};
use crate::compare::fingerprint::fingerprint_of;
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::results::{Budget, Limits};
use crate::compare::rules::CompareRules;
use bson::Bson;

const SENTINEL: &str = "VALUE-SENTINEL-93";

fn spec() -> PairSpec {
    PairSpec {
        id: 1,
        source: Namespace::new("shop", "items"),
        target: Namespace::new("shop2", "items2"),
        source_filter: doc! {},
        target_filter: doc! {},
        key: KeySpec::id(),
        rules: CompareRules::for_key(&KeySpec::id()),
    }
}

fn entry_for(source: &Document, target: &Document) -> DiffEntry {
    let rules = spec().rules;
    DiffEntry {
        key: KeyTuple(vec![Bson::Int32(1)]),
        source: Some(fingerprint_of(source, &rules).unwrap()),
        target: Some(fingerprint_of(target, &rules).unwrap()),
    }
}

#[test]
fn display_names_are_database_dot_collection() {
    assert_eq!(display_name(&Namespace::new("a", "b.c")), "a.b.c");
}

#[test]
fn the_pair_summary_counts_come_from_the_stored_results() {
    let mut results = ResultSet::new(Budget::new(Limits::default()));
    for (n, category) in [
        (1, Category::OnlyInSource),
        (2, Category::OnlyInSource),
        (3, Category::Different),
    ] {
        let entry = DiffEntry {
            key: KeyTuple(vec![Bson::Int32(n)]),
            source: None,
            target: None,
        };
        results.push(category, entry).unwrap();
    }
    let reported = Summary {
        equal: 7,
        only_in_source: 99,
        only_in_target: 99,
        different: 99,
        scanned_source: 11,
        scanned_target: 12,
    };
    let s = pair_summary(&spec(), &results, reported);
    assert_eq!(
        (s.source.as_str(), s.target.as_str()),
        ("shop.items", "shop2.items2")
    );
    assert_eq!(
        s.summary,
        Summary {
            equal: 7,
            only_in_source: 2,
            only_in_target: 0,
            different: 1,
            scanned_source: 11,
            scanned_target: 12,
        }
    );
    assert_eq!(row_count(&results), 3);
}

#[test]
fn changed_paths_are_segments_and_never_values() {
    let source = doc! { "_id": 1, "a": { "b": SENTINEL }, "list": [1, 2], "same": 1 };
    let target = doc! { "_id": 1, "a": { "b": "other" }, "list": [1, 3], "same": 1 };
    let entry = entry_for(&source, &target);
    let found = changed_paths(&spec(), &entry, &source, &target).unwrap();
    let ChangedPaths::Paths(paths) = found else {
        panic!("{found:?}");
    };
    assert_eq!(
        paths,
        vec![
            vec![Field("a".into()), Field("b".into())],
            vec![Field("list".into()), Index(1)],
        ]
    );
}

#[test]
fn a_document_that_changed_since_the_compare_is_stale() {
    let source = doc! { "_id": 1, "a": 1 };
    let target = doc! { "_id": 1, "a": 2 };
    let entry = entry_for(&source, &target);
    let moved = doc! { "_id": 1, "a": 3 };
    assert_eq!(
        changed_paths(&spec(), &entry, &moved, &target).unwrap(),
        ChangedPaths::Stale
    );
    assert_eq!(
        changed_paths(&spec(), &entry, &source, &moved).unwrap(),
        ChangedPaths::Stale
    );
    let unrecorded = DiffEntry {
        source: None,
        ..entry
    };
    assert_eq!(
        changed_paths(&spec(), &unrecorded, &source, &target).unwrap(),
        ChangedPaths::Stale
    );
}
