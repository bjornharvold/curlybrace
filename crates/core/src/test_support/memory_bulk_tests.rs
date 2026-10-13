//! `InMemoryStore::bulk_write` and the defaulted `DocumentStore::bulk_write`.

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use super::memory_store::InMemoryStore;
use super::mock_objects::*;
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, Page, Query, WriteFailure, WriteOp};

fn three() -> (Namespace, InMemoryStore) {
    let ns = mock_namespace();
    let docs = (0..3)
        .map(|i| doc! { "_id": i, "n": i, "a": i % 2, "b": "x" })
        .collect();
    (ns.clone(), InMemoryStore::new().with_docs(&ns, docs))
}

async fn all_docs(s: &InMemoryStore, ns: &Namespace) -> Vec<Document> {
    let q = Query {
        sort: Some(doc! { "_id": 1 }),
        ..Query::default()
    };
    s.find(ns, &q).await.unwrap().docs
}

#[tokio::test]
async fn bulk_write_inserts_replaces_and_deletes_by_id() {
    let (ns, s) = three();
    let ops = vec![
        WriteOp::Insert {
            doc: doc! { "_id": 9, "n": 9 },
        },
        WriteOp::Replace {
            id: Bson::Int32(1),
            doc: doc! { "_id": 1, "n": 100 },
        },
        WriteOp::Delete { id: Bson::Int32(2) },
    ];
    assert_eq!(s.bulk_write(&ns, ops).await.unwrap(), vec![Ok(()); 3]);
    assert_eq!(
        all_docs(&s, &ns).await,
        vec![
            doc! { "_id": 0, "n": 0, "a": 0, "b": "x" },
            doc! { "_id": 1, "n": 100 },
            doc! { "_id": 9, "n": 9 },
        ]
    );
}

#[tokio::test]
async fn bulk_write_reports_missing_for_replace_and_delete_of_an_absent_id() {
    let (ns, s) = three();
    let ops = vec![
        WriteOp::Replace {
            id: Bson::Int32(7),
            doc: doc! { "_id": 7 },
        },
        WriteOp::Delete { id: Bson::Int32(8) },
    ];
    assert_eq!(
        s.bulk_write(&ns, ops).await.unwrap(),
        vec![Err(WriteFailure::Missing); 2]
    );
    assert_eq!(all_docs(&s, &ns).await.len(), 3);
}

#[tokio::test]
async fn bulk_write_reports_duplicate_key_for_an_insert_with_an_existing_id_and_still_applies_the_rest()
 {
    let (ns, s) = three();
    let ops = vec![
        WriteOp::Insert {
            doc: doc! { "_id": 1 },
        },
        WriteOp::Insert {
            doc: doc! { "_id": 5 },
        },
    ];
    assert_eq!(
        s.bulk_write(&ns, ops).await.unwrap(),
        vec![Err(WriteFailure::DuplicateKey), Ok(())]
    );
    assert_eq!(all_docs(&s, &ns).await.len(), 4);
}

#[tokio::test]
async fn bulk_write_on_a_missing_collection_creates_it_for_insert() {
    let s = InMemoryStore::new();
    let ns = mock_namespace();
    let out = s
        .bulk_write(
            &ns,
            vec![WriteOp::Insert {
                doc: doc! { "x": 1 },
            }],
        )
        .await
        .unwrap();
    assert_eq!(out, vec![Ok(())]);
    let docs = all_docs(&s, &ns).await;
    assert_eq!(docs.len(), 1);
    assert!(docs[0].contains_key("_id"));
}

#[tokio::test]
async fn bulk_write_of_an_empty_op_list_is_a_no_op() {
    let (ns, s) = three();
    assert!(s.bulk_write(&ns, vec![]).await.unwrap().is_empty());
    assert_eq!(all_docs(&s, &ns).await.len(), 3);
}

struct Minimal;

#[async_trait]
impl DocumentStore for Minimal {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _: &Namespace, _: &Query) -> Result<Page> {
        Err(CoreError::NotFound)
    }
    async fn insert(&self, _: &Namespace, _: Document) -> Result<Bson> {
        Err(CoreError::NotFound)
    }
    async fn replace(&self, _: &Namespace, _: &Bson, _: Document) -> Result<()> {
        Err(CoreError::NotFound)
    }
    async fn delete(&self, _: &Namespace, _: &Bson) -> Result<()> {
        Err(CoreError::NotFound)
    }
}

#[tokio::test]
async fn bulk_write_default_is_unsupported() {
    let r = Minimal.bulk_write(&mock_namespace(), vec![]).await;
    assert!(matches!(r, Err(CoreError::Unsupported(_))));
}
