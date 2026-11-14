//! Tier 1 (local server): a confirmed `$out` / `$merge` run against a real MongoDB. Writes only
//! to throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{AGGREGATE_RUN_MAX_TIME_MS, Stage, run_pipeline};
use curlybrace_core::read_only_store::ReadOnlyStore;
use curlybrace_core::store::{DocumentStore, Namespace};
use support::{insert_labelled, local_enabled, local_mongo};

const MAX_TIME: Duration = Duration::from_millis(AGGREGATE_RUN_MAX_TIME_MS);

fn stages(texts: &[&str]) -> Vec<Stage> {
    texts
        .iter()
        .map(|t| Stage {
            text: (*t).to_string(),
            enabled: true,
        })
        .collect()
}

async fn all(store: &dyn DocumentStore, ns: &Namespace) -> Vec<Document> {
    store
        .aggregate_preview(
            ns,
            vec![doc! { "$sort": { "_id": 1 } }, doc! { "$limit": 100 }],
            Duration::from_secs(10),
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn out_replaces_the_target_and_merge_merges_and_the_source_is_untouched() {
    let name = "out_replaces_the_target_and_merge_merges_and_the_source_is_untouched";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let source = dbs.ns("orders");
    let source_docs: Vec<Document> = (0..6).map(|i| doc! { "_id": i, "k": i % 2 }).collect();
    insert_labelled(&client, &source, &source_docs).await;

    // $out replaces the whole target: the old document is gone.
    let target = dbs.ns("copy_out");
    insert_labelled(&client, &target, &[doc! { "_id": "stale" }]).await;
    let plan = run_pipeline(&stages(&[
        r#"{"$match": {"k": 1}}"#,
        r#"{"$out": "copy_out"}"#,
    ]))
    .unwrap();
    store
        .aggregate_write(&source, plan.pipeline, MAX_TIME)
        .await
        .unwrap();
    assert_eq!(
        all(&store, &target).await,
        vec![
            doc! { "_id": 1, "k": 1 },
            doc! { "_id": 3, "k": 1 },
            doc! { "_id": 5, "k": 1 },
        ]
    );
    assert_eq!(store.estimated_count(&target).await.unwrap(), 3);

    // $merge into an existing collection keeps what it does not touch.
    let merged = dbs.ns("copy_merge");
    insert_labelled(
        &client,
        &merged,
        &[doc! { "_id": "keep" }, doc! { "_id": 0, "k": 9 }],
    )
    .await;
    let plan = run_pipeline(&stages(&[
        r#"{"$match": {"k": 0}}"#,
        r#"{"$merge": {"into": "copy_merge", "whenMatched": "replace"}}"#,
    ]))
    .unwrap();
    store
        .aggregate_write(&source, plan.pipeline, MAX_TIME)
        .await
        .unwrap();
    assert_eq!(
        all(&store, &merged).await,
        vec![
            doc! { "_id": 0, "k": 0 },
            doc! { "_id": 2, "k": 0 },
            doc! { "_id": 4, "k": 0 },
            doc! { "_id": "keep" },
        ]
    );
    assert_eq!(store.estimated_count(&merged).await.unwrap(), 4);

    // `{ db, coll }` writes into the other database.
    let elsewhere = Namespace::new(dbs.unicode_db.clone(), "copy_db");
    let text = format!(
        r#"{{"$out": {{"db": "{}", "coll": "copy_db"}}}}"#,
        dbs.unicode_db
    );
    let plan = run_pipeline(&stages(&[&text])).unwrap();
    store
        .aggregate_write(&source, plan.pipeline, MAX_TIME)
        .await
        .unwrap();
    assert_eq!(store.estimated_count(&elsewhere).await.unwrap(), 6);

    // The source is untouched by every run.
    assert_eq!(all(&store, &source).await, source_docs);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_misshapen_run_and_a_locked_connection_never_write() {
    let name = "a_misshapen_run_and_a_locked_connection_never_write";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let source = dbs.ns("orders");
    insert_labelled(&client, &source, &[doc! { "_id": 1 }]).await;

    for pipeline in [
        vec![doc! { "$match": {} }],
        vec![doc! { "$out": "never_a" }, doc! { "$match": {} }],
        vec![doc! { "$out": "never_b" }, doc! { "$merge": "never_c" }],
    ] {
        let err = store
            .aggregate_write(&source, pipeline, MAX_TIME)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidDocument(_)), "{err:?}");
    }

    let locked = ReadOnlyStore::wrap(Arc::new(store), Arc::new(AtomicBool::new(true)));
    let plan = run_pipeline(&stages(&[r#"{"$out": "never_d"}"#])).unwrap();
    assert_eq!(
        locked
            .aggregate_write(&source, plan.pipeline, MAX_TIME)
            .await,
        Err(CoreError::ReadOnly)
    );

    let names = client
        .database(&dbs.db)
        .list_collection_names()
        .await
        .unwrap();
    assert_eq!(names, vec!["orders".to_string()]);
}
