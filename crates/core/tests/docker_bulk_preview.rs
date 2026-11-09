//! The update preview against a STANDALONE server, which has no transactions: it must degrade to
//! the "before" documents with a gap and write nothing.
//! Tier 2 (docker): own `cb-it-*` container; run with `scripts/container-tests.sh`.

mod support;

use bson::{Document, doc};
use curlybrace_core::bulk::{BulkUpdateRequest, MatchScope, PreviewGap, UpdateSpec};
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use support::{docker_enabled, docker_mongo};

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn preview_on_standalone_degrades_to_before_only() {
    if !docker_enabled("preview_on_standalone_degrades_to_before_only") {
        return;
    }
    let (_container, client, store) = docker_mongo().await;
    let ns = Namespace::new("cbit_preview", "c");
    let docs: Vec<Document> = (0..5).map(|n| doc! { "_id": n, "group": "a" }).collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
    let req = BulkUpdateRequest {
        filter: doc! { "group": "a" },
        update: UpdateSpec::Operators(doc! { "$set": { "flag": true } }),
        upsert: false,
        scope: MatchScope::Filtered,
    };
    let preview = store.preview_update(&ns, &req).await.unwrap();
    assert_eq!(preview.gap, Some(PreviewGap::TransactionsUnavailable));
    assert_eq!(preview.docs.len(), 3);
    assert!(preview.docs.iter().all(|d| d.after.is_none()));
    let stored = store.find(&ns, &Query::default()).await.unwrap().docs;
    assert_eq!(stored.len(), 5);
    assert!(stored.iter().all(|d| !d.contains_key("flag")));
}
