//! The in-memory aggregate preview: the scripted answer, the record, and the refusal that
//! happens before anything is recorded.

use std::time::Duration;

use bson::doc;

use super::memory_store::InMemoryStore;
use crate::error::CoreError;
use crate::store::{DocumentStore, Namespace};

const LIMIT: Duration = Duration::from_secs(1);

fn ns() -> Namespace {
    Namespace::new("db", "c")
}

#[tokio::test]
async fn an_unscripted_store_answers_nothing_and_records_the_pipeline() {
    let store = InMemoryStore::new();
    let pipeline = vec![doc! { "$match": { "a": 1 } }];
    let out = store
        .aggregate_preview(&ns(), pipeline.clone(), LIMIT)
        .await
        .unwrap();
    assert!(out.is_empty());
    assert_eq!(store.received_pipelines(), vec![pipeline]);
}

#[tokio::test]
async fn the_scripted_answer_is_returned() {
    let store = InMemoryStore::new().with_aggregate(vec![doc! { "n": 3 }]);
    let out = store
        .aggregate_preview(&ns(), vec![doc! { "$count": "n" }], LIMIT)
        .await
        .unwrap();
    assert_eq!(out, vec![doc! { "n": 3 }]);
}

#[tokio::test]
async fn a_write_stage_is_refused_before_anything_is_recorded() {
    let store = InMemoryStore::new().with_aggregate(vec![doc! { "n": 3 }]);
    let nested = doc! { "$lookup": { "from": "x", "pipeline": [ { "$merge": "t" } ], "as": "y" } };
    for pipeline in [vec![doc! { "$out": "t" }], vec![nested]] {
        let err = store
            .aggregate_preview(&ns(), pipeline, LIMIT)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidDocument(_)));
    }
    assert!(store.received_pipelines().is_empty());
}
