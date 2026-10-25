//! Tests for the preview counts, against in-memory stores.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use super::*;
use crate::compare::key::KeyTuple;
use crate::copy::policy::ConflictPolicy;
use crate::store::{DocumentStore, Namespace, Page, Query};
use crate::test_support::InMemoryStore;

fn src_ns() -> Namespace {
    Namespace::new("db", "src")
}

fn dst_ns() -> Namespace {
    Namespace::new("db", "dst")
}

fn job(source: Vec<Document>, target: Option<Vec<Document>>, filter: Document) -> CopyJob {
    let target_store = match target {
        Some(docs) => InMemoryStore::new().with_docs(&dst_ns(), docs),
        None => InMemoryStore::new(),
    };
    CopyJob {
        source: Arc::new(InMemoryStore::new().with_docs(&src_ns(), source)),
        source_ns: src_ns(),
        filter,
        target: Arc::new(target_store),
        target_ns: dst_ns(),
        policy: ConflictPolicy::Skip,
        batch: 10,
    }
}

async fn preview(job: &CopyJob) -> (EngineOutcome, PreviewCounts) {
    preview_collection(job, &CancelFlag::default(), &mut |_| {})
        .await
        .unwrap()
}

fn counts(new: u64, changed: u64, unchanged: u64) -> PreviewCounts {
    PreviewCounts {
        new,
        changed,
        unchanged,
    }
}

#[tokio::test]
async fn counts_new_changed_unchanged_between_two_stores() {
    let source = vec![
        doc! { "_id": 1, "v": 1 },
        doc! { "_id": 2, "v": 2 },
        doc! { "_id": 3, "v": 3 },
        doc! { "_id": 4, "v": 4 },
    ];
    let target = vec![doc! { "_id": 2, "v": 2 }, doc! { "_id": 3, "v": 99 }];
    let (outcome, got) = preview(&job(source, Some(target), doc! {})).await;
    assert_eq!(outcome, EngineOutcome::Completed);
    assert_eq!(got, counts(2, 1, 1));
}

#[tokio::test]
async fn target_only_documents_are_not_counted() {
    let source = vec![doc! { "_id": 1 }];
    let target = vec![doc! { "_id": 1 }, doc! { "_id": 8 }, doc! { "_id": 9 }];
    let (_, got) = preview(&job(source, Some(target), doc! {})).await;
    assert_eq!(got, counts(0, 0, 1));
}

#[tokio::test]
async fn missing_target_collection_counts_all_as_new() {
    let source: Vec<Document> = (0..5).map(|n| doc! { "_id": n }).collect();
    let (outcome, got) = preview(&job(source, None, doc! {})).await;
    assert_eq!(outcome, EngineOutcome::Completed);
    assert_eq!(got, counts(5, 0, 0));
}

#[tokio::test]
async fn filter_applies_to_the_source_side_only() {
    let source = vec![
        doc! { "_id": 1, "k": "a" },
        doc! { "_id": 2, "k": "b" },
        doc! { "_id": 3, "k": "a" },
    ];
    // The target copy of _id 1 has k "z", so it does not match the filter. Filtered on the source
    // side only it is found and counts as changed; had the filter leaked to the target it would be
    // hidden and _id 1 would count as new. _id 2 is excluded from the source: it is not counted.
    let target = vec![doc! { "_id": 1, "k": "z" }, doc! { "_id": 2, "k": "b" }];
    let (_, got) = preview(&job(source, Some(target), doc! { "k": "a" })).await;
    assert_eq!(got, counts(1, 1, 0));
}

#[tokio::test]
async fn cancel_returns_cancelled_outcome() {
    let source: Vec<Document> = (0..5).map(|n| doc! { "_id": n }).collect();
    let cancel = CancelFlag::default();
    cancel.cancel();
    let (outcome, _) = preview_collection(&job(source, None, doc! {}), &cancel, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(outcome, EngineOutcome::Cancelled);
}

/// An in-memory store that counts the documents its sorted scans hand back.
struct Counting {
    inner: InMemoryStore,
    returned: AtomicUsize,
}

#[async_trait]
impl DocumentStore for Counting {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }

    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }

    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        self.inner.insert(ns, doc).await
    }

    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }

    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        let docs = self.inner.scan_sorted(ns, filter, key, from, limit).await?;
        self.returned.fetch_add(docs.len(), Ordering::SeqCst);
        Ok(docs)
    }
}

fn counting_job(filter: Document) -> (CopyJob, Arc<Counting>) {
    let source: Vec<Document> = (0..20).map(|n| doc! { "_id": n, "v": n }).collect();
    let target_docs: Vec<Document> = (0..200).map(|n| doc! { "_id": n, "v": n }).collect();
    let target = Arc::new(Counting {
        inner: InMemoryStore::new().with_docs(&dst_ns(), target_docs),
        returned: AtomicUsize::new(0),
    });
    let job = CopyJob {
        target: target.clone(),
        ..job(source, None, filter)
    };
    (job, target)
}

#[tokio::test]
async fn an_id_list_selection_scans_only_those_ids_of_the_target() {
    let filter = doc! { "_id": { "$in": [1, 2, 3, 500] } };
    let (job, target) = counting_job(filter);
    let (outcome, got) = preview(&job).await;
    assert_eq!(outcome, EngineOutcome::Completed);
    assert_eq!(got, counts(0, 0, 3));
    assert_eq!(target.returned.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn any_other_filter_still_scans_the_whole_target() {
    let (job, target) = counting_job(doc! { "v": 5 });
    let (_, got) = preview(&job).await;
    assert_eq!(got, counts(0, 0, 1));
    assert_eq!(target.returned.load(Ordering::SeqCst), 200);
}
