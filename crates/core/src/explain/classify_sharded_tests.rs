use bson::{Document, doc};

use super::{VerdictKind, classify};
use crate::error::CoreError;
use crate::store::Query;

fn filtered() -> Query {
    Query {
        filter: doc! { "status": "open" },
        ..Query::default()
    }
}

fn stats() -> Document {
    doc! { "nReturned": 12, "totalKeysExamined": 0, "totalDocsExamined": 5000 }
}

/// What mongos reports: the merge stage on top, the per-shard plans under `shards`.
fn sharded(stage: &str, shard_plans: Vec<Document>) -> Document {
    let shards: Vec<Document> = shard_plans
        .into_iter()
        .enumerate()
        .map(|(i, plan)| doc! { "shardName": format!("shard{i}"), "winningPlan": plan })
        .collect();
    doc! {
        "queryPlanner": {
            "namespace": "shop.orders",
            "winningPlan": { "stage": stage, "shards": shards },
        },
        "executionStats": stats(),
    }
}

fn collscan() -> Document {
    doc! { "stage": "COLLSCAN", "direction": "forward" }
}

fn fetch_ixscan() -> Document {
    doc! {
        "stage": "FETCH",
        "inputStage": { "stage": "IXSCAN", "indexName": "status_1" },
    }
}

#[test]
fn sharded_collscan_under_shards_is_a_collection_scan() {
    let d = sharded("SHARD_MERGE", vec![collscan(), collscan()]);
    let v = classify(&d, &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::CollectionScan);
    assert!(v.suggestion.is_some());
}

#[test]
fn sharded_single_shard_ixscan_is_index_used() {
    let mut d = sharded("SINGLE_SHARD", vec![fetch_ixscan()]);
    d.insert(
        "executionStats",
        doc! { "nReturned": 12, "totalKeysExamined": 12, "totalDocsExamined": 12 },
    );
    let v = classify(&d, &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("status_1"));
}

#[test]
fn sharded_plan_wrapped_in_query_plan_is_read() {
    let wrapped = doc! { "stage": "SBE", "queryPlan": collscan() };
    let d = sharded("SINGLE_SHARD", vec![wrapped]);
    let v = classify(&d, &filtered()).unwrap();
    assert_eq!(v.kind, VerdictKind::CollectionScan);
}

#[test]
fn a_plan_with_no_access_stage_is_an_error_not_a_good_verdict() {
    let d = doc! {
        "queryPlanner": { "winningPlan": { "stage": "SHARD_MERGE", "shards": [] } },
        "executionStats": stats(),
    };
    let err = classify(&d, &filtered()).unwrap_err();
    assert!(matches!(err, CoreError::Database(_)), "{err:?}");
}

#[test]
fn sort_over_a_collection_scan_reports_both_reasons() {
    let plan = doc! { "stage": "SORT", "sortPattern": { "c": -1 }, "inputStage": collscan() };
    let d = doc! {
        "queryPlanner": { "winningPlan": plan },
        "executionStats": stats(),
    };
    let q = Query {
        sort: Some(doc! { "c": -1 }),
        ..filtered()
    };
    let v = classify(&d, &q).unwrap();
    assert_eq!(v.kind, VerdictKind::SortNotCovered);
    let keys: Vec<_> = v.reasons.iter().map(|r| r.key).collect();
    assert_eq!(keys, vec!["sort_blocking", "collscan"]);
}

#[test]
fn sort_over_an_index_reports_only_the_sort() {
    let plan = doc! { "stage": "SORT", "inputStage": fetch_ixscan() };
    let d = doc! {
        "queryPlanner": { "winningPlan": plan },
        "executionStats": stats(),
    };
    let q = Query {
        sort: Some(doc! { "c": -1 }),
        ..filtered()
    };
    let v = classify(&d, &q).unwrap();
    assert_eq!(v.reasons.len(), 1);
}
