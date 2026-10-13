//! Tier 1 (local server): `MongoStore::bulk_write` with payloads larger than one write command
//! can carry. Writes only to throwaway `cbit_*` databases. See docs/superpowers/notes/testing.md
//! for running tier 1 against a dev container of your own.

mod support;

use bson::{Bson, Document, doc};
use curlybrace_core::store::{DocumentStore, WriteOp};
use support::{local_enabled, local_mongo};

const DOCS: i32 = 20;
const PAYLOAD_BYTES: usize = 1_000_000;

fn payload(tag: char) -> String {
    tag.to_string().repeat(PAYLOAD_BYTES)
}

async fn count(client: &mongodb::Client, ns: &curlybrace_core::store::Namespace) -> u64 {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .count_documents(doc! {})
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_splits_commands_by_bytes_for_insert_replace_and_delete() {
    if !local_enabled("bulk_write_splits_commands_by_bytes_for_insert_replace_and_delete") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("large");
    let inserts = (0..DOCS)
        .map(|n| WriteOp::Insert {
            doc: doc! { "_id": n, "p": payload('a') },
        })
        .collect();
    let outcomes = store.bulk_write(&ns, inserts).await.unwrap();
    assert_eq!(outcomes, vec![Ok(()); DOCS as usize]);
    assert_eq!(count(&client, &ns).await, DOCS as u64);

    let replaces = (0..DOCS)
        .map(|n| WriteOp::Replace {
            id: Bson::Int32(n),
            doc: doc! { "_id": n, "p": payload('b') },
        })
        .collect();
    let outcomes = store.bulk_write(&ns, replaces).await.unwrap();
    assert_eq!(outcomes, vec![Ok(()); DOCS as usize]);
    let first = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find_one(doc! { "_id": 7 })
        .await
        .unwrap()
        .unwrap();
    assert!(first.get_str("p").unwrap().starts_with('b'));

    let deletes = (0..DOCS)
        .map(|n| WriteOp::Delete { id: Bson::Int32(n) })
        .collect();
    let outcomes = store.bulk_write(&ns, deletes).await.unwrap();
    assert_eq!(outcomes, vec![Ok(()); DOCS as usize]);
    assert_eq!(count(&client, &ns).await, 0);
}
