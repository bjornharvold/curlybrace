//! The in-memory aggregate preview: the scripted answer, the record, and the refusal that
//! happens before anything is recorded.

use std::time::Duration;

use bson::doc;

use super::memory_store::InMemoryStore;
use crate::error::CoreError;
use crate::explain::ExplainVerbosity;
use crate::pipeline::{AppliedOptions, Operation, PipelineOptions};
use crate::store::{DocumentStore, Namespace};

const LIMIT: Duration = Duration::from_secs(1);

fn limit() -> AppliedOptions {
    AppliedOptions::bounded(Operation::Preview, LIMIT)
}

fn ns() -> Namespace {
    Namespace::new("db", "c")
}

#[tokio::test]
async fn an_unscripted_store_answers_nothing_and_records_the_pipeline() {
    let store = InMemoryStore::new();
    let pipeline = vec![doc! { "$match": { "a": 1 } }];
    let out = store
        .aggregate_preview(&ns(), pipeline.clone(), &limit())
        .await
        .unwrap();
    assert!(out.is_empty());
    assert_eq!(store.received_pipelines(), vec![pipeline]);
}

#[tokio::test]
async fn the_scripted_answer_is_returned() {
    let store = InMemoryStore::new().with_aggregate(vec![doc! { "n": 3 }]);
    let out = store
        .aggregate_preview(&ns(), vec![doc! { "$count": "n" }], &limit())
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
            .aggregate_preview(&ns(), pipeline, &limit())
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
        .aggregate_write(&ns(), pipeline.clone(), &limit())
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
            .aggregate_write(&ns(), pipeline, &limit())
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
        .aggregate_write(&ns(), vec![doc! { "$out": "t" }], &limit())
        .await
        .unwrap_err();
    assert_eq!(err, CoreError::Database("boom".into()));
}

#[tokio::test]
async fn an_explain_is_recorded_with_its_verbosity_and_answers_the_scripted_document() {
    let store = InMemoryStore::new().with_aggregate_explain(doc! { "ok": 1, "stages": [] });
    let pipeline = vec![doc! { "$match": { "a": 1 } }];
    let planned = store
        .aggregate_explain(
            &ns(),
            pipeline.clone(),
            ExplainVerbosity::QueryPlanner,
            &limit(),
        )
        .await
        .unwrap();
    assert_eq!(planned.document, doc! { "ok": 1, "stages": [] });
    assert!(planned.plan_only);
    let executed = store
        .aggregate_explain(
            &ns(),
            pipeline.clone(),
            ExplainVerbosity::ExecutionStats,
            &limit(),
        )
        .await
        .unwrap();
    assert!(!executed.plan_only);
    assert_eq!(
        store.explained_pipelines(),
        vec![
            (pipeline.clone(), ExplainVerbosity::QueryPlanner),
            (pipeline, ExplainVerbosity::ExecutionStats)
        ]
    );
    assert!(store.received_pipelines().is_empty());
}

#[tokio::test]
async fn an_unscripted_explain_is_unsupported() {
    let err = InMemoryStore::new()
        .aggregate_explain(
            &ns(),
            vec![doc! { "$match": {} }],
            ExplainVerbosity::QueryPlanner,
            &limit(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Unsupported(_)));
}

#[tokio::test]
async fn an_explain_of_a_write_stage_is_refused_before_anything_is_recorded() {
    let store = InMemoryStore::new().with_aggregate_explain(doc! { "ok": 1 });
    let facet = doc! { "$facet": { "f": [ { "$out": "t" } ] } };
    let lookup = doc! { "$lookup": { "from": "x", "pipeline": [ { "$merge": "t" } ], "as": "y" } };
    for pipeline in [vec![doc! { "$out": "t" }], vec![facet], vec![lookup]] {
        let err = store
            .aggregate_explain(&ns(), pipeline, ExplainVerbosity::ExecutionStats, &limit())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidDocument(_)));
    }
    assert!(store.explained_pipelines().is_empty());
}

#[tokio::test]
async fn a_bare_target_from_a_server_database_is_refused_and_not_recorded() {
    let store = InMemoryStore::new();
    for db in ["admin", "local", "config"] {
        let err = store
            .aggregate_write(
                &Namespace::new(db, "c"),
                vec![doc! { "$out": "t" }],
                &limit(),
            )
            .await
            .unwrap_err();
        assert_eq!(err, CoreError::ReservedDatabase, "{db}");
    }
    assert!(store.written_pipelines().is_empty());
}

#[tokio::test]
async fn the_store_records_the_options_each_call_received() {
    let store = InMemoryStore::new()
        .with_aggregate(vec![doc! { "n": 1 }])
        .with_aggregate_explain(doc! { "ok": 1 });
    let options = PipelineOptions {
        comment: Some("why".into()),
        max_time_ms: Some(2_000),
        allow_disk_use: Some(true),
        ..PipelineOptions::default()
    };
    let preview = options.applied(Operation::Preview);
    let explain = options.applied(Operation::Explain);
    let run = options.applied(Operation::Run);
    store
        .aggregate_preview(&ns(), vec![doc! { "$limit": 1 }], &preview)
        .await
        .unwrap();
    store
        .aggregate_explain(
            &ns(),
            vec![doc! { "$match": {} }],
            ExplainVerbosity::QueryPlanner,
            &explain,
        )
        .await
        .unwrap();
    store
        .aggregate_write(&ns(), vec![doc! { "$out": "t" }], &run)
        .await
        .unwrap();
    assert_eq!(
        store.received_options(),
        vec![
            (Operation::Preview, preview),
            (Operation::Explain, explain),
            (Operation::Run, run),
        ]
    );
}

#[tokio::test]
async fn a_refused_pipeline_records_no_options() {
    let store = InMemoryStore::new().with_aggregate_explain(doc! { "ok": 1 });
    let options = PipelineOptions::default();
    let preview = options.applied(Operation::Preview);
    let explain = options.applied(Operation::Explain);
    let run = options.applied(Operation::Run);
    let refused_preview = store
        .aggregate_preview(&ns(), vec![doc! { "$out": "t" }], &preview)
        .await;
    assert!(refused_preview.is_err());
    let refused_explain = store
        .aggregate_explain(
            &ns(),
            vec![doc! { "$merge": "t" }],
            ExplainVerbosity::QueryPlanner,
            &explain,
        )
        .await;
    assert!(refused_explain.is_err());
    let refused_run = store
        .aggregate_write(&ns(), vec![doc! { "$match": {} }], &run)
        .await;
    assert!(refused_run.is_err());
    assert!(store.received_options().is_empty());
}
