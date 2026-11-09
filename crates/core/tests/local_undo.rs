//! Tier 1 (local server): undo of bulk deletes and updates against an already-running MongoDB
//! (`CURLYBRACE_IT_MONGODB_URI`, default `mongodb://127.0.0.1:27017`). Writes only to throwaway
//! `cbit_*` databases. Run with `scripts/integration-tests.sh local_undo`.
//!
//! These exercise the part a fake store cannot: the server-side compare-and-swap that keeps a
//! concurrent edit from being overwritten by an undo.

mod support;

use bson::{Bson, Document, doc};
use curlybrace_core::bulk::{
    BulkDeleteRequest, BulkRunId, BulkUpdateRequest, MatchScope, UpdateSpec,
};
use curlybrace_core::snapshot::{
    SnapshotHeader, SnapshotKind, SnapshotWriter, UndoReport, after_path_for, capture_after,
    run_undo,
};
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use support::{local_enabled, local_mongo};

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

fn header(ns: &Namespace, kind: SnapshotKind) -> SnapshotHeader {
    SnapshotHeader {
        run: BulkRunId(uuid::Uuid::new_v4()),
        connection: uuid::Uuid::nil(),
        ns: ns.clone(),
        kind,
        created_unix: 0,
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_bulk_delete_is_undone_and_documents_recreated_meanwhile_are_kept() {
    if !local_enabled("a_bulk_delete_is_undone_and_documents_recreated_meanwhile_are_kept") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let dir = tempfile::tempdir().unwrap();
    let filter = doc! { "group": "b" };
    let mut writer = SnapshotWriter::create(
        dir.path(),
        header(
            &ns,
            SnapshotKind::Delete {
                filter: filter.clone(),
            },
        ),
        u64::MAX,
    )
    .unwrap();
    let saved = store
        .snapshot_matching(&ns, &filter, &mut writer)
        .await
        .unwrap();
    let path = writer.finish().unwrap();
    assert_eq!(saved, 2);
    store
        .delete_many(
            &ns,
            &BulkDeleteRequest {
                filter,
                scope: MatchScope::Filtered,
            },
            BulkRunId(uuid::Uuid::new_v4()),
        )
        .await
        .unwrap();
    // Someone re-creates document 3 with different content before the undo.
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_one(doc! { "_id": 3, "n": 99 })
        .await
        .unwrap();

    let report = run_undo(&store, &path, None).await.unwrap();

    assert_eq!(
        report,
        UndoReport {
            restored: 1,
            skipped_present: 1,
            ..UndoReport::default()
        }
    );
    let docs = all_docs(&store, &ns).await;
    assert_eq!(docs.len(), 5);
    assert_eq!(
        docs[3],
        doc! { "_id": 3, "n": 99 },
        "the newer document is kept"
    );
    assert_eq!(docs[4], doc! { "_id": 4, "n": 4, "group": "b" });
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_bulk_update_is_undone_but_a_concurrent_edit_is_not_overwritten() {
    if !local_enabled("a_bulk_update_is_undone_but_a_concurrent_edit_is_not_overwritten") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("c");
    seed(&client, &ns).await;
    let coll = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection);
    let before = all_docs(&store, &ns).await;
    let req = BulkUpdateRequest {
        filter: doc! { "group": "a" },
        update: UpdateSpec::Operators(doc! { "$set": { "flag": true, "label": "$notAFieldPath" } }),
        upsert: true,
        scope: MatchScope::Filtered,
    };
    let dir = tempfile::tempdir().unwrap();
    let kind = SnapshotKind::Update {
        filter: req.filter.clone(),
        update: req.update.clone(),
        upsert: true,
    };
    let mut writer = SnapshotWriter::create(dir.path(), header(&ns, kind), u64::MAX).unwrap();
    store
        .snapshot_matching(&ns, &req.filter, &mut writer)
        .await
        .unwrap();
    let path = writer.finish().unwrap();
    store
        .update_many(&ns, &req, BulkRunId(uuid::Uuid::new_v4()))
        .await
        .unwrap();
    capture_after(&store, &path, &[], u64::MAX).await.unwrap();
    // After the update, someone else edits document 1.
    coll.update_one(doc! { "_id": 1 }, doc! { "$set": { "n": 1000 } })
        .await
        .unwrap();

    let report = run_undo(&store, &path, Some(&after_path_for(&path)))
        .await
        .unwrap();

    assert_eq!(
        report,
        UndoReport {
            restored: 2,
            skipped_changed: 1,
            ..UndoReport::default()
        }
    );
    let after = all_docs(&store, &ns).await;
    assert_eq!(after[0], before[0], "document 0 is back as it was");
    assert_eq!(after[2], before[2], "document 2 is back as it was");
    assert_eq!(
        after[1].get("n"),
        Some(&Bson::Int32(1000)),
        "the concurrent edit survives"
    );
    assert_eq!(
        after[1].get("label"),
        Some(&Bson::String("$notAFieldPath".into()))
    );
}
