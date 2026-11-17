//! Tier 1 (local server): what the Browse find sends, seen from the server's profiler: the
//! comment, `allowDiskUse`, and no `maxTimeMS` or other option when none is set. Writes only to
//! throwaway `cbit_*` databases. Run against a throwaway `mongod` (never a server that holds
//! data).

mod support;

use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::pipeline::PipelineOptions;
use curlybrace_core::store::{DocumentStore, Query};
use mongodb::Client;
use support::{insert_labelled, local_enabled, local_mongo};

fn query(filter: Document, options: PipelineOptions) -> Query {
    Query {
        filter,
        limit: 0,
        options,
        ..Query::default()
    }
}

async fn profiled(client: &Client, db: &str, filter: Document) -> Option<Document> {
    let profile = client.database(db).collection::<Document>("system.profile");
    for _ in 0..20 {
        if let Some(entry) = profile.find_one(filter.clone()).await.unwrap() {
            return Some(entry);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

async fn set_profiling(client: &Client, db: &str, level: i32) {
    client
        .database(db)
        .run_command(doc! { "profile": level })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn the_comment_reaches_the_profiler() {
    if !local_enabled("the_comment_reaches_the_profiler") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1 }]).await;
    set_profiling(&client, &ns.db, 2).await;
    let options = PipelineOptions {
        comment: Some("curlybrace-it-find-comment".into()),
        ..PipelineOptions::default()
    };
    store.find(&ns, &query(doc! {}, options)).await.unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.find": "orders", "command.comment": "curlybrace-it-find-comment" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    assert!(entry.is_some(), "no profiled find carried the comment");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn allow_disk_use_reaches_the_server() {
    if !local_enabled("allow_disk_use_reaches_the_server") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    let docs: Vec<Document> = (0..50).map(|i| doc! { "_id": i, "k": i % 7 }).collect();
    insert_labelled(&client, &ns, &docs).await;
    set_profiling(&client, &ns.db, 2).await;
    let options = PipelineOptions {
        allow_disk_use: Some(true),
        comment: Some("curlybrace-it-find-disk".into()),
        ..PipelineOptions::default()
    };
    let q = Query {
        sort: Some(doc! { "k": 1 }),
        ..query(doc! {}, options)
    };
    store.find(&ns, &q).await.unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.find": "orders", "command.comment": "curlybrace-it-find-disk" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    let command = entry
        .expect("the profiled find")
        .get_document("command")
        .cloned();
    assert_eq!(command.unwrap().get_bool("allowDiskUse"), Ok(true));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn the_default_find_sends_no_max_time() {
    if !local_enabled("the_default_find_sends_no_max_time") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1, "tag": "plain-find" }]).await;
    set_profiling(&client, &ns.db, 2).await;
    store
        .find(
            &ns,
            &query(doc! { "tag": "plain-find" }, PipelineOptions::default()),
        )
        .await
        .unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.find": "orders", "command.filter.tag": "plain-find" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    let command = entry
        .expect("the profiled find")
        .get_document("command")
        .cloned()
        .unwrap();
    assert!(!command.contains_key("maxTimeMS"), "{command:?}");
    assert!(!command.contains_key("allowDiskUse"), "{command:?}");
    assert!(!command.contains_key("comment"), "{command:?}");
}
