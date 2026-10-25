//! Tests for options-first, indexes-last copying, against in-memory stores.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use super::*;
use crate::compare::engine::CancelFlag;
use crate::compare::key::{KeySpec, KeyTuple};
use crate::copy::engine::{CopyOutcome, copy_collection};
use crate::copy::policy::ConflictPolicy;
use crate::error::{CoreError, Result};
use crate::indexes::{IndexSpec, KeyKind};
use crate::store::{
    CappedSpec, CollectionIndex, CollectionKind, CollectionOptions, DocumentStore, Namespace, Page,
    Query, WriteOp, WriteOutcome,
};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_index_spec_named;

const SECRET: &str = "SECRET-VALUE";

fn src_ns() -> Namespace {
    Namespace::new("db", "src")
}

fn dst_ns() -> Namespace {
    Namespace::new("db", "dst")
}

fn capped() -> CollectionOptions {
    CollectionOptions {
        capped: Some(CappedSpec {
            size: 4096,
            max: Some(10),
        }),
        validator: Some(doc! { "$jsonSchema": { "required": ["a"] } }),
        ..CollectionOptions::default()
    }
}

fn index(name: &str, field: &str) -> IndexSpec {
    mock_index_spec_named(name, &[(field, KeyKind::Asc)])
}

/// An in-memory store that logs the calls that matter to the order and can misbehave.
struct Spy {
    inner: InMemoryStore,
    log: Mutex<Vec<String>>,
    /// `create_index` of this name fails the way a unique build over duplicates does.
    failing_index: Option<&'static str>,
    fail_writes: bool,
    kind: CollectionKind,
}

impl Spy {
    fn new() -> Self {
        Self {
            inner: InMemoryStore::new(),
            log: Mutex::new(Vec::new()),
            failing_index: None,
            fail_writes: false,
            kind: CollectionKind::Collection,
        }
    }

    fn note(&self, entry: impl Into<String>) {
        self.log.lock().unwrap().push(entry.into());
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn at(&self, entry: &str) -> Option<usize> {
        self.log().iter().position(|e| e == entry)
    }

    async fn contents(&self, ns: &Namespace) -> Vec<Document> {
        let query = Query {
            limit: 0,
            ..Query::default()
        };
        self.inner.find(ns, &query).await.unwrap().docs
    }

    /// A source collection created with `options`, holding three documents and `indexes`.
    async fn source(options: &CollectionOptions, indexes: &[IndexSpec]) -> Arc<Self> {
        let spy = Self::new();
        spy.inner
            .create_collection(&src_ns(), options)
            .await
            .unwrap();
        for n in 0..3 {
            spy.inner
                .insert(&src_ns(), doc! { "_id": n, "a": SECRET })
                .await
                .unwrap();
        }
        for spec in indexes {
            spy.inner.create_index(&src_ns(), spec).await.unwrap();
        }
        Arc::new(spy)
    }
}

#[async_trait]
impl DocumentStore for Spy {
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
        self.note("write");
        if self.fail_writes {
            return Err(CoreError::ApplyOutcomeUnknown);
        }
        self.inner.bulk_write(ns, ops).await
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }

    async fn create_collection(&self, ns: &Namespace, options: &CollectionOptions) -> Result<()> {
        self.note("create_collection");
        self.inner.create_collection(ns, options).await
    }

    async fn collection_options(&self, ns: &Namespace) -> Result<Option<CollectionOptions>> {
        self.inner.collection_options(ns).await
    }

    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        let listed = self.inner.list_collection_kinds(db).await?;
        Ok(listed
            .into_iter()
            .map(|(name, _)| (name, self.kind))
            .collect())
    }

    async fn list_indexes(&self, ns: &Namespace) -> Result<Vec<CollectionIndex>> {
        self.inner.list_indexes(ns).await
    }

    async fn create_index(&self, ns: &Namespace, spec: &IndexSpec) -> Result<()> {
        self.note(format!("index:{}", spec.name));
        if self.failing_index == Some(spec.name.as_str()) {
            return Err(CoreError::Database(format!(
                "Index build failed: E11000 duplicate key error dup key: {{ a: \"{SECRET}\" }}"
            )));
        }
        self.inner.create_index(ns, spec).await
    }
}

fn job(source: &Arc<Spy>, target: &Arc<Spy>) -> CopyJob {
    CopyJob {
        source: source.clone(),
        source_ns: src_ns(),
        filter: Document::new(),
        target: target.clone(),
        target_ns: dst_ns(),
        policy: ConflictPolicy::Skip,
        batch: 2,
    }
}

async fn run(source: &Arc<Spy>, target: &Arc<Spy>, with_structure: bool) -> CopyReport {
    copy_with_structure(
        job(source, target),
        with_structure,
        &CancelFlag::default(),
        &mut |_| {},
    )
    .await
}

fn structure(outcome: StructureOutcome) -> Option<StructureReport> {
    Some(StructureReport { outcome })
}

#[tokio::test]
async fn new_target_gets_options_before_the_data() {
    let source = Spy::source(&capped(), &[]).await;
    let target = Arc::new(Spy::new());
    let report = run(&source, &target, true).await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(report.progress.written, 3);
    assert_eq!(report.structure, structure(StructureOutcome::Created));
    // The in-memory store refuses to create a collection that already holds data, so the options
    // can only be there if they came first; the log states the order as well.
    assert!(target.at("create_collection").unwrap() < target.at("write").unwrap());
    assert_eq!(
        target.collection_options(&dst_ns()).await.unwrap(),
        Some(capped())
    );
}

#[tokio::test]
async fn existing_target_keeps_its_options() {
    let source = Spy::source(&capped(), &[]).await;
    let target = Arc::new(Spy::new());
    let own = CollectionOptions {
        validation_level: Some("strict".into()),
        ..CollectionOptions::default()
    };
    target
        .inner
        .create_collection(&dst_ns(), &own)
        .await
        .unwrap();
    let report = run(&source, &target, true).await;
    assert_eq!(report.progress.written, 3);
    assert_eq!(report.structure, structure(StructureOutcome::ExistingKept));
    assert_eq!(target.at("create_collection"), None);
    assert_eq!(
        target.collection_options(&dst_ns()).await.unwrap(),
        Some(own)
    );
}

#[tokio::test]
async fn indexes_are_built_after_the_data() {
    let source = Spy::source(&capped(), &[index("a_1", "a"), index("b_1", "b")]).await;
    let target = Arc::new(Spy::new());
    let report = run(&source, &target, true).await;
    let indexes = report.indexes.expect("indexes were copied");
    assert_eq!(indexes.built, ["a_1", "b_1"]);
    let log = target.log();
    let last_write = log.iter().rposition(|e| e == "write").unwrap();
    let first_index = target.at("index:a_1").unwrap();
    assert!(last_write < first_index, "{log:?}");
}

#[tokio::test]
async fn id_index_is_not_copied() {
    let source = Spy::source(&capped(), &[index("a_1", "a")]).await;
    let target = Arc::new(Spy::new());
    let report = run(&source, &target, true).await;
    assert_eq!(target.at("index:_id_"), None);
    let indexes = report.indexes.unwrap();
    let named = indexes.built.iter().chain(&indexes.existing);
    assert!(!named.into_iter().any(|n| n == "_id_"));
}

#[tokio::test]
async fn existing_index_with_the_same_name_is_left_alone() {
    let source = Spy::source(&capped(), &[index("a_1", "a"), index("b_1", "b")]).await;
    let target = Arc::new(Spy::new());
    target
        .inner
        .create_index(&dst_ns(), &index("a_1", "other"))
        .await
        .unwrap();
    let report = run(&source, &target, true).await;
    let indexes = report.indexes.unwrap();
    assert_eq!(indexes.existing, ["a_1"]);
    assert_eq!(indexes.built, ["b_1"]);
    assert_eq!(target.at("index:a_1"), None);
    let kept = target.inner.list_index_specs(&dst_ns()).await.unwrap();
    let a = kept.iter().find(|s| s.name == "a_1").unwrap();
    assert_eq!(a.keys[0].field, "other");
}

#[tokio::test]
async fn failed_unique_index_is_reported_and_data_stays() {
    let source = Spy::source(&capped(), &[index("u_1", "a"), index("b_1", "b")]).await;
    let mut target = Spy::new();
    target.failing_index = Some("u_1");
    let target = Arc::new(target);
    let report = run(&source, &target, true).await;
    assert_eq!(report.outcome, CopyOutcome::Completed);
    let indexes = report.indexes.clone().unwrap();
    assert_eq!(
        indexes.failed,
        [IndexFailure {
            name: "u_1".into(),
            reason: IndexFailReason::DuplicateData
        }]
    );
    assert_eq!(indexes.built, ["b_1"]);
    assert_eq!(target.contents(&dst_ns()).await.len(), 3);
    assert!(!format!("{report:?}").contains(SECRET));
}

#[tokio::test]
async fn unchecked_option_copies_data_only() {
    let source = Spy::source(&capped(), &[index("a_1", "a")]).await;
    let target = Arc::new(Spy::new());
    let report = run(&source, &target, false).await;
    assert_eq!(report.progress.written, 3);
    assert_eq!(report.structure, None);
    assert_eq!(report.indexes, None);
    assert!(target.log().iter().all(|e| e == "write"));
    assert_eq!(
        target.collection_options(&dst_ns()).await.unwrap(),
        Some(CollectionOptions::default())
    );
    // Exactly what the plain engine does.
    let plain = Arc::new(Spy::new());
    let baseline = copy_collection(job(&source, &plain), &CancelFlag::default(), &mut |_| {}).await;
    assert_eq!(report, baseline);
}

#[tokio::test]
async fn failed_data_copy_builds_no_indexes() {
    let source = Spy::source(&capped(), &[index("a_1", "a")]).await;
    let mut target = Spy::new();
    target.fail_writes = true;
    let target = Arc::new(target);
    let report = run(&source, &target, true).await;
    assert!(matches!(report.outcome, CopyOutcome::Failed(_)));
    assert_eq!(report.indexes, None);
    assert_eq!(target.at("index:a_1"), None);
}

#[tokio::test]
async fn cancelled_copy_builds_no_indexes() {
    let source = Spy::source(&capped(), &[index("a_1", "a")]).await;
    let target = Arc::new(Spy::new());
    let cancel = CancelFlag::default();
    cancel.cancel();
    let report = copy_with_structure(job(&source, &target), true, &cancel, &mut |_| {}).await;
    assert_eq!(report.outcome, CopyOutcome::Cancelled);
    assert_eq!(report.indexes, None);
    assert!(target.log().is_empty());
}

#[tokio::test]
async fn a_view_source_is_copied_data_only() {
    let mut source = Spy::new();
    source.kind = CollectionKind::View;
    source.inner = InMemoryStore::new().with_docs(&src_ns(), vec![doc! { "_id": 1 }]);
    let source = Arc::new(source);
    let target = Arc::new(Spy::new());
    let report = run(&source, &target, true).await;
    assert_eq!(report.progress.written, 1);
    assert_eq!(
        report.structure,
        structure(StructureOutcome::SourceNotPlain)
    );
    assert_eq!(report.indexes, None);
    assert_eq!(target.at("create_collection"), None);
}

#[test]
fn index_failures_are_classified_without_keeping_text() {
    use super::classify_index_error as classify;
    let dup = CoreError::Database("E11000 duplicate key error".into());
    assert_eq!(classify(&dup), IndexFailReason::DuplicateData);
    assert_eq!(
        classify(&CoreError::Unauthorized),
        IndexFailReason::NotAllowed
    );
    assert_eq!(
        classify(&CoreError::Unreachable),
        IndexFailReason::ConnectionLost
    );
    assert_eq!(
        classify(&CoreError::Unsupported("x".into())),
        IndexFailReason::Unsupported
    );
    assert_eq!(
        classify(&CoreError::Database("anything".into())),
        IndexFailReason::Rejected
    );
}
