//! Tests for the apply engine (kept apart to keep both files small).

use super::*;
use crate::compare::apply::{ApplyKind, Direction};
use crate::compare::engine::{EngineOutcome, PairInput, run_pair};
use crate::compare::key::KeySpec;
use crate::compare::results::{Budget, Limits};
use crate::compare::rules::FieldScope;
use crate::store::{Page, Query, WriteFailure};
use crate::test_support::InMemoryStore;
use async_trait::async_trait;
use bson::{Bson, doc};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ApplyKind::{Delete, Insert, Replace};
use Direction::{SourceToTarget as ST, TargetToSource as TS};

pub(super) const PAIR: u32 = 1;

pub(super) fn src_ns() -> Namespace {
    Namespace::new("db", "a")
}

pub(super) fn tgt_ns() -> Namespace {
    Namespace::new("db", "b")
}

pub(super) fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

pub(super) fn email(s: &str) -> KeyTuple {
    KeyTuple(vec![Bson::String(s.into())])
}

pub(super) fn op(key: KeyTuple, kind: ApplyKind, direction: Direction) -> ApplyOp {
    ApplyOp {
        pair: PAIR,
        key,
        kind,
        direction,
        fields: Vec::new(),
    }
}

/// Wraps a target store: records the size of every `bulk_write` call and can inject
/// failures.
pub(super) struct Spy {
    pub(super) inner: Arc<InMemoryStore>,
    pub(super) calls: Mutex<Vec<usize>>,
    pub(super) duplicate_id: Mutex<Option<Bson>>,
    pub(super) fail_call: Mutex<Option<usize>>,
    pub(super) short_reply: AtomicBool,
    /// How many `scan_sorted` / `scan_sorted_primary` calls reached this store.
    pub(super) plain_scans: AtomicUsize,
    pub(super) primary_scans: AtomicUsize,
}

impl Spy {
    fn new(inner: Arc<InMemoryStore>) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
            duplicate_id: Mutex::new(None),
            fail_call: Mutex::new(None),
            short_reply: AtomicBool::new(false),
            plain_scans: AtomicUsize::new(0),
            primary_scans: AtomicUsize::new(0),
        }
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
    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.plain_scans.fetch_add(1, Ordering::SeqCst);
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
    async fn scan_sorted_primary(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.primary_scans.fetch_add(1, Ordering::SeqCst);
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(ops.len());
            calls.len() - 1
        };
        if *self.fail_call.lock().unwrap() == Some(call) {
            return Err(CoreError::Database("boom".into()));
        }
        let duplicate = self.duplicate_id.lock().unwrap().clone();
        let is_dup = |o: &WriteOp| matches!(o, WriteOp::Insert { doc } if duplicate.is_some() && doc.get("_id") == duplicate.as_ref());
        let flags: Vec<bool> = ops.iter().map(is_dup).collect();
        let passed: Vec<WriteOp> = ops
            .into_iter()
            .zip(&flags)
            .filter(|(_, d)| !**d)
            .map(|(o, _)| o)
            .collect();
        let mut real = self.inner.bulk_write(ns, passed).await?.into_iter();
        if self.short_reply.load(Ordering::SeqCst) {
            real.next();
            return Ok(Vec::new());
        }
        Ok(flags
            .iter()
            .map(|d| {
                if *d {
                    Err(WriteFailure::DuplicateKey)
                } else {
                    real.next().expect("one outcome per op")
                }
            })
            .collect())
    }
}

pub(super) struct Fixture {
    pub(super) spec: PairSpec,
    pub(super) results: ResultSet,
    pub(super) source_inner: Arc<InMemoryStore>,
    pub(super) source: Arc<Spy>,
    pub(super) target_inner: Arc<InMemoryStore>,
    pub(super) target: Arc<Spy>,
}

impl Fixture {
    pub(super) async fn build(
        source: Vec<Document>,
        target: Vec<Document>,
        key: KeySpec,
        rules: CompareRules,
        filter: Document,
    ) -> Self {
        let source_inner = Arc::new(InMemoryStore::new().with_docs(&src_ns(), source));
        let source = Arc::new(Spy::new(source_inner.clone()));
        let target_inner = Arc::new(InMemoryStore::new().with_docs(&tgt_ns(), target));
        let target = Arc::new(Spy::new(target_inner.clone()));
        let spec = PairSpec {
            id: PAIR,
            source: src_ns(),
            target: tgt_ns(),
            source_filter: filter.clone(),
            target_filter: filter,
            key,
            rules,
        };
        let results = compare(&spec, source.clone(), target.clone()).await;
        Self {
            spec,
            results,
            source_inner,
            source,
            target_inner,
            target,
        }
    }

    pub(super) async fn by_id(source: Vec<Document>, target: Vec<Document>) -> Self {
        let key = KeySpec::id();
        let rules = CompareRules::for_key(&key);
        Self::build(source, target, key, rules, doc! {}).await
    }

    pub(super) fn input<'a>(&'a self, ops: &'a [ApplyOp]) -> ApplyInput<'a> {
        ApplyInput {
            spec: &self.spec,
            results: &self.results,
            source: self.source.clone(),
            target: self.target.clone(),
            ops,
            policy: IdPolicy::Keep,
        }
    }

    pub(super) async fn apply(&self, ops: &[ApplyOp]) -> ApplyReport {
        apply_pair(self.input(ops), &CancelFlag::default(), &mut |_| {})
            .await
            .unwrap()
    }

    pub(super) async fn recompare(&self) -> ResultSet {
        compare(&self.spec, self.source.clone(), self.target.clone()).await
    }

    pub(super) fn source_docs(&self) -> Vec<Document> {
        all(&self.source_inner, &src_ns())
    }

    pub(super) fn target_docs(&self) -> Vec<Document> {
        all(&self.target_inner, &tgt_ns())
    }
}

pub(super) async fn compare(
    spec: &PairSpec,
    source: Arc<dyn DocumentStore>,
    target: Arc<dyn DocumentStore>,
) -> ResultSet {
    let input = PairInput {
        source,
        source_ns: spec.source.clone(),
        source_filter: spec.source_filter.clone(),
        target,
        target_ns: spec.target.clone(),
        target_filter: spec.target_filter.clone(),
        key: spec.key.clone(),
        rules: spec.rules.clone(),
        budget: Budget::new(Limits::default()),
    };
    let (outcome, _, set) = run_pair(input, &CancelFlag::default(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(outcome, EngineOutcome::Completed);
    set
}

pub(super) fn all(store: &InMemoryStore, ns: &Namespace) -> Vec<Document> {
    let q = Query {
        limit: 5000,
        ..Query::default()
    };
    let mut docs = block_on(store.find(ns, &q)).docs;
    docs.sort_by_key(|d| d.get_i32("_id").unwrap_or_default());
    docs
}

pub(super) fn block_on<T>(f: impl std::future::Future<Output = Result<T>>) -> T {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f)).unwrap()
}

pub(super) fn total(r: &ApplyReport) -> u64 {
    r.applied + r.stale + r.not_planned + r.failed + r.not_run
}

#[tokio::test(flavor = "multi_thread")]
async fn copies_a_source_only_document_to_the_target() {
    let f = Fixture::by_id(vec![doc! {"_id": 1, "v": 1}], vec![]).await;
    let r = f.apply(&[op(k(1), Insert, ST)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1, "v": 1}]);
    assert_eq!(f.source_docs(), vec![doc! {"_id": 1, "v": 1}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_target_only_document() {
    let f = Fixture::by_id(vec![], vec![doc! {"_id": 4, "v": 1}]).await;
    let r = f.apply(&[op(k(4), Delete, ST)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert!(f.target_docs().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn replaces_a_changed_document() {
    let f = Fixture::by_id(vec![doc! {"_id": 3, "v": 1}], vec![doc! {"_id": 3, "v": 2}]).await;
    let r = f.apply(&[op(k(3), Replace, ST)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 3, "v": 1}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn copies_a_target_only_document_back_to_the_source() {
    let f = Fixture::by_id(vec![], vec![doc! {"_id": 4, "v": 1}]).await;
    let r = f.apply(&[op(k(4), Insert, TS)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(f.source_docs(), vec![doc! {"_id": 4, "v": 1}]);
    assert_eq!(f.target_docs(), vec![doc! {"_id": 4, "v": 1}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_source_only_document() {
    let f = Fixture::by_id(vec![doc! {"_id": 1, "v": 1}], vec![]).await;
    let r = f.apply(&[op(k(1), Delete, TS)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert!(f.source_docs().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn replace_with_a_business_key_keeps_the_destination_id() {
    let key = KeySpec::new(vec!["email".into()]).unwrap();
    let rules = CompareRules::for_key(&key);
    let f = Fixture::build(
        vec![doc! {"_id": 1, "email": "a", "v": 2}],
        vec![doc! {"_id": 50, "email": "a", "v": 1}],
        key,
        rules,
        doc! {},
    )
    .await;
    let r = f.apply(&[op(email("a"), Replace, ST)]).await;
    assert_eq!((r.applied, total(&r)), (1, 1));
    assert_eq!(
        f.target_docs(),
        vec![doc! {"_id": 50, "email": "a", "v": 2}]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_changed_on_the_destination_since_the_compare_is_skipped_stale() {
    let f = Fixture::by_id(vec![doc! {"_id": 3, "v": 1}], vec![doc! {"_id": 3, "v": 2}]).await;
    let changed = doc! {"_id": 3, "v": 99};
    f.target_inner
        .replace(&tgt_ns(), &Bson::Int32(3), changed.clone())
        .await
        .unwrap();
    let r = f.apply(&[op(k(3), Replace, ST)]).await;
    assert_eq!((r.stale, r.applied, total(&r)), (1, 0, 1));
    assert_eq!(f.target_docs(), vec![changed]);
    assert_eq!(*f.target.calls.lock().unwrap(), Vec::<usize>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_changed_on_the_origin_since_the_compare_is_skipped_stale() {
    let f = Fixture::by_id(vec![doc! {"_id": 3, "v": 1}], vec![doc! {"_id": 3, "v": 2}]).await;
    f.source
        .replace(&src_ns(), &Bson::Int32(3), doc! {"_id": 3, "v": 7})
        .await
        .unwrap();
    let r = f.apply(&[op(k(3), Replace, ST)]).await;
    assert_eq!((r.stale, r.applied, total(&r)), (1, 0, 1));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 3, "v": 2}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_insert_is_stale_when_the_destination_gained_the_document() {
    let f = Fixture::by_id(vec![doc! {"_id": 1, "v": 1}], vec![]).await;
    let appeared = doc! {"_id": 1, "v": 5};
    f.target_inner
        .insert(&tgt_ns(), appeared.clone())
        .await
        .unwrap();
    let r = f.apply(&[op(k(1), Insert, ST)]).await;
    assert_eq!((r.stale, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![appeared]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_op_whose_kind_does_not_fit_the_row_is_skipped_not_planned_and_nothing_is_written() {
    let f = Fixture::by_id(vec![doc! {"_id": 3, "v": 1}], vec![doc! {"_id": 3, "v": 2}]).await;
    let ops = [
        op(k(3), Insert, ST),
        op(k(3), Delete, TS),
        op(k(3), Delete, ST),
    ];
    let r = f.apply(&ops).await;
    assert_eq!((r.not_planned, total(&r)), (3, 3));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 3, "v": 2}]);
    assert_eq!(f.source_docs(), vec![doc! {"_id": 3, "v": 1}]);
    assert_eq!(*f.target.calls.lock().unwrap(), Vec::<usize>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_op_for_a_key_that_is_not_in_the_results_fails_and_writes_nothing() {
    let f = Fixture::by_id(
        vec![doc! {"_id": 1}, doc! {"_id": 2}],
        vec![doc! {"_id": 2}],
    )
    .await;
    // 2 is equal on both sides (not in the results); 99 exists nowhere.
    let r = f
        .apply(&[op(k(2), Delete, ST), op(k(99), Insert, ST)])
        .await;
    assert_eq!((r.failed, total(&r)), (2, 2));
    assert!(r.failures.iter().all(|x| x.error == CoreError::NotFound));
    assert_eq!(f.target_docs(), vec![doc! {"_id": 2}]);
    assert_eq!(*f.target.calls.lock().unwrap(), Vec::<usize>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_op_for_another_pair_is_refused() {
    let f = Fixture::by_id(vec![doc! {"_id": 1}], vec![]).await;
    let mut foreign = op(k(1), Insert, ST);
    foreign.pair = PAIR + 1;
    let r = f.apply(&[foreign]).await;
    assert_eq!((r.failed, r.applied), (1, 0));
    assert!(f.target_docs().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn one_duplicate_key_failure_does_not_stop_its_neighbours() {
    let source = (1..=3).map(|n| doc! {"_id": n}).collect();
    let f = Fixture::by_id(source, vec![]).await;
    *f.target.duplicate_id.lock().unwrap() = Some(Bson::Int32(2));
    let ops: Vec<_> = (1..=3).map(|n| op(k(n), Insert, ST)).collect();
    let r = f.apply(&ops).await;
    assert_eq!((r.applied, r.failed, total(&r)), (2, 1, 3));
    assert_eq!(r.failures.len(), 1);
    assert_eq!(r.failures[0].key, k(2));
    assert_eq!(r.failures[0].error, CoreError::ApplyDuplicateKey);
    assert_eq!(f.target_docs(), vec![doc! {"_id": 1}, doc! {"_id": 3}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn batches_of_500() {
    let source = (0..1201).map(|n| doc! {"_id": n}).collect();
    let f = Fixture::by_id(source, vec![]).await;
    let ops: Vec<_> = (0..1201).map(|n| op(k(n), Insert, ST)).collect();
    let mut seen = Vec::new();
    let r = apply_pair(f.input(&ops), &CancelFlag::default(), &mut |d| seen.push(d))
        .await
        .unwrap();
    assert_eq!((r.applied, total(&r)), (1201, 1201));
    assert_eq!(*f.target.calls.lock().unwrap(), vec![500, 500, 201]);
    assert_eq!(seen, vec![500, 1000, 1201]);
    assert_eq!(f.target_docs().len(), 1201);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_between_batches_reports_not_run() {
    let source = (0..1201).map(|n| doc! {"_id": n}).collect();
    let f = Fixture::by_id(source, vec![]).await;
    let ops: Vec<_> = (0..1201).map(|n| op(k(n), Insert, ST)).collect();
    let cancel = CancelFlag::default();
    let c = cancel.clone();
    let r = apply_pair(f.input(&ops), &cancel, &mut |_| c.cancel())
        .await
        .unwrap();
    assert_eq!((r.applied, r.not_run, total(&r)), (500, 701, 1201));
    assert_eq!(f.target_docs().len(), 500);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_the_first_batch_runs_nothing() {
    let f = Fixture::by_id(vec![doc! {"_id": 1}], vec![]).await;
    let cancel = CancelFlag::default();
    cancel.cancel();
    let ops = [op(k(1), Insert, ST)];
    let r = apply_pair(f.input(&ops), &cancel, &mut |_| {})
        .await
        .unwrap();
    assert_eq!((r.applied, r.not_run), (0, 1));
    assert!(f.target_docs().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_op_list_writes_nothing_and_reports_zeroes() {
    let f = Fixture::by_id(vec![doc! {"_id": 1}], vec![]).await;
    let mut calls = 0;
    let r = apply_pair(f.input(&[]), &CancelFlag::default(), &mut |_| calls += 1)
        .await
        .unwrap();
    assert_eq!(r, ApplyReport::default());
    assert_eq!(calls, 0);
    assert!(f.target.calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_whole_call_error_marks_that_group_failed_and_the_next_batch_still_runs() {
    let source = (0..1001).map(|n| doc! {"_id": n}).collect();
    let f = Fixture::by_id(source, vec![]).await;
    *f.target.fail_call.lock().unwrap() = Some(0);
    let ops: Vec<_> = (0..1001).map(|n| op(k(n), Insert, ST)).collect();
    let r = f.apply(&ops).await;
    assert_eq!((r.failed, r.applied, total(&r)), (500, 501, 1001));
    assert_eq!(r.failures.len(), 100);
    assert!(r.more_failures);
    assert_eq!(r.failures[0].error, CoreError::ApplyOutcomeUnknown);
    assert!(!format!("{:?}", r.failures[0]).contains("boom"));
    assert_eq!(*f.target.calls.lock().unwrap(), vec![500, 500, 1]);
    assert_eq!(f.target_docs().len(), 501);
}

#[tokio::test(flavor = "multi_thread")]
async fn applying_then_comparing_again_finds_no_difference_for_an_id_key() {
    let f = Fixture::by_id(
        vec![
            doc! {"_id": 1, "v": 1},
            doc! {"_id": 2, "v": 2},
            doc! {"_id": 3, "v": 3},
        ],
        vec![
            doc! {"_id": 2, "v": 20},
            doc! {"_id": 3, "v": 3},
            doc! {"_id": 4, "v": 4},
        ],
    )
    .await;
    let ops = [
        op(k(1), Insert, ST),
        op(k(2), Replace, ST),
        op(k(4), Delete, ST),
    ];
    let r = f.apply(&ops).await;
    assert_eq!((r.applied, total(&r)), (3, 3));
    assert_no_differences(&f.recompare().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn applying_then_comparing_again_finds_no_difference_for_a_business_key() {
    let key = KeySpec::new(vec!["email".into()]).unwrap();
    let rules = CompareRules::for_key(&key);
    assert!(!rules.include_id);
    let f = Fixture::build(
        vec![
            doc! {"_id": 1, "email": "a", "v": 1},
            doc! {"_id": 2, "email": "b", "v": 2},
        ],
        vec![
            doc! {"_id": 90, "email": "b", "v": 20},
            doc! {"_id": 91, "email": "c", "v": 3},
        ],
        key,
        rules,
        doc! {},
    )
    .await;
    let ops = [
        op(email("a"), Insert, ST),
        op(email("b"), Replace, ST),
        op(email("c"), Delete, ST),
    ];
    let r = f.apply(&ops).await;
    assert_eq!((r.applied, total(&r)), (3, 3));
    assert_no_differences(&f.recompare().await);
}

pub(super) fn assert_no_differences(set: &ResultSet) {
    for c in CATEGORIES {
        assert_eq!(set.count(c), 0, "{c:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fingerprint_check_uses_the_pairs_filter() {
    // `region` is excluded from the comparison, so moving the target document out of the
    // filter does not change its fingerprint: only the filter can reveal the change.
    let key = KeySpec::id();
    let mut rules = CompareRules::for_key(&key);
    rules.field_scope = FieldScope::AllExcept(vec!["region".into()]);
    let f = Fixture::build(
        vec![doc! {"_id": 1, "region": "eu", "v": 1}],
        vec![doc! {"_id": 1, "region": "eu", "v": 2}],
        key,
        rules,
        doc! {"region": "eu"},
    )
    .await;
    let moved = doc! {"_id": 1, "region": "us", "v": 2};
    f.target_inner
        .replace(&tgt_ns(), &Bson::Int32(1), moved.clone())
        .await
        .unwrap();
    let r = f.apply(&[op(k(1), Replace, ST)]).await;
    assert_eq!((r.stale, r.applied, total(&r)), (1, 0, 1));
    assert_eq!(f.target_docs(), vec![moved]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_generate_policy_inserts_without_the_origin_id() {
    let f = Fixture::by_id(vec![doc! {"_id": 1, "v": 1}], vec![]).await;
    let ops = [op(k(1), Insert, ST)];
    let mut input = f.input(&ops);
    input.policy = IdPolicy::Generate;
    let r = apply_pair(input, &CancelFlag::default(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(r.applied, 1);
    let docs = f.target_docs();
    assert_eq!(docs.len(), 1);
    assert_ne!(docs[0].get("_id"), Some(&Bson::Int32(1)));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_ops_on_one_key_run_only_the_first() {
    let f = Fixture::by_id(vec![doc! {"_id": 3, "v": 1}], vec![doc! {"_id": 3, "v": 2}]).await;
    let r = f
        .apply(&[op(k(3), Replace, ST), op(k(3), Replace, TS)])
        .await;
    assert_eq!((r.applied, r.not_planned, total(&r)), (1, 1, 2));
    assert_eq!(*f.target.calls.lock().unwrap(), vec![1]);
    assert!(f.source.calls.lock().unwrap().is_empty());
    assert_eq!(f.source_docs(), vec![doc! {"_id": 3, "v": 1}]);
    assert_eq!(f.target_docs(), vec![doc! {"_id": 3, "v": 1}]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_insert_is_written_once() {
    let f = Fixture::by_id(vec![doc! {"_id": 1}], vec![]).await;
    let r = f.apply(&[op(k(1), Insert, ST), op(k(1), Insert, ST)]).await;
    assert_eq!((r.applied, r.not_planned, r.failed), (1, 1, 0));
    assert_eq!(*f.target.calls.lock().unwrap(), vec![1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_that_differ_only_in_numeric_type_count_as_the_same_key() {
    let f = Fixture::by_id(vec![doc! {"_id": 1}], vec![]).await;
    let wide = KeyTuple(vec![Bson::Int64(1)]);
    let r = f.apply(&[op(k(1), Insert, ST), op(wide, Insert, ST)]).await;
    assert_eq!((r.applied, r.not_planned, r.failed), (1, 1, 0));
    assert_eq!(f.target_docs().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_to_both_sides_in_one_batch_keep_each_outcome_with_its_op() {
    let f = Fixture::by_id(
        vec![doc! {"_id": 1}, doc! {"_id": 3, "v": 1}],
        vec![doc! {"_id": 3, "v": 2}, doc! {"_id": 4}],
    )
    .await;
    *f.target.duplicate_id.lock().unwrap() = Some(Bson::Int32(1));
    let ops = [
        op(k(4), Insert, TS),
        op(k(1), Insert, ST),
        op(k(3), Replace, ST),
    ];
    let r = f.apply(&ops).await;
    assert_eq!((r.applied, r.failed, total(&r)), (2, 1, 3));
    assert_eq!(r.failures[0].key, k(1));
    assert_eq!(*f.source.calls.lock().unwrap(), vec![1]);
    assert_eq!(*f.target.calls.lock().unwrap(), vec![2]);
    assert_eq!(f.source_docs().len(), 3);
    assert_eq!(
        f.target_docs(),
        vec![doc! {"_id": 3, "v": 1}, doc! {"_id": 4}]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delete_is_stale_when_the_origin_gained_the_document() {
    let f = Fixture::by_id(vec![], vec![doc! {"_id": 4, "v": 1}]).await;
    let appeared = doc! {"_id": 4, "v": 1};
    f.source_inner
        .insert(&src_ns(), appeared.clone())
        .await
        .unwrap();
    let r = f.apply(&[op(k(4), Delete, ST)]).await;
    assert_eq!((r.stale, total(&r)), (1, 1));
    assert_eq!(f.target_docs(), vec![appeared]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_to_source_replace_is_stale_when_the_target_changed() {
    let f = Fixture::by_id(vec![doc! {"_id": 3, "v": 1}], vec![doc! {"_id": 3, "v": 2}]).await;
    f.target_inner
        .replace(&tgt_ns(), &Bson::Int32(3), doc! {"_id": 3, "v": 9})
        .await
        .unwrap();
    let r = f.apply(&[op(k(3), Replace, TS)]).await;
    assert_eq!((r.stale, r.applied, total(&r)), (1, 0, 1));
    assert_eq!(f.source_docs(), vec![doc! {"_id": 3, "v": 1}]);
    assert!(f.source.calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_key_that_appeared_since_the_compare_makes_the_op_stale() {
    let key = KeySpec::new(vec!["email".into()]).unwrap();
    let rules = CompareRules::for_key(&key);
    let f = Fixture::build(
        vec![doc! {"_id": 1, "email": "a", "v": 2}],
        vec![doc! {"_id": 50, "email": "a", "v": 1}],
        key,
        rules,
        doc! {},
    )
    .await;
    f.target_inner
        .insert(&tgt_ns(), doc! {"_id": 51, "email": "a", "v": 1})
        .await
        .unwrap();
    let r = f.apply(&[op(email("a"), Replace, ST)]).await;
    assert_eq!((r.stale, r.applied, total(&r)), (1, 0, 1));
    assert_eq!(*f.target.calls.lock().unwrap(), Vec::<usize>::new());
    assert_eq!(f.target_docs().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_reads_both_sides_from_the_primary_and_the_compare_does_not() {
    let f = Fixture::by_id(vec![doc! {"_id": 1, "v": 1}], vec![doc! {"_id": 1, "v": 2}]).await;
    for spy in [&f.source, &f.target] {
        assert_eq!(spy.primary_scans.load(Ordering::SeqCst), 0, "compare scans");
        assert!(spy.plain_scans.load(Ordering::SeqCst) > 0);
        spy.plain_scans.store(0, Ordering::SeqCst);
    }
    let r = f.apply(&[op(k(1), Replace, ST)]).await;
    assert_eq!(r.applied, 1);
    for spy in [&f.source, &f.target] {
        assert!(spy.primary_scans.load(Ordering::SeqCst) >= 1);
        assert_eq!(spy.plain_scans.load(Ordering::SeqCst), 0, "verification");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_with_the_wrong_number_of_outcomes_is_outcome_unknown() {
    let f = Fixture::by_id(vec![doc! {"_id": 1}, doc! {"_id": 2}], vec![]).await;
    f.target.short_reply.store(true, Ordering::SeqCst);
    let r = f.apply(&[op(k(1), Insert, ST), op(k(2), Insert, ST)]).await;
    assert_eq!((r.failed, r.applied), (2, 0));
    assert!(
        r.failures
            .iter()
            .all(|x| x.error == CoreError::ApplyOutcomeUnknown)
    );
}
