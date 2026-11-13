//! Tier 1 (local server): aggregate previews against a real MongoDB. Writes only to throwaway
//! `cbit_*` databases. Run with `scripts/integration-tests.sh`.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bson::{Bson, Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::pipeline::{
    PREVIEW_LIMIT, PREVIEW_MAX_TIME_MS, PipelineError, Stage, preview_pipeline,
};
use curlybrace_core::read_only_store::ReadOnlyStore;
use curlybrace_core::store::{DocumentStore, Namespace};
use mongodb::Client;
use support::{insert_labelled, local_enabled, local_mongo};

const MAX_TIME: Duration = Duration::from_millis(PREVIEW_MAX_TIME_MS);

fn stages(texts: &[&str]) -> Vec<Stage> {
    texts
        .iter()
        .map(|t| Stage {
            text: (*t).to_string(),
            enabled: true,
        })
        .collect()
}

/// 30 orders (more than the preview limit): `k` cycles 0..3, `amt` is the index, two items each.
fn orders() -> Vec<Document> {
    (0..30)
        .map(|i| doc! { "_id": i, "k": i % 3, "amt": i, "items": [i, i + 100] })
        .collect()
}

async fn preview(store: &dyn DocumentStore, ns: &Namespace, texts: &[&str]) -> Vec<Document> {
    let plan = preview_pipeline(&stages(texts), texts.len() - 1, PREVIEW_LIMIT).unwrap();
    store
        .aggregate_preview(ns, plan.pipeline, MAX_TIME)
        .await
        .unwrap()
}

async fn collections(client: &Client, db: &str) -> Vec<String> {
    client.database(db).list_collection_names().await.unwrap()
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn group_lookup_unwind_and_facet_previews_return_the_expected_documents() {
    let name = "group_lookup_unwind_and_facet_previews_return_the_expected_documents";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let people = dbs.ns("people");
    insert_labelled(
        &client,
        &people,
        &[
            doc! { "_id": 0, "name": "zero" },
            doc! { "_id": 1, "name": "one" },
            doc! { "_id": 2, "name": "two" },
        ],
    )
    .await;

    // $group: per k, 10 orders and the sum of amt (k=0: 0+3+..+27 = 135).
    let grouped = preview(
        &store,
        &ns,
        &[
            r#"{"$group": {"_id": "$k", "n": {"$sum": 1}, "total": {"$sum": "$amt"}}}"#,
            r#"{"$sort": {"_id": 1}}"#,
        ],
    )
    .await;
    assert_eq!(
        grouped,
        vec![
            doc! { "_id": 0, "n": 10, "total": 135 },
            doc! { "_id": 1, "n": 10, "total": 145 },
            doc! { "_id": 2, "n": 10, "total": 155 },
        ]
    );

    // $lookup into another collection.
    let looked = preview(
        &store,
        &ns,
        &[
            r#"{"$match": {"_id": {"$lt": 2}}}"#,
            r#"{"$lookup": {"from": "people", "localField": "k", "foreignField": "_id", "as": "who"}}"#,
            r#"{"$project": {"_id": 1, "who": "$who.name"}}"#,
            r#"{"$sort": {"_id": 1}}"#,
        ],
    )
    .await;
    assert_eq!(
        looked,
        vec![
            doc! { "_id": 0, "who": ["zero"] },
            doc! { "_id": 1, "who": ["one"] },
        ]
    );

    // $unwind: 60 rows exist, the preview is capped at the limit.
    let unwound = preview(&store, &ns, &[r#"{"$unwind": "$items"}"#]).await;
    assert_eq!(unwound.len() as i64, PREVIEW_LIMIT);
    assert_eq!(unwound[0].get_i32("items"), Ok(0));
    assert_eq!(unwound[1].get_i32("items"), Ok(100));

    // $facet: two branches in one document.
    let faceted = preview(
        &store,
        &ns,
        &[
            r#"{"$facet": {"count": [{"$count": "n"}], "evens": [{"$match": {"amt": {"$lt": 4}}}, {"$group": {"_id": null, "s": {"$sum": "$amt"}}}]}}"#,
        ],
    )
    .await;
    assert_eq!(
        faceted,
        vec![doc! {
            "count": [ { "n": 30 } ],
            "evens": [ { "_id": Bson::Null, "s": 6 } ],
        }]
    );

    // A plain scan of 30 documents is capped at the limit.
    let capped = preview(&store, &ns, &[r#"{"$match": {}}"#]).await;
    assert_eq!(capped.len() as i64, PREVIEW_LIMIT);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_trailing_out_or_merge_is_not_run() {
    if !local_enabled("a_trailing_out_or_merge_is_not_run") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;

    for write in [
        r#"{"$out": "copy_out"}"#,
        r#"{"$merge": {"into": "copy_merge"}}"#,
    ] {
        let texts = ["{\"$match\": {\"k\": 1}}", write];
        let plan = preview_pipeline(&stages(&texts), 1, PREVIEW_LIMIT).unwrap();
        assert_eq!(plan.not_run.len(), 1, "{write}");
        let docs = store
            .aggregate_preview(&ns, plan.pipeline, MAX_TIME)
            .await
            .unwrap();
        assert_eq!(docs.len(), 10, "{write}");
    }
    assert_eq!(
        collections(&client, &dbs.db).await,
        vec!["orders".to_string()]
    );

    // An existing target is left unchanged by a trailing $out.
    let target = dbs.ns("existing");
    insert_labelled(&client, &target, &[doc! { "_id": "keep" }]).await;
    let texts = ["{\"$match\": {}}", r#"{"$out": "existing"}"#];
    let plan = preview_pipeline(&stages(&texts), 1, PREVIEW_LIMIT).unwrap();
    store
        .aggregate_preview(&ns, plan.pipeline, MAX_TIME)
        .await
        .unwrap();
    let kept: Vec<Document> = store
        .aggregate_preview(&target, vec![doc! { "$match": {} }], MAX_TIME)
        .await
        .unwrap();
    assert_eq!(kept, vec![doc! { "_id": "keep" }]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_nested_write_stage_is_refused_before_the_server() {
    if !local_enabled("a_nested_write_stage_is_refused_before_the_server") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;

    let nested = [
        r#"{"$facet": {"a": [{"$match": {}}, {"$out": "nested_out"}]}}"#,
        r#"{"$lookup": {"from": "orders", "pipeline": [{"$merge": {"into": "nested_merge"}}], "as": "x"}}"#,
    ];
    for text in nested {
        let err = preview_pipeline(&stages(&[text]), 0, PREVIEW_LIMIT).unwrap_err();
        assert!(
            matches!(err, PipelineError::NestedWriteStage { .. }),
            "{err:?}"
        );

        // Forced straight into the store, the store still refuses before any server call.
        let pipeline = vec![serde_json::from_str::<Document>(text).unwrap()];
        let err = store
            .aggregate_preview(&ns, pipeline, MAX_TIME)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidDocument(_)), "{err:?}");
    }
    assert_eq!(
        collections(&client, &dbs.db).await,
        vec!["orders".to_string()]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn max_time_expiry_is_the_timeout_error() {
    if !local_enabled("max_time_expiry_is_the_timeout_error") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("big");
    let docs: Vec<Document> = (0..3_000).map(|i| doc! { "_id": i, "v": i }).collect();
    insert_labelled(&client, &ns, &docs).await;
    // An unindexed self cross join: ~9 million expression evaluations, far beyond the 50 ms bound.
    let pipeline = vec![
        doc! { "$lookup": {
            "from": "big",
            "let": { "v": "$v" },
            "pipeline": [ { "$match": { "$expr": { "$gte": ["$v", "$$v"] } } } ],
            "as": "cross",
        } },
        doc! { "$group": { "_id": null, "n": { "$sum": { "$size": "$cross" } } } },
    ];
    let err = store
        .aggregate_preview(&ns, pipeline, Duration::from_millis(50))
        .await
        .unwrap_err();
    assert_eq!(err, CoreError::Database("timeout".into()));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_read_only_store_still_previews() {
    if !local_enabled("a_read_only_store_still_previews") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let locked = Arc::new(AtomicBool::new(true));
    let read_only = ReadOnlyStore::wrap(Arc::new(store as MongoStore), locked);
    assert!(read_only.is_read_only());
    let docs = preview(read_only.as_ref(), &ns, &[r#"{"$match": {"k": 2}}"#]).await;
    assert_eq!(docs.len(), 10);
}
