//! Tests for the streaming copy engine, against in-memory stores.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bson::doc;

use super::*;
use crate::compare::apply::APPLY_BATCH;
use crate::compare::key::compare_bson;
use crate::store::Page;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_docs_with_mixed_ids;

fn src_ns() -> Namespace {
    Namespace::new("db", "src")
}

fn dst_ns() -> Namespace {
    Namespace::new("db", "dst")
}

/// How a `Probe` makes writes fail.
#[derive(Clone, Copy)]
enum Fail {
    Never,
    /// The n-th `bulk_write` call (1-based) fails as a whole.
    CallNumber(usize),
    /// Every op of every `bulk_write` is refused, and nothing is written.
    EveryOp,
}

/// An in-memory store that counts scans and bulk writes and can fail writes.
struct Probe {
    inner: InMemoryStore,
    fail: Fail,
    scans: AtomicUsize,
    writes: AtomicUsize,
}

impl Probe {
    fn new(ns: &Namespace, docs: Vec<Document>, fail: Fail) -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryStore::new().with_docs(ns, docs),
            fail,
            scans: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
        })
    }

    async fn contents(&self, ns: &Namespace) -> Vec<Document> {
        let query = Query {
            limit: 0,
            ..Query::default()
        };
        self.inner.find(ns, &query).await.unwrap().docs
    }
}

#[async_trait]
impl DocumentStore for Probe {
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

    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        let call = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        match self.fail {
            Fail::CallNumber(n) if n == call => Err(CoreError::ApplyOutcomeUnknown),
            Fail::EveryOp => Ok(ops
                .iter()
                .map(|_| Err(WriteFailure::Rejected(121)))
                .collect()),
            _ => self.inner.bulk_write(ns, ops).await,
        }
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
}

fn job(source: &Arc<Probe>, target: &Arc<Probe>, policy: ConflictPolicy, batch: usize) -> CopyJob {
    CopyJob {
        source: source.clone(),
        source_ns: src_ns(),
        filter: Document::new(),
        target: target.clone(),
        target_ns: dst_ns(),
        policy,
        batch,
    }
}

async fn run(job: CopyJob) -> CopyReport {
    copy_collection(job, &CancelFlag::default(), &mut |_| {}).await
}

fn numbered(count: i32) -> Vec<Document> {
    (0..count)
        .map(|n| doc! { "_id": (n * 7919) % count, "even": n % 2 == 0 })
        .collect()
}

fn id_of(doc: &Document) -> &Bson {
    doc.get("_id").unwrap()
}

#[tokio::test]
async fn copies_all_documents_in_id_order_across_batches() {
    let source = Probe::new(&src_ns(), numbered(1250), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let mut calls = 0;
    let report = copy_collection(
        job(&source, &target, ConflictPolicy::Skip, APPLY_BATCH),
        &CancelFlag::default(),
        &mut |_| calls += 1,
    )
    .await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(
        report.progress,
        CopyProgress {
            scanned: 1250,
            written: 1250,
            skipped: 0,
            failed: 0
        }
    );
    assert_eq!(calls, 3);
    let ids: Vec<i32> = target
        .contents(&dst_ns())
        .await
        .iter()
        .map(|d| d.get_i32("_id").unwrap())
        .collect();
    assert_eq!(ids, (0..1250).collect::<Vec<i32>>());
}

#[tokio::test]
async fn mixed_id_types_are_neither_skipped_nor_duplicated_at_batch_boundaries() {
    let docs = mock_docs_with_mixed_ids();
    let source = Probe::new(&src_ns(), docs.clone(), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let report = run(job(&source, &target, ConflictPolicy::Skip, 3)).await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(report.progress.scanned, docs.len() as u64);
    assert_eq!(report.progress.written, docs.len() as u64);
    let mut want: Vec<Document> = docs;
    want.sort_by(|a, b| compare_bson(id_of(a), id_of(b)));
    assert_eq!(target.contents(&dst_ns()).await, want);
}

#[tokio::test]
async fn filter_limits_the_copied_set() {
    let source = Probe::new(&src_ns(), numbered(20), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let report = run(CopyJob {
        filter: doc! { "even": true },
        ..job(&source, &target, ConflictPolicy::Skip, 4)
    })
    .await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(report.progress.scanned, 10);
    let copied = target.contents(&dst_ns()).await;
    assert_eq!(copied.len(), 10);
    assert!(copied.iter().all(|d| d.get_bool("even").unwrap()));
}

#[tokio::test]
async fn skip_overwrite_upsert_end_states_match_policy() {
    let source_docs = vec![
        doc! { "_id": 1, "v": "new1" },
        doc! { "_id": 2, "v": "new2" },
    ];
    let target_docs = vec![
        doc! { "_id": 2, "v": "old2" },
        doc! { "_id": 3, "v": "old3" },
    ];
    let cases = [
        (
            ConflictPolicy::Skip,
            vec![
                doc! { "_id": 2, "v": "old2" },
                doc! { "_id": 3, "v": "old3" },
                doc! { "_id": 1, "v": "new1" },
            ],
            (1, 1),
        ),
        (
            ConflictPolicy::Overwrite,
            vec![
                doc! { "_id": 2, "v": "new2" },
                doc! { "_id": 3, "v": "old3" },
            ],
            (1, 1),
        ),
        (
            ConflictPolicy::Upsert,
            vec![
                doc! { "_id": 2, "v": "new2" },
                doc! { "_id": 3, "v": "old3" },
                doc! { "_id": 1, "v": "new1" },
            ],
            (2, 0),
        ),
    ];
    for (policy, want, (written, skipped)) in cases {
        let source = Probe::new(&src_ns(), source_docs.clone(), Fail::Never);
        let target = Probe::new(&dst_ns(), target_docs.clone(), Fail::Never);
        let report = run(job(&source, &target, policy, 500)).await;
        assert_eq!(report.outcome, CopyOutcome::Completed, "{policy:?}");
        assert_eq!(
            (report.progress.written, report.progress.skipped),
            (written, skipped),
            "{policy:?}"
        );
        assert_eq!(target.contents(&dst_ns()).await, want, "{policy:?}");
    }
}

#[tokio::test]
async fn cancel_between_batches_reports_exact_written_count_equal_to_target_len() {
    let source = Probe::new(&src_ns(), numbered(10), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let cancel = CancelFlag::default();
    let report = copy_collection(
        job(&source, &target, ConflictPolicy::Skip, 3),
        &cancel,
        &mut |_| cancel.cancel(),
    )
    .await;
    assert_eq!(report.outcome, CopyOutcome::Cancelled);
    assert_eq!(report.progress.written, 3);
    assert_eq!(
        report.progress.written,
        target.contents(&dst_ns()).await.len() as u64
    );
}

#[tokio::test]
async fn a_cancel_before_the_start_writes_nothing() {
    let source = Probe::new(&src_ns(), numbered(10), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let cancel = CancelFlag::default();
    cancel.cancel();
    let report = copy_collection(
        job(&source, &target, ConflictPolicy::Skip, 3),
        &cancel,
        &mut |_| {},
    )
    .await;
    assert_eq!(report.outcome, CopyOutcome::Cancelled);
    assert_eq!(report.progress, CopyProgress::default());
    assert!(target.contents(&dst_ns()).await.is_empty());
}

#[tokio::test]
async fn a_failing_batch_reports_written_so_far_and_stops() {
    let source = Probe::new(&src_ns(), numbered(10), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::CallNumber(2));
    let report = run(job(&source, &target, ConflictPolicy::Skip, 3)).await;
    assert_eq!(
        report.outcome,
        CopyOutcome::Failed(CoreError::ApplyOutcomeUnknown)
    );
    assert_eq!(report.progress.written, 3);
    assert_eq!(target.contents(&dst_ns()).await.len(), 3);
    assert_eq!(target.writes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn per_document_failures_are_bounded_and_counted() {
    let source = Probe::new(&src_ns(), numbered(150), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::EveryOp);
    let report = run(job(&source, &target, ConflictPolicy::Skip, 500)).await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(report.progress.failed, 150);
    assert_eq!(report.progress.written, 0);
    assert_eq!(report.failures.len(), MAX_REPORTED_FAILURES);
    assert_eq!(report.failures[0].reason, WriteFailure::Rejected(121));
    assert_eq!(report.failures[0].key, Bson::Int32(0));
}

#[tokio::test]
async fn documents_with_dollar_and_dotted_field_names_are_copied_unchanged() {
    let docs = vec![
        doc! { "_id": 1, "$set": 1, "a.b": 2, "nested": { "$x": 3, "c.d": 4 } },
        doc! { "_id": 2, "$ref": "x", "e.f.g": [1, 2] },
    ];
    let source = Probe::new(&src_ns(), docs.clone(), Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let report = run(job(&source, &target, ConflictPolicy::Skip, 1)).await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(target.contents(&dst_ns()).await, docs);
}

#[tokio::test]
async fn same_namespace_in_same_store_is_refused() {
    let store = Probe::new(&src_ns(), numbered(5), Fail::Never);
    let report = run(CopyJob {
        target_ns: src_ns(),
        ..job(&store, &store, ConflictPolicy::Upsert, 2)
    })
    .await;
    assert!(matches!(
        report.outcome,
        CopyOutcome::Failed(CoreError::Unsupported(_))
    ));
    assert_eq!(report.progress, CopyProgress::default());
    assert_eq!(store.scans.load(Ordering::SeqCst), 0);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn same_namespace_in_another_store_is_allowed() {
    let source = Probe::new(&src_ns(), numbered(5), Fail::Never);
    let target = Probe::new(&src_ns(), vec![], Fail::Never);
    let report = run(CopyJob {
        target_ns: src_ns(),
        ..job(&source, &target, ConflictPolicy::Skip, 2)
    })
    .await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(report.progress.written, 5);
}

#[tokio::test]
async fn an_empty_source_completes_with_nothing_written() {
    let source = Probe::new(&src_ns(), vec![], Fail::Never);
    let target = Probe::new(&dst_ns(), vec![], Fail::Never);
    let report = run(job(&source, &target, ConflictPolicy::Skip, 3)).await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(report.progress, CopyProgress::default());
}
