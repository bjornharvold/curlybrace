//! Apply through the runtime: events and their order, the job slot, cancel, close, panics and
//! the up-front destination checks. In-memory stores with gates make the timing deterministic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::Notify;

use super::closed_tests::Hooks;
use super::tests::{
    MapConnector, Tx, cfg, collect_run, expect_failed, next_compare, ns, same_pair, send_start,
    start_with,
};
use crate::compare::apply::{ApplyKind, ApplyOp, ApplyReport, Direction, IdPolicy};
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::protocol::{CompareCommand, CompareEvent, PairOutcome, RunId};
use crate::compare::results::Category;
use crate::error::{CoreError, Result};
use crate::runtime::{Command, RuntimeConfig, RuntimeHandle};
use crate::store::{
    CollectionKind, DocumentStore, IndexStatus, Namespace, Page, Query, WriteOp, WriteOutcome,
};
use crate::test_support::InMemoryStore;

const SENTINEL: &str = "SENTINEL-DOCUMENT-VALUE";

/// A rendezvous: the parked call announces itself, then waits for the test to release it.
#[derive(Default)]
pub(super) struct Gate {
    pub(super) entered: Notify,
    pub(super) release: Notify,
}

/// An in-memory store that can report another collection kind, park its next scan or bulk write
/// on a gate, panic in its next bulk write, and counts the bulk writes it received.
pub(super) struct ApplyStore {
    inner: InMemoryStore,
    kinds: Vec<(&'static str, CollectionKind)>,
    scan_gate: Mutex<Option<Arc<Gate>>>,
    bulk_gate: Mutex<Option<Arc<Gate>>>,
    pub(super) panic_bulk: AtomicBool,
    pub(super) panic_kinds: AtomicBool,
    pub(super) panic_scan: AtomicBool,
    pub(super) fail_reads: AtomicBool,
    pub(super) bulk_calls: AtomicUsize,
}

impl ApplyStore {
    pub(super) fn new(inner: InMemoryStore) -> Arc<Self> {
        Self::with_kinds(inner, Vec::new())
    }

    pub(super) fn with_kinds(
        inner: InMemoryStore,
        kinds: Vec<(&'static str, CollectionKind)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            kinds,
            scan_gate: Mutex::new(None),
            bulk_gate: Mutex::new(None),
            panic_bulk: AtomicBool::new(false),
            panic_kinds: AtomicBool::new(false),
            panic_scan: AtomicBool::new(false),
            fail_reads: AtomicBool::new(false),
            bulk_calls: AtomicUsize::new(0),
        })
    }

    pub(super) fn park_next_scan(&self, gate: &Arc<Gate>) {
        *self.scan_gate.lock().unwrap() = Some(gate.clone());
    }

    fn park_next_bulk(&self, gate: &Arc<Gate>) {
        *self.bulk_gate.lock().unwrap() = Some(gate.clone());
    }

    pub(super) fn writes(&self) -> usize {
        self.bulk_calls.load(Ordering::SeqCst)
    }

    pub(super) async fn docs(&self, ns: &Namespace) -> Vec<Document> {
        let q = Query {
            sort: Some(doc! { "_id": 1 }),
            ..Query::default()
        };
        self.inner.find(ns, &q).await.unwrap().docs
    }
}

async fn park(slot: &Mutex<Option<Arc<Gate>>>) {
    let gate = slot.lock().unwrap().take();
    if let Some(gate) = gate {
        gate.entered.notify_one();
        gate.release.notified().await;
    }
}

#[async_trait]
impl DocumentStore for ApplyStore {
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
    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.inner.estimated_count(ns).await
    }
    async fn key_index_status(&self, _: &Namespace, _: &KeySpec) -> Result<IndexStatus> {
        Ok(IndexStatus::Indexed)
    }
    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        assert!(!self.panic_scan.load(Ordering::SeqCst), "store blew up");
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(CoreError::Database("read failed".into()));
        }
        park(&self.scan_gate).await;
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        assert!(!self.panic_kinds.load(Ordering::SeqCst), "store blew up");
        let mut listed = self.inner.list_collection_kinds(db).await?;
        for (name, kind) in &self.kinds {
            match listed.iter_mut().find(|(n, _)| n == name) {
                Some(entry) => entry.1 = *kind,
                None => listed.push((name.to_string(), *kind)),
            }
        }
        Ok(listed)
    }
    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        self.bulk_calls.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic_bulk.load(Ordering::SeqCst), "store blew up");
        park(&self.bulk_gate).await;
        self.inner.bulk_write(ns, ops).await
    }
}

/// Parks the target's next bulk write on a gate and returns the gate.
pub(super) fn park_target_bulk(r: &Rig) -> Arc<Gate> {
    let gate = Arc::new(Gate::default());
    r.target.park_next_bulk(&gate);
    gate
}

pub(super) fn source_store() -> InMemoryStore {
    InMemoryStore::new()
        .with_docs(
            &ns("d", "c"),
            vec![
                doc! { "_id": 1, "v": SENTINEL },
                doc! { "_id": 2, "v": "new" },
            ],
        )
        .with_docs(&ns("d", "c2"), vec![doc! { "_id": 1, "v": SENTINEL }])
}

pub(super) fn target_store() -> InMemoryStore {
    InMemoryStore::new()
        .with_docs(&ns("d", "c"), vec![doc! { "_id": 2, "v": "old" }])
        .with_docs(&ns("d", "c3"), vec![doc! { "_id": 7, "v": "t" }])
}

pub(super) struct Rig {
    pub(super) tx: Tx,
    pub(super) h: RuntimeHandle,
    pub(super) source: Arc<ApplyStore>,
    pub(super) target: Arc<ApplyStore>,
}

fn connector(source: &Arc<ApplyStore>, target: &Arc<ApplyStore>) -> MapConnector {
    let stores: HashMap<String, Arc<dyn DocumentStore>> = HashMap::from([
        (
            "mongodb://a".to_string(),
            source.clone() as Arc<dyn DocumentStore>,
        ),
        (
            "mongodb://b".to_string(),
            target.clone() as Arc<dyn DocumentStore>,
        ),
    ]);
    MapConnector {
        stores,
        ..MapConnector::default()
    }
}

/// An opened session (id 1) whose pairs 1 (`d.c`) and 2 (`d.c2` onto `d.c2`) have been compared.
pub(super) async fn rig(source: Arc<ApplyStore>, target: Arc<ApplyStore>) -> Rig {
    let (tx, mut h) = start_with(connector(&source, &target), RuntimeConfig::default());
    super::tests::open(&mut h, &tx, 1).await;
    send_start(&tx, 1, vec![same_pair(1), c2_pair()]);
    let done = collect_run(&mut h).await;
    assert!(
        done.iter()
            .all(|(_, o)| matches!(o, PairOutcome::Completed(_)))
    );
    Rig {
        tx,
        h,
        source,
        target,
    }
}

fn c2_pair() -> crate::compare::protocol::PairSpec {
    super::tests::pair(2, ns("d", "c2"), ns("d", "c2"))
}

pub(super) async fn default_rig() -> Rig {
    rig(
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
    )
    .await
}

pub(super) fn key(id: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(id)])
}

pub(super) fn op(pair: u32, id: i32, kind: ApplyKind) -> ApplyOp {
    ApplyOp {
        pair,
        key: key(id),
        kind,
        direction: Direction::SourceToTarget,
        fields: Vec::new(),
    }
}

pub(super) fn send_apply(tx: &Tx, run: RunId, ops: Vec<ApplyOp>) {
    let apply = CompareCommand::Apply {
        session: 1,
        run,
        ops,
        bulk: Vec::new(),
        id_policy: IdPolicy::Keep,
    };
    tx.send(Command::Compare(apply)).unwrap();
}

/// Events up to and including `ApplyDone`.
pub(super) async fn collect_apply(h: &mut RuntimeHandle) -> Vec<CompareEvent> {
    let mut events = Vec::new();
    loop {
        let event = next_compare(h).await;
        let last = matches!(event, CompareEvent::ApplyDone { .. });
        events.push(event);
        if last {
            return events;
        }
    }
}

pub(super) fn pair_reports(events: &[CompareEvent]) -> Vec<(u32, ApplyReport)> {
    events
        .iter()
        .filter_map(|e| match e {
            CompareEvent::ApplyPairDone { pair, report, .. } => Some((*pair, report.clone())),
            _ => None,
        })
        .collect()
}

/// Proves every command sent so far was handled (unknown sessions are answered in order).
pub(super) async fn barrier(tx: &Tx, h: &mut RuntimeHandle) {
    let rows = CompareCommand::Rows {
        session: 99,
        pair: 1,
        category: Category::Different,
        offset: 0,
        limit: 1,
    };
    tx.send(Command::Compare(rows)).unwrap();
    match next_compare(h).await {
        CompareEvent::Failed { session: 99, .. } => {}
        other => panic!("{other:?}"),
    }
}

/// Bounds a wait so a regression fails instead of hanging.
pub(super) async fn within<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), f)
        .await
        .expect("timed out")
}

pub(super) async fn assert_silent(h: &mut RuntimeHandle) {
    let late = tokio::time::timeout(Duration::from_millis(200), h.rx.recv()).await;
    assert!(late.is_err(), "no event expected: {late:?}");
}

#[tokio::test]
async fn apply_writes_the_selected_rows_and_reports_per_pair() {
    let mut r = default_rig().await;
    let ops = vec![
        op(1, 1, ApplyKind::Insert),
        op(1, 2, ApplyKind::Replace),
        op(2, 1, ApplyKind::Insert),
    ];
    send_apply(&r.tx, 5, ops);
    let events = within(collect_apply(&mut r.h)).await;
    let shape: Vec<&str> = events
        .iter()
        .map(|e| match e {
            CompareEvent::ApplyProgress { .. } => "progress",
            CompareEvent::ApplyPairDone { pair: 1, .. } => "pair1",
            CompareEvent::ApplyPairDone { pair: 2, .. } => "pair2",
            CompareEvent::ApplyDone { run: 5, .. } => "done",
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        ["progress", "pair1", "progress", "pair2", "done"],
        "progress precedes its pair's terminal; one done at the end"
    );
    let reports = pair_reports(&events);
    assert_eq!((reports[0].1.applied, reports[0].1.failed), (2, 0));
    assert_eq!((reports[1].1.applied, reports[1].1.failed), (1, 0));
    let c = ns("d", "c");
    assert_eq!(
        r.target.docs(&c).await,
        vec![
            doc! { "_id": 1, "v": SENTINEL },
            doc! { "_id": 2, "v": "new" }
        ]
    );
    assert_eq!(r.target.docs(&ns("d", "c2")).await.len(), 1);
    assert_silent(&mut r.h).await;
}

#[tokio::test]
async fn apply_is_refused_while_a_run_is_active_and_a_run_while_an_apply_is_active() {
    let mut r = default_rig().await;
    let scan = Arc::new(Gate::default());
    r.source.park_next_scan(&scan);
    send_start(&r.tx, 8, vec![same_pair(1)]);
    within(scan.entered.notified()).await;
    send_apply(&r.tx, 9, vec![op(1, 1, ApplyKind::Insert)]);
    expect_failed(&mut r.h, "compare_apply").await;
    scan.release.notify_one();
    within(collect_run(&mut r.h)).await;

    let bulk = Arc::new(Gate::default());
    r.target.park_next_bulk(&bulk);
    send_apply(&r.tx, 10, vec![op(1, 1, ApplyKind::Insert)]);
    within(bulk.entered.notified()).await;
    send_start(&r.tx, 11, vec![same_pair(1)]);
    expect_failed(&mut r.h, "compare_start").await;
    send_apply(&r.tx, 12, vec![op(1, 1, ApplyKind::Insert)]);
    expect_failed(&mut r.h, "compare_apply").await;
    bulk.release.notify_one();
    let events = within(collect_apply(&mut r.h)).await;
    assert_eq!(pair_reports(&events)[0].1.applied, 1);
}

#[tokio::test]
async fn apply_for_an_unknown_or_discarded_pair_is_refused_and_writes_nothing() {
    let mut r = default_rig().await;
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(99, 1, ApplyKind::Insert)],
    );
    expect_failed(&mut r.h, "compare_apply").await;
    let discard = CompareCommand::Discard {
        session: 1,
        pair: 2,
    };
    r.tx.send(Command::Compare(discard)).unwrap();
    send_apply(&r.tx, 6, vec![op(2, 1, ApplyKind::Insert)]);
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_apply",
            error: CoreError::Unsupported(text),
            ..
        } => assert_eq!(text, "no results for this pair"),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
    assert_silent(&mut r.h).await;
}

#[tokio::test]
async fn a_pair_discarded_after_the_request_fails_its_ops_as_not_found() {
    let mut r = default_rig().await;
    let gate = Arc::new(Gate::default());
    r.target.park_next_bulk(&gate);
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(2, 1, ApplyKind::Insert)],
    );
    within(gate.entered.notified()).await;
    let discard = CompareCommand::Discard {
        session: 1,
        pair: 2,
    };
    r.tx.send(Command::Compare(discard)).unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
    assert_eq!(reports[1].1.failed, 1);
    assert_eq!(reports[1].1.failures[0].error, CoreError::NotFound);
    assert!(r.target.docs(&ns("d", "c2")).await.is_empty());
}

#[tokio::test]
async fn apply_into_a_view_is_refused_up_front_and_writes_nothing() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c2", CollectionKind::View)]);
    let mut r = rig(ApplyStore::new(source_store()), target).await;
    // Pair 1 is writable; pair 2 is not: nothing at all may be written.
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(2, 1, ApplyKind::Insert)],
    );
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_apply",
            error,
            ..
        } => assert_eq!(error, CoreError::ApplyTargetNotWritable),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
    assert_eq!(r.target.docs(&ns("d", "c")).await.len(), 1);
    // The slot was freed before the `Failed` was sent: an immediate Start is accepted.
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn apply_to_a_timeseries_destination_allows_insert_only() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c", CollectionKind::Timeseries)]);
    let mut r = rig(ApplyStore::new(source_store()), target).await;
    send_apply(&r.tx, 5, vec![op(1, 2, ApplyKind::Replace)]);
    match next_compare(&mut r.h).await {
        CompareEvent::Failed { error, .. } => assert_eq!(error, CoreError::ApplyTargetNotWritable),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
    send_apply(&r.tx, 6, vec![op(1, 1, ApplyKind::Insert)]);
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
}

#[tokio::test]
async fn duplicate_ops_are_refused() {
    let mut r = default_rig().await;
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(1, 1, ApplyKind::Delete)],
    );
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_apply",
            error: CoreError::Unsupported(text),
            ..
        } => assert_eq!(text, "duplicate operation"),
        other => panic!("{other:?}"),
    }
    // No ops at all is refused too.
    send_apply(&r.tx, 6, Vec::new());
    expect_failed(&mut r.h, "compare_apply").await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn cancel_mid_apply_emits_pair_done_with_not_run_then_apply_done_and_frees_the_slot() {
    let mut r = default_rig().await;
    let gate = Arc::new(Gate::default());
    r.target.park_next_bulk(&gate);
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(2, 1, ApplyKind::Insert)],
    );
    within(gate.entered.notified()).await;
    let cancel = CompareCommand::Cancel { session: 1, run: 5 };
    r.tx.send(Command::Compare(cancel)).unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].1.applied, 1, "the batch in flight finishes");
    assert_eq!(reports[1].1.not_run, 1);
    assert_eq!(reports[1].1.applied, 0);
    assert!(r.target.docs(&ns("d", "c2")).await.is_empty());
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn closing_the_session_mid_apply_emits_nothing_more() {
    let source = ApplyStore::new(source_store());
    let target = ApplyStore::new(target_store());
    // Held outside the session: this handle and the connector's map.
    let baseline = Arc::strong_count(&target) + 1;
    let mut r = rig(source, target).await;
    let gate = Arc::new(Gate::default());
    r.target.park_next_bulk(&gate);
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(2, 1, ApplyKind::Insert)],
    );
    within(gate.entered.notified()).await;
    r.tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    assert_silent(&mut r.h).await;
    assert!(
        r.target.docs(&ns("d", "c2")).await.is_empty(),
        "pair 2 untouched"
    );
    for _ in 0..200 {
        if Arc::strong_count(&r.target) <= baseline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the session's connections were not released");
}

#[tokio::test]
async fn a_panicking_store_stops_the_apply_frees_the_slot_and_ends_with_apply_done() {
    let mut r = default_rig().await;
    r.target.panic_bulk.store(true, Ordering::SeqCst);
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(2, 1, ApplyKind::Insert)],
    );
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports.len(), 2, "each pair still gets its terminal");
    assert_eq!((reports[0].1.failed, reports[0].1.applied), (1, 0));
    assert_eq!(
        reports[0].1.failures[0].error,
        CoreError::ApplyOutcomeUnknown,
        "a panic during the write leaves the outcome unknown"
    );
    assert_eq!(
        (reports[1].1.not_run, reports[1].1.failed),
        (1, 0),
        "the panic stops the rest"
    );
    assert_eq!(r.target.writes(), 1, "pair 2 never reached the store");
    assert!(r.target.docs(&ns("d", "c2")).await.is_empty());
    assert!(matches!(
        events.last(),
        Some(CompareEvent::ApplyDone { run: 5, .. })
    ));
    r.target.panic_bulk.store(false, Ordering::SeqCst);
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn a_panicking_destination_check_is_a_refusal_and_frees_the_slot() {
    let mut r = default_rig().await;
    r.target.panic_kinds.store(true, Ordering::SeqCst);
    send_apply(&r.tx, 5, vec![op(1, 1, ApplyKind::Insert)]);
    expect_failed(&mut r.h, "compare_apply").await;
    r.target.panic_kinds.store(false, Ordering::SeqCst);
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
    assert_eq!(r.target.writes(), 0);
}

fn to_source(mut op: ApplyOp) -> ApplyOp {
    op.direction = Direction::TargetToSource;
    op
}

#[tokio::test]
async fn target_to_source_into_a_view_is_refused_and_into_a_missing_collection_allowed() {
    let source = ApplyStore::with_kinds(source_store(), vec![("c", CollectionKind::View)]);
    let mut r = rig(source, ApplyStore::new(target_store())).await;
    send_apply(&r.tx, 5, vec![to_source(op(1, 2, ApplyKind::Replace))]);
    match next_compare(&mut r.h).await {
        CompareEvent::Failed { error, .. } => assert_eq!(error, CoreError::ApplyTargetNotWritable),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.source.writes(), 0);
    // Pair 3: its source collection does not exist yet; the insert creates it.
    send_start(
        &r.tx,
        6,
        vec![super::tests::pair(3, ns("d", "c3"), ns("d", "c3"))],
    );
    within(collect_run(&mut r.h)).await;
    send_apply(&r.tx, 7, vec![to_source(op(3, 7, ApplyKind::Insert))]);
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
    assert_eq!(r.source.docs(&ns("d", "c3")).await.len(), 1);
}

#[tokio::test]
async fn keys_equal_across_numeric_types_are_duplicate_ops() {
    let mut r = default_rig().await;
    let mut wide = op(1, 1, ApplyKind::Insert);
    wide.key = KeyTuple(vec![Bson::Int64(1)]);
    send_apply(&r.tx, 5, vec![op(1, 1, ApplyKind::Insert), wide]);
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            error: CoreError::Unsupported(text),
            ..
        } => assert_eq!(text, "duplicate operation"),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn an_apply_before_opened_is_refused() {
    let (source, target) = (
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
    );
    let hooks = Arc::new(Hooks::default());
    let mut conn = connector(&source, &target);
    conn.parked = HashMap::from([("mongodb://b".to_string(), hooks.clone())]);
    let (tx, mut h) = start_with(conn, RuntimeConfig::default());
    tx.send(Command::Compare(CompareCommand::Open {
        session: 1,
        source: cfg("mongodb://a"),
        target: cfg("mongodb://b"),
    }))
    .unwrap();
    within(hooks.entered.notified()).await;
    send_apply(&tx, 5, vec![op(1, 1, ApplyKind::Insert)]);
    expect_failed(&mut h, "compare_apply").await;
    hooks.release.notify_one();
    assert_eq!(
        next_compare(&mut h).await,
        CompareEvent::Opened { session: 1 }
    );
}

#[tokio::test]
async fn no_event_contains_a_document_value_in_its_debug_output_except_op_failure_keys() {
    let mut r = default_rig().await;
    // Mutate the destination behind the compare's back so one op fails verification (stale).
    let stale = doc! { "_id": 2, "v": "changed" };
    r.target
        .inner
        .replace(&ns("d", "c"), &Bson::Int32(2), stale)
        .await
        .unwrap();
    let ops = vec![
        op(1, 1, ApplyKind::Insert),
        op(1, 2, ApplyKind::Replace),
        op(2, 1, ApplyKind::Insert),
    ];
    send_apply(&r.tx, 5, ops);
    let events = within(collect_apply(&mut r.h)).await;
    let text = format!("{events:?}");
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(!text.contains("changed") && !text.contains("old"), "{text}");
    assert_eq!(pair_reports(&events)[0].1.stale, 1);
}
