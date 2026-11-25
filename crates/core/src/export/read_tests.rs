use std::sync::Arc;

use bson::{Document, doc};

use super::read::DocReader;
use crate::error::CoreError;
use crate::store::DocumentStore;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_namespace, mock_numbered_docs};

fn reader(docs: Vec<Document>, filter: Document, batch: usize) -> DocReader {
    let ns = mock_namespace();
    let store: Arc<dyn DocumentStore> = Arc::new(InMemoryStore::new().with_docs(&ns, docs));
    DocReader::new(store, ns, filter, batch)
}

async fn drain(reader: &mut DocReader) -> Vec<Vec<Document>> {
    let mut batches = Vec::new();
    while let Some(batch) = reader.next_batch().await.expect("batch") {
        batches.push(batch);
    }
    batches
}

#[tokio::test]
async fn reads_every_document_once_in_id_order_across_batches() {
    let mut docs = mock_numbered_docs(10);
    docs.reverse();
    docs.swap(2, 7);
    let mut r = reader(docs, doc! {}, 3);
    let batches = drain(&mut r).await;
    assert!(batches.iter().all(|b| b.len() <= 3));
    let flat: Vec<Document> = batches.into_iter().flatten().collect();
    assert_eq!(flat, mock_numbered_docs(10));
}

#[tokio::test]
async fn the_filter_selects_the_documents() {
    let docs: Vec<Document> = (0..10)
        .map(|n| doc! { "_id": n, "tag": if n % 3 == 0 { "keep" } else { "drop" } })
        .collect();
    let expected: Vec<Document> = docs
        .iter()
        .filter(|d| d.get_str("tag") == Ok("keep"))
        .cloned()
        .collect();
    let mut r = reader(docs, doc! { "tag": "keep" }, 2);
    let flat: Vec<Document> = drain(&mut r).await.into_iter().flatten().collect();
    assert_eq!(flat, expected);
    assert_eq!(flat.len(), 4);
}

#[tokio::test]
async fn an_empty_or_missing_collection_yields_no_batches() {
    let mut empty = reader(Vec::new(), doc! {}, 3);
    assert!(drain(&mut empty).await.is_empty());
    let store: Arc<dyn DocumentStore> = Arc::new(InMemoryStore::new());
    let mut missing = DocReader::new(store, mock_namespace(), doc! {}, 3);
    assert!(drain(&mut missing).await.is_empty());
}

fn big_docs(count: i32, bytes: usize) -> Vec<Document> {
    (0..count)
        .map(|n| doc! { "_id": n, "pad": "x".repeat(bytes) })
        .collect()
}

fn size(doc: &Document) -> usize {
    bson::to_vec(doc).expect("serializes").len()
}

#[tokio::test]
async fn a_batch_never_exceeds_the_byte_budget_and_order_and_count_are_unchanged() {
    let docs = big_docs(25, 100_000);
    let budget = 350_000;
    let mut r = reader(docs.clone(), doc! {}, 500).with_byte_budget(budget);
    let batches = drain(&mut r).await;
    assert!(batches.len() > 1);
    for batch in &batches {
        assert!(
            batch.iter().map(size).sum::<usize>() <= budget,
            "over budget"
        );
    }
    let flat: Vec<Document> = batches.into_iter().flatten().collect();
    assert_eq!(flat, docs);
}

#[tokio::test]
async fn a_single_oversized_document_is_still_exported_alone() {
    let mut docs = big_docs(6, 1_000);
    docs[2] = doc! { "_id": 2, "pad": "x".repeat(500_000) };
    let mut r = reader(docs.clone(), doc! {}, 500).with_byte_budget(100_000);
    let batches = drain(&mut r).await;
    let oversized: Vec<&Vec<Document>> = batches
        .iter()
        .filter(|b| b.iter().any(|d| d.get_i32("_id") == Ok(2)))
        .collect();
    assert_eq!(oversized.len(), 1);
    assert_eq!(oversized[0].len(), 1);
    let flat: Vec<Document> = batches.into_iter().flatten().collect();
    assert_eq!(flat, docs);
}

#[tokio::test]
async fn small_documents_still_batch_by_count() {
    let mut r = reader(mock_numbered_docs(20), doc! {}, 5);
    let batches = drain(&mut r).await;
    assert!(batches.iter().all(|b| b.len() <= 5));
    assert!(batches.iter().any(|b| b.len() == 5));
}

#[tokio::test]
async fn an_unusable_id_is_a_typed_error_and_ends_the_read() {
    let mut r = reader(vec![doc! { "_id": [1, 2] }], doc! {}, 3);
    let err = r.next_batch().await.expect_err("array _id");
    assert!(matches!(err, CoreError::CompareKeyInvalid), "{err:?}");
}

mod protocol_tests {
    use std::path::PathBuf;

    use bson::doc;

    use crate::export::protocol::{DocExportRequest, DocFormat, JsonMode};
    use crate::store::Namespace;

    #[test]
    fn debug_of_a_request_never_shows_the_path_or_the_filter() {
        let request = DocExportRequest {
            session: crate::runtime::SessionId::nil(),
            ns: Namespace::new("secretdb", "secretcoll"),
            filter: doc! { "ssn": "123-45-6789" },
            format: DocFormat::Ndjson,
            mode: JsonMode::Relaxed,
            path: PathBuf::from("/home/alice/private.ndjson"),
        };
        let shown = format!("{request:?}");
        assert_eq!(
            shown,
            "DocExportRequest { format: Ndjson, mode: Relaxed, .. }"
        );
    }
}
