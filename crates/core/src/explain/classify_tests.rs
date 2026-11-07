use bson::{Document, doc};

use super::{VerdictKind, classify};
use crate::ejson::editor_json_to_doc;
use crate::store::Query;

fn fixture(name: &str) -> Document {
    let text = match name {
        "collscan" => include_str!("fixtures/collscan.json"),
        "ixscan" => include_str!("fixtures/ixscan.json"),
        "sort" => include_str!("fixtures/sort.json"),
        "inefficient" => include_str!("fixtures/inefficient.json"),
        "covered" => include_str!("fixtures/covered.json"),
        "or_multi_ixscan" => include_str!("fixtures/or_multi_ixscan.json"),
        "sbe" => include_str!("fixtures/sbe.json"),
        "eof" => include_str!("fixtures/eof.json"),
        "plan_only" => include_str!("fixtures/plan_only.json"),
        "id_only" => include_str!("fixtures/id_only.json"),
        "real_collscan" => include_str!("fixtures/real_collscan.json"),
        "real_ixscan" => include_str!("fixtures/real_ixscan.json"),
        "real_sort" => include_str!("fixtures/real_sort.json"),
        "real_inefficient" => include_str!("fixtures/real_inefficient.json"),
        "real_covered" => include_str!("fixtures/real_covered.json"),
        "real_view" => include_str!("fixtures/real_view.json"),
        "real_empty" => include_str!("fixtures/real_empty.json"),
        "real_missing" => include_str!("fixtures/real_missing.json"),
        "real_plan_only" => include_str!("fixtures/real_plan_only.json"),
        other => panic!("unknown fixture {other}"),
    };
    editor_json_to_doc(text).expect("fixture parses")
}

fn filtered() -> Query {
    Query {
        filter: doc! { "status": "open" },
        ..Query::default()
    }
}

#[test]
fn classify_collscan_is_collection_scan() {
    let v = classify(&fixture("collscan"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::CollectionScan);
    assert_eq!(v.docs_examined, 5000);
    assert_eq!(v.returned, 12);
    assert!(!v.plan_only);
    assert_eq!(v.reasons[0].key, "collscan");
    assert!(v.suggestion.is_some());
}

#[test]
fn classify_ixscan_close_to_returned_is_index_used() {
    let v = classify(&fixture("ixscan"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("status_1"));
    assert!(v.reasons.is_empty());
    assert!(v.suggestion.is_none());
}

#[test]
fn classify_sort_stage_is_sort_not_covered() {
    let q = Query {
        sort: Some(doc! { "createdAt": -1 }),
        ..filtered()
    };
    let v = classify(&fixture("sort"), &q).unwrap();
    assert_eq!(v.kind, VerdictKind::SortNotCovered);
    assert_eq!(v.reasons[0].key, "sort_blocking");
    assert_eq!(
        v.suggestion.unwrap().keys,
        vec![("status".to_string(), 1), ("createdAt".to_string(), -1)]
    );
}

#[test]
fn classify_many_keys_few_returned_is_inefficient() {
    let v = classify(&fixture("inefficient"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::InefficientIndex);
    assert_eq!(v.index_name.as_deref(), Some("status_1"));
    assert_eq!(v.reasons[0].key, "inefficient");
    assert_eq!(
        v.reasons[0].params,
        vec![
            ("examined", "8000".to_string()),
            ("returned", "3".to_string())
        ]
    );
}

#[test]
fn classify_projection_covered_is_covered() {
    let v = classify(&fixture("covered"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::CoveredByIndex);
    assert_eq!(v.docs_examined, 0);
}

#[test]
fn classify_or_with_two_ixscans_is_index_used() {
    let v = classify(&fixture("or_multi_ixscan"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("status_1"));
}

#[test]
fn classify_sbe_query_plan_wrapper_is_read() {
    let v = classify(&fixture("sbe"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("status_1"));
}

#[test]
fn classify_eof_is_empty_collection() {
    let v = classify(&fixture("eof"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::EmptyCollection);
    assert_eq!(v.reasons[0].key, "empty_collection");
}

#[test]
fn classify_empty_filter_without_sort_is_no_filter() {
    let v = classify(&fixture("collscan"), &Query::default()).unwrap();
    assert_eq!(v.kind, VerdictKind::NoFilter);
    assert_eq!(v.reasons[0].key, "no_filter");
    assert!(v.suggestion.is_none());
}

#[test]
fn classify_empty_filter_with_uncovered_sort_is_sort_not_covered() {
    let q = Query {
        sort: Some(doc! { "createdAt": -1 }),
        ..Query::default()
    };
    let v = classify(&fixture("sort"), &q).unwrap();
    assert_eq!(v.kind, VerdictKind::SortNotCovered);
}

#[test]
fn classify_id_only_filter_is_index_used() {
    let q = Query {
        filter: doc! { "_id": 7 },
        ..Query::default()
    };
    let v = classify(&fixture("id_only"), &q).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("_id_"));
}

#[test]
fn classify_query_planner_only_sets_plan_only() {
    let v = classify(&fixture("plan_only"), &filtered()).unwrap();
    assert!(v.plan_only);
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!((v.keys_examined, v.docs_examined, v.returned), (0, 0, 0));
}

#[test]
fn classify_garbage_document_errors() {
    let err = classify(&doc! { "hello": "world" }, &filtered()).unwrap_err();
    assert!(matches!(err, crate::error::CoreError::Database(_)));
}

// Documents captured from a real server (mongod 9.0.4); see tests/local_explain.rs.

#[test]
fn real_empty_existing_collection_is_empty_not_a_collection_scan() {
    // The server plans a COLLSCAN (not EOF) for a collection that exists but holds nothing.
    let v = classify(&fixture("real_empty"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::EmptyCollection);
    assert_eq!(v.reasons[0].key, "empty_collection");
    assert!(v.suggestion.is_none());
}

#[test]
fn real_empty_existing_collection_with_no_filter_is_empty() {
    let v = classify(&fixture("real_empty"), &Query::default()).unwrap();
    assert_eq!(v.kind, VerdictKind::EmptyCollection);
}

#[test]
fn real_nonexistent_collection_is_empty() {
    let v = classify(&fixture("real_missing"), &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::EmptyCollection);
}

#[test]
fn real_documents_classify_like_the_hand_written_ones() {
    let q = |sort: Option<bson::Document>, filter: bson::Document| Query {
        filter,
        sort,
        ..Query::default()
    };
    let kind = |name: &str, q: &Query| classify(&fixture(name), q).unwrap().kind;
    assert_eq!(
        kind("real_collscan", &q(None, doc! { "c": 5 })),
        VerdictKind::CollectionScan
    );
    assert_eq!(
        kind("real_ixscan", &q(None, doc! { "c": 5 })),
        VerdictKind::IndexUsed
    );
    assert_eq!(
        kind("real_sort", &q(Some(doc! { "c": -1 }), doc! { "a": 3 })),
        VerdictKind::SortNotCovered
    );
    assert_eq!(
        kind("real_inefficient", &q(None, doc! { "a": 3 })),
        VerdictKind::InefficientIndex
    );
    assert_eq!(
        kind("real_covered", &q(None, doc! { "c": 5 })),
        VerdictKind::CoveredByIndex
    );
    assert_eq!(
        kind("real_view", &q(None, doc! { "c": 5 })),
        VerdictKind::CollectionScan
    );
    assert_eq!(
        kind("real_plan_only", &q(None, doc! { "c": 5 })),
        VerdictKind::IndexUsed
    );
}
