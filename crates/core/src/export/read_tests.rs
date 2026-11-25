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
