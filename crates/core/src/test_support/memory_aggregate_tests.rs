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

#[tokio::test]
async fn a_write_run_is_recorded_apart_from_previews() {
    let store = InMemoryStore::new();
    let pipeline = vec![doc! { "$match": { "a": 1 } }, doc! { "$out": "t" }];
    store
        .aggregate_write(&ns(), pipeline.clone(), LIMIT)
        .await
        .unwrap();
    assert_eq!(store.written_pipelines(), vec![pipeline]);
    assert!(store.received_pipelines().is_empty());
}

#[tokio::test]
async fn a_malformed_run_is_refused_and_not_recorded() {
    let store = InMemoryStore::new();
    for pipeline in [
        Vec::new(),
        vec![doc! { "$match": {} }],
        vec![doc! { "$out": "t" }, doc! { "$match": {} }],
    ] {
        let err = store
            .aggregate_write(&ns(), pipeline, LIMIT)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidDocument(_)));
    }
    assert!(store.written_pipelines().is_empty());
}

#[tokio::test]
async fn a_scripted_write_error_is_returned() {
    let store = InMemoryStore::new().with_aggregate_write_error(CoreError::Database("boom".into()));
    let err = store
        .aggregate_write(&ns(), vec![doc! { "$out": "t" }], LIMIT)
        .await
        .unwrap_err();
    assert_eq!(err, CoreError::Database("boom".into()));
}

#[tokio::test]
async fn a_bare_target_from_a_server_database_is_refused_and_not_recorded() {
    let store = InMemoryStore::new();
    for db in ["admin", "local", "config"] {
        let err = store
            .aggregate_write(&Namespace::new(db, "c"), vec![doc! { "$out": "t" }], LIMIT)
            .await
            .unwrap_err();
        assert_eq!(err, CoreError::ReservedDatabase, "{db}");
    }
    assert!(store.written_pipelines().is_empty());
}
