//! Tier 1 (local server): bulk update / delete / count against an already-running MongoDB
//! (`CURLYBRACE_IT_MONGODB_URI`, default `mongodb://127.0.0.1:27017`). Writes only to throwaway
//! `cbit_*` databases. Run with `scripts/integration-tests.sh local_bulk`.
//!
//! The preview tests that run the update in a transaction need a replica set (a standalone
//! `mongod` has no transactions). Start one with `mongod --replSet rs0 --dbpath <dir>` and run
//! `rs.initiate()` once. On a standalone server those tests detect it (`hello.setName`) and skip
//! with a printed message; the degradation path itself is covered by `docker_bulk_preview.rs`.

mod support;

use std::time::Duration;

use bson::{Bson, Document, doc};
use curlybrace_core::bulk::{
    BulkDeleteRequest, BulkFailureReason, BulkOutcome, BulkRunId, BulkUpdateRequest, MatchScope,
    PreviewGap, UpdateSpec,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use support::{local_enabled, local_mongo, replica_set_or_skip};

const WAIT: Duration = Duration::from_secs(30);

fn run_id() -> BulkRunId {
    BulkRunId(uuid::Uuid::new_v4())
}

/// Five documents, `_id` 0..5; `n` is 0..5 and `group` is "a" for ids 0..3, "b" otherwise.
async fn seed(client: &mongodb::Client, ns: &Namespace) {
    let docs: Vec<Document> = (0..5)
        .map(|n| doc! { "_id": n, "n": n, "group": if n < 3 { "a" } else { "b" } })
        .collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
}

async fn all_docs(store: &impl DocumentStore, ns: &Namespace) -> Vec<Document> {
    let query = Query {
        sort: Some(doc! { "_id": 1 }),
        ..Query::default()
    };
    store.find(ns, &query).await.unwrap().docs
}

fn set_update(filter: Document, scope: MatchScope, upsert: bool) -> BulkUpdateRequest {
    BulkUpdateRequest {
        filter,
        update: UpdateSpec::Operators(doc! { "$set": { "flag": true } }),
        upsert,
        scope,
    }
}

fn applied(matched: u64, modified: u64, deleted: u64) -> BulkOutcome {
    BulkOutcome::Applied {
        matched,
        modified,
        upserted: vec![],
        deleted,
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn count_matching_counts_the_filter() {
    if !local_enabled("count_matching_counts_the_filter") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let got = store
        .count_matching(&ns, &doc! { "group": "a" }, WAIT)
        .await
        .unwrap();
    assert_eq!(got, Some(3));
    let all = store.count_matching(&ns, &doc! {}, WAIT).await.unwrap();
    assert_eq!(all, Some(5));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn count_matching_never_errors_on_a_tiny_max_time() {
    if !local_enabled("count_matching_never_errors_on_a_tiny_max_time") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    // The server may be fast enough to finish: either answer is fine, an error is not.
    let got = store
        .count_matching(
            &ns,
            &doc! { "$expr": { "$gt": ["$n", -1] } },
            Duration::from_millis(1),
        )
        .await
        .unwrap();
    assert!(got.is_none() || got == Some(5));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_sets_every_match_and_reports_counts() {
    if !local_enabled("update_many_sets_every_match_and_reports_counts") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = set_update(doc! { "group": "a" }, MatchScope::Filtered, false);
    let got = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(got, applied(3, 3, 0));
    let flagged = all_docs(&store, &ns)
        .await
        .iter()
        .filter(|d| d.get_bool("flag") == Ok(true))
        .count();
    assert_eq!(flagged, 3);
    let again = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(again, applied(3, 0, 0));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_upsert_reports_the_new_id() {
    if !local_enabled("update_many_upsert_reports_the_new_id") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = set_update(doc! { "_id": 99 }, MatchScope::Filtered, true);
    let got = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(
        got,
        BulkOutcome::Applied {
            matched: 0,
            modified: 0,
            upserted: vec![Bson::Int32(99)],
            deleted: 0
        }
    );
    assert_eq!(all_docs(&store, &ns).await.len(), 6);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_pipeline_adds_a_field() {
    if !local_enabled("update_many_pipeline_adds_a_field") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = BulkUpdateRequest {
        filter: doc! { "group": "b" },
        update: UpdateSpec::Pipeline(vec![
            doc! { "$set": { "double": { "$multiply": ["$n", 2] } } },
        ]),
        upsert: false,
        scope: MatchScope::Filtered,
    };
    let got = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(got, applied(2, 2, 0));
    let docs = all_docs(&store, &ns).await;
    assert_eq!(docs[3].get_i32("double"), Ok(6));
    assert!(docs[0].get("double").is_none());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_with_empty_filter_and_filtered_scope_touches_nothing() {
    if !local_enabled("update_many_with_empty_filter_and_filtered_scope_touches_nothing") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let req = set_update(doc! {}, MatchScope::Filtered, false);
    let err = store.update_many(&ns, &req, run_id()).await.unwrap_err();
    assert_eq!(err, CoreError::BulkEmptyFilter);
    assert_eq!(all_docs(&store, &ns).await, before);
}

/// A unique index on `u` over the five seeded documents (`u` = `n`).
async fn seed_unique(client: &mongodb::Client, ns: &Namespace) {
    seed(client, ns).await;
    let coll = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection);
    coll.update_many(doc! {}, vec![doc! { "$set": { "u": "$n" } }])
        .await
        .unwrap();
    let index = mongodb::IndexModel::builder()
        .keys(doc! { "u": 1 })
        .options(
            mongodb::options::IndexOptions::builder()
                .unique(true)
                .build(),
        )
        .build();
    coll.create_index(index).await.unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_that_collides_midway_is_rejected_and_may_be_partial() {
    if !local_enabled("update_many_that_collides_midway_is_rejected_and_may_be_partial") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed_unique(&client, &ns).await;
    // Every document gets u = 99: the first one succeeds, the second collides with it.
    let req = BulkUpdateRequest {
        update: UpdateSpec::Operators(doc! { "$set": { "u": 99 } }),
        ..set_update(doc! {}, MatchScope::AllDocuments, false)
    };
    let got = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(
        got,
        BulkOutcome::Rejected {
            reason: BulkFailureReason::DuplicateKey
        }
    );
    // The reply's counts read 0, yet the documents written before the failure stay written.
    let changed = all_docs(&store, &ns)
        .await
        .iter()
        .filter(|d| d.get_i32("u") == Ok(99))
        .count();
    assert_eq!(changed, 1);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_failing_validation_midway_is_rejected_as_validation_failed() {
    if !local_enabled("update_many_failing_validation_midway_is_rejected_as_validation_failed") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    let db = client.database(&ns.db);
    db.create_collection(&ns.collection)
        .validator(doc! { "n": { "$lt": 3 } })
        .await
        .unwrap();
    seed_below(&client, &ns).await;
    let req = BulkUpdateRequest {
        update: UpdateSpec::Pipeline(vec![doc! { "$set": { "n": "$_id" } }]),
        ..set_update(doc! {}, MatchScope::AllDocuments, false)
    };
    let got = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(
        got,
        BulkOutcome::Rejected {
            reason: BulkFailureReason::ValidationFailed
        }
    );
}

/// Five documents with `n` = 0 that satisfy the validator `n < 3`.
async fn seed_below(client: &mongodb::Client, ns: &Namespace) {
    let docs: Vec<Document> = (0..5).map(|id| doc! { "_id": id, "n": 0 }).collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_many_all_documents_updates_all() {
    if !local_enabled("update_many_all_documents_updates_all") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = set_update(doc! {}, MatchScope::AllDocuments, false);
    let got = store.update_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(got, applied(5, 5, 0));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn delete_many_removes_matches_and_reports_deleted() {
    if !local_enabled("delete_many_removes_matches_and_reports_deleted") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = BulkDeleteRequest {
        filter: doc! { "group": "b" },
        scope: MatchScope::Filtered,
    };
    let got = store.delete_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(got, applied(2, 0, 2));
    assert_eq!(all_docs(&store, &ns).await.len(), 3);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn delete_many_empty_filter_without_flag_deletes_nothing() {
    if !local_enabled("delete_many_empty_filter_without_flag_deletes_nothing") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = BulkDeleteRequest {
        filter: doc! {},
        scope: MatchScope::Filtered,
    };
    let err = store.delete_many(&ns, &req, run_id()).await.unwrap_err();
    assert_eq!(err, CoreError::BulkEmptyFilter);
    assert_eq!(all_docs(&store, &ns).await.len(), 5);
    let req = BulkDeleteRequest {
        filter: doc! {},
        scope: MatchScope::AllDocuments,
    };
    let got = store.delete_many(&ns, &req, run_id()).await.unwrap();
    assert_eq!(got, applied(5, 0, 5));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn invalid_update_is_rejected_before_the_server() {
    if !local_enabled("invalid_update_is_rejected_before_the_server") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let req = BulkUpdateRequest {
        filter: doc! { "group": "a" },
        update: UpdateSpec::Operators(doc! { "replaced": 1 }),
        upsert: false,
        scope: MatchScope::Filtered,
    };
    let err = store.update_many(&ns, &req, run_id()).await.unwrap_err();
    assert_eq!(err, CoreError::BulkInvalidUpdate);
    assert_eq!(all_docs(&store, &ns).await, before);
}

fn update_req(filter: Document, update: UpdateSpec, upsert: bool) -> BulkUpdateRequest {
    BulkUpdateRequest {
        filter,
        update,
        upsert,
        scope: MatchScope::Filtered,
    }
}

fn doc_by_id(docs: &[Document], id: &Bson) -> Document {
    docs.iter()
        .find(|d| d.get("_id") == Some(id))
        .cloned()
        .expect("document present")
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_after_matches_what_update_many_then_produces() {
    if !local_enabled("preview_after_matches_what_update_many_then_produces") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    if !replica_set_or_skip(
        &client,
        "preview_after_matches_what_update_many_then_produces",
    )
    .await
    {
        return;
    }
    let updates = [
        UpdateSpec::Operators(doc! { "$set": { "flag": true } }),
        UpdateSpec::Operators(doc! { "$inc": { "n": 5 } }),
        UpdateSpec::Operators(doc! { "$push": { "tags": "x" } }),
        UpdateSpec::Pipeline(vec![doc! { "$set": { "n2": { "$add": ["$n", 10] } } }]),
    ];
    for (i, update) in updates.into_iter().enumerate() {
        let ns = dbs.ns(&format!("c{i}"));
        seed(&client, &ns).await;
        let req = update_req(doc! { "group": "a" }, update, false);
        let preview = store.preview_update(&ns, &req).await.unwrap();
        assert_eq!(preview.gap, None, "update #{i}");
        assert_eq!(preview.docs.len(), 3, "update #{i}");
        store.update_many(&ns, &req, run_id()).await.unwrap();
        let stored = all_docs(&store, &ns).await;
        for pair in &preview.docs {
            let id = pair.before.get("_id").unwrap();
            let after = pair.after.clone().expect("an after-state");
            assert_eq!(after, doc_by_id(&stored, id), "update #{i}");
            assert_ne!(pair.before, after, "update #{i}");
        }
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_leaves_the_collection_unchanged() {
    if !local_enabled("preview_leaves_the_collection_unchanged") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    if !replica_set_or_skip(&client, "preview_leaves_the_collection_unchanged").await {
        return;
    }
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let set = update_req(
        doc! { "group": "a" },
        UpdateSpec::Operators(doc! { "$set": { "flag": true } }),
        false,
    );
    let preview = store.preview_update(&ns, &set).await.unwrap();
    assert_eq!(preview.gap, None);
    assert_eq!(all_docs(&store, &ns).await, before);

    // An update that errors on the server ($inc on a string) fails the preview, still writes
    // nothing, and leaves the connection usable.
    let bad = update_req(
        doc! { "group": "a" },
        UpdateSpec::Operators(doc! { "$inc": { "group": 1 } }),
        false,
    );
    let err = store.preview_update(&ns, &bad).await.unwrap_err();
    assert_eq!(err, CoreError::BulkInvalidUpdate);
    assert_eq!(all_docs(&store, &ns).await, before);
    let again = store.preview_update(&ns, &set).await.unwrap();
    assert_eq!(again.docs.len(), 3);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_with_an_empty_filter_and_filtered_scope_is_refused() {
    if !local_enabled("preview_with_an_empty_filter_and_filtered_scope_is_refused") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = set_update(doc! {}, MatchScope::Filtered, false);
    let err = store.preview_update(&ns, &req).await.unwrap_err();
    assert_eq!(err, CoreError::BulkEmptyFilter);
    let all = set_update(doc! {}, MatchScope::AllDocuments, false);
    assert_eq!(store.preview_update(&ns, &all).await.unwrap().docs.len(), 3);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn upsert_with_no_match_previews_the_inserted_document_and_inserts_nothing() {
    if !local_enabled("upsert_with_no_match_previews_the_inserted_document_and_inserts_nothing") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    if !replica_set_or_skip(
        &client,
        "upsert_with_no_match_previews_the_inserted_document_and_inserts_nothing",
    )
    .await
    {
        return;
    }
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let req = set_update(doc! { "_id": 99 }, MatchScope::Filtered, true);
    let preview = store.preview_update(&ns, &req).await.unwrap();
    assert_eq!(preview.gap, None);
    assert_eq!(preview.docs.len(), 1);
    // There is no document before an insert: `before` is the empty document.
    assert!(preview.docs[0].before.is_empty());
    assert_eq!(
        preview.docs[0].after,
        Some(doc! { "_id": 99, "flag": true })
    );
    assert_eq!(all_docs(&store, &ns).await, before);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_with_no_match_and_no_upsert_reports_nomatch() {
    if !local_enabled("preview_with_no_match_and_no_upsert_reports_nomatch") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let req = set_update(doc! { "group": "zzz" }, MatchScope::Filtered, false);
    let preview = store.preview_update(&ns, &req).await.unwrap();
    assert!(preview.docs.is_empty());
    assert_eq!(preview.gap, Some(PreviewGap::NoMatch));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_on_capped_collection_degrades() {
    if !local_enabled("preview_on_capped_collection_degrades") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("capped");
    let db = client.database(&ns.db);
    db.create_collection(&ns.collection)
        .capped(true)
        .size(4096)
        .await
        .unwrap();
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let req = set_update(doc! { "group": "a" }, MatchScope::Filtered, false);
    let preview = store.preview_update(&ns, &req).await.unwrap();
    assert_eq!(preview.gap, Some(PreviewGap::TransactionsUnavailable));
    assert_eq!(preview.docs.len(), 3);
    assert!(preview.docs.iter().all(|d| d.after.is_none()));
    assert_eq!(all_docs(&store, &ns).await, before);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_reports_data_errors_of_the_update_as_invalid() {
    if !local_enabled("preview_reports_data_errors_of_the_update_as_invalid") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    if !replica_set_or_skip(
        &client,
        "preview_reports_data_errors_of_the_update_as_invalid",
    )
    .await
    {
        return;
    }
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let updates = [
        doc! { "$set": { "n.x": 1 } },   // 28 PathNotViable: `n` is a number
        doc! { "$set": { "_id": 42 } },  // 66 ImmutableField
        doc! { "$inc": { "group": 1 } }, // 14 TypeMismatch
        doc! { "$sett": { "a": 1 } },    // 9 FailedToParse: an unknown operator, not a gap
    ];
    for update in updates {
        let req = update_req(doc! { "group": "a" }, UpdateSpec::Operators(update), false);
        let err = store.preview_update(&ns, &req).await.unwrap_err();
        assert_eq!(err, CoreError::BulkInvalidUpdate);
    }
    assert_eq!(all_docs(&store, &ns).await, before);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_rejects_an_invalid_update_spec_before_touching_anything() {
    if !local_enabled("preview_rejects_an_invalid_update_spec_before_touching_anything") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    let req = update_req(
        doc! { "group": "a" },
        UpdateSpec::Operators(doc! { "replaced": 1 }),
        false,
    );
    let err = store.preview_update(&ns, &req).await.unwrap_err();
    assert_eq!(err, CoreError::BulkInvalidUpdate);
    assert_eq!(all_docs(&store, &ns).await, before);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn preview_during_a_write_conflict_reports_conflict() {
    if !local_enabled("preview_during_a_write_conflict_reports_conflict") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    if !replica_set_or_skip(&client, "preview_during_a_write_conflict_reports_conflict").await {
        return;
    }
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let before = all_docs(&store, &ns).await;
    // A second session holds an uncommitted transaction write on _id 0.
    let mut holder = client.start_session().await.unwrap();
    holder.start_transaction().await.unwrap();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .update_one(doc! { "_id": 0 }, doc! { "$set": { "held": true } })
        .session(&mut holder)
        .await
        .unwrap();
    let req = set_update(doc! { "group": "a" }, MatchScope::Filtered, false);
    let preview = store.preview_update(&ns, &req).await.unwrap();
    holder.abort_transaction().await.unwrap();
    assert_eq!(preview.gap, Some(PreviewGap::Conflict));
    assert!(preview.docs.iter().all(|d| d.after.is_none()));
    assert_eq!(all_docs(&store, &ns).await, before);
}

/// Opids of the operations whose `command.comment` is the run id. Deliberately its own
/// `$currentOp` pipeline, an independent oracle of the one `cancel_bulk` runs in production.
/// Returns an error instead of panicking so callers can clean up first.
async fn ops_for_run(client: &mongodb::Client, run: BulkRunId) -> Result<Vec<i64>, String> {
    use futures::TryStreamExt;
    let pipeline = vec![
        doc! { "$currentOp": {} },
        doc! { "$match": { "command.comment": run.0.to_string() } },
    ];
    let cursor = client
        .database("admin")
        .aggregate(pipeline)
        .await
        .map_err(|e| format!("$currentOp on admin must be allowed on the test server: {e}"))?;
    let ops: Vec<Document> = cursor.try_collect().await.map_err(|e| e.to_string())?;
    Ok(ops
        .iter()
        .filter_map(|op| match op.get("opid") {
            Some(Bson::Int32(n)) => Some(i64::from(*n)),
            Some(Bson::Int64(n)) => Some(*n),
            _ => None,
        })
        .collect())
}

/// Waits (bounded) until the run's operation shows in `currentOp`; the error says why not.
async fn wait_for_op(
    client: &mongodb::Client,
    run: BulkRunId,
    task: &tokio::task::JoinHandle<Result<BulkOutcome, CoreError>>,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if !ops_for_run(client, run).await?.is_empty() {
            return Ok(());
        }
        if task.is_finished() {
            return Err(
                "the bulk write ended before currentOp showed it: the server rejected \
                        the `comment` or the write was not slow enough"
                    .into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(
        "currentOp never showed an operation with command.comment == the run id: the comment \
         path is wrong or the server did not accept the comment"
            .into(),
    )
}

/// 300 documents; an update filtered by `$where` sleeping 20 ms per document runs for ~6 s.
async fn seed_slow(client: &mongodb::Client, ns: &Namespace) {
    let docs: Vec<Document> = (0..300).map(|n| doc! { "_id": n, "n": n }).collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
}

fn slow_update() -> BulkUpdateRequest {
    BulkUpdateRequest {
        filter: doc! { "$where": "sleep(20); true" },
        update: UpdateSpec::Operators(doc! { "$set": { "flag": true } }),
        upsert: false,
        scope: MatchScope::Filtered,
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancel_stops_a_slow_update_and_reports_unknown_cancelled() {
    if !local_enabled("cancel_stops_a_slow_update_and_reports_unknown_cancelled") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed_slow(&client, &ns).await;
    let store = std::sync::Arc::new(store);
    let run = run_id();
    let task = {
        let (store, ns) = (store.clone(), ns.clone());
        tokio::spawn(async move { store.update_many(&ns, &slow_update(), run).await })
    };
    let seen = wait_for_op(&client, run, &task).await;
    // Cleanup is unconditional and happens before any assertion can fail: keep killing the run
    // until the task returns (an early failure may precede the op's registration), bounded.
    let cancelled = store.cancel_bulk(run).await;
    let mut task = task;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let outcome = loop {
        let wait = tokio::time::timeout(Duration::from_millis(50), &mut task).await;
        if let Ok(done) = wait {
            break Some(done);
        }
        if tokio::time::Instant::now() > deadline {
            task.abort();
            break None;
        }
        let _ = store.cancel_bulk(run).await;
    };
    let left = ops_for_run(&client, run).await;

    seen.unwrap();
    cancelled.unwrap();
    let outcome = outcome.expect("the killed update must return").unwrap();
    assert_eq!(outcome.unwrap(), BulkOutcome::Unknown { cancelled: true });
    assert_eq!(left.unwrap(), Vec::<i64>::new());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancel_after_completion_returns_the_real_counts() {
    if !local_enabled("cancel_after_completion_returns_the_real_counts") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let run = run_id();
    let req = set_update(doc! { "group": "a" }, MatchScope::Filtered, false);
    let got = store.update_many(&ns, &req, run).await.unwrap();
    assert_eq!(got, applied(3, 3, 0));
    store.cancel_bulk(run).await.unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancel_of_an_unknown_run_is_ok() {
    if !local_enabled("cancel_of_an_unknown_run_is_ok") {
        return;
    }
    let (_dbs, _client, store) = local_mongo().await;
    store.cancel_bulk(run_id()).await.unwrap();
}
