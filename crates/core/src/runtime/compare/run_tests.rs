//! Run control: refusal of a second start, cancel, close mid-run, panic containment, and the
//! budget bookkeeping of stored results. Gated stores make the timing deterministic.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::Notify;

use super::runner;
use super::session::{Connections, Session};
use super::tests::{
    MapConnector, Tx, collect_run, expect_failed, next_compare, ns, open, pair, same_pair,
    send_rows, send_start, start_with,
};
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::protocol::{CompareCommand, CompareEvent, PairOutcome};
use crate::compare::results::{Category, Limits};
use crate::error::{CoreError, Result};
use crate::runtime::{Command, Event, RuntimeConfig, RuntimeHandle};
use crate::store::{DocumentStore, IndexStatus, Namespace, Page, Query};
use crate::test_support::InMemoryStore;

/// A rendezvous: the first scan announces itself, then waits for the test to release it.
#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

/// An in-memory store whose first sorted scan parks on a gate, and whose scans of one named
/// collection panic.
struct GateStore {
    inner: InMemoryStore,
    gate: Option<Arc<Gate>>,
    parked: AtomicBool,
    panic_on: Option<&'static str>,
}

impl GateStore {
    fn new(inner: InMemoryStore) -> Self {
        Self {
            inner,
            gate: None,
            parked: AtomicBool::new(false),
            panic_on: None,
        }
    }

    fn gated(self, gate: &Arc<Gate>) -> Self {
        Self {
            gate: Some(gate.clone()),
            ..self
        }
    }

    fn panicking_on(self, collection: &'static str) -> Self {
        Self {
            panic_on: Some(collection),
            ..self
        }
    }
}

#[async_trait]
impl DocumentStore for GateStore {
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
        assert!(
            self.panic_on != Some(ns.collection.as_str()),
            "store blew up"
        );
        let first = !self.parked.swap(true, Ordering::SeqCst);
        if let (true, Some(gate)) = (first, &self.gate) {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
}

fn docs(count: i32) -> Vec<Document> {
    (0..count).map(|n| doc! { "_id": n }).collect()
}

fn store_with(collection: &str, count: i32) -> InMemoryStore {
    InMemoryStore::new().with_docs(&ns("d", collection), docs(count))
}

fn rig(source: GateStore, target: GateStore) -> (Tx, RuntimeHandle) {
    let stores = [("mongodb://a", source), ("mongodb://b", target)]
        .into_iter()
        .map(|(uri, s)| (uri.to_string(), Arc::new(s) as Arc<dyn DocumentStore>))
        .collect();
    let connector = MapConnector {
        stores,
        ..MapConnector::default()
    };
    start_with(connector, RuntimeConfig::default())
}

/// An opened session whose source parks its first scan on the returned gate.
async fn gated_rig() -> (Tx, RuntimeHandle, Arc<Gate>) {
    let gate = Arc::new(Gate::default());
    let source = GateStore::new(store_with("c", 5)).gated(&gate);
    let (tx, mut h) = rig(source, GateStore::new(store_with("c", 5)));
    open(&mut h, &tx, 1).await;
    (tx, h, gate)
}

/// Proves every command sent so far was handled: a command for an unknown session is answered
/// `Failed` synchronously from the dispatcher, which handles commands in order.
async fn barrier(tx: &Tx, h: &mut RuntimeHandle) {
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

/// A parked first scan on the source of a one-session rig; returns once the run is mid-scan.
async fn parked_run(pairs: u32) -> (Tx, RuntimeHandle, Arc<Gate>) {
    let (tx, h, gate) = gated_rig().await;
    send_start(&tx, 7, (1..=pairs).map(same_pair).collect());
    gate.entered.notified().await;
    (tx, h, gate)
}

#[tokio::test]
async fn a_second_start_while_running_is_refused_and_the_first_run_finishes() {
    let (tx, mut h, gate) = parked_run(1).await;
    send_start(&tx, 8, vec![same_pair(1)]);
    expect_failed(&mut h, "compare_start").await;
    gate.release.notify_one();
    let done = collect_run(&mut h).await;
    assert_eq!(done.len(), 1);
    assert!(matches!(done[0].1, PairOutcome::Completed(_)));
}

#[tokio::test]
async fn a_start_after_the_run_finished_is_accepted() {
    let (tx, mut h, gate) = parked_run(1).await;
    gate.release.notify_one();
    collect_run(&mut h).await;
    send_start(&tx, 9, vec![same_pair(1)]);
    assert_eq!(collect_run(&mut h).await.len(), 1);
}

#[tokio::test]
async fn cancel_ends_every_pair_as_cancelled_and_stores_none() {
    let (tx, mut h, gate) = parked_run(3).await;
    let cancel = CompareCommand::Cancel { session: 1, run: 7 };
    tx.send(Command::Compare(cancel)).unwrap();
    barrier(&tx, &mut h).await;
    gate.release.notify_one();
    let done = collect_run(&mut h).await;
    assert_eq!(done.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(
        done.iter()
            .all(|(_, o)| matches!(o, PairOutcome::Cancelled(_)))
    );
    send_rows(&tx, 1, Category::OnlyInSource);
    expect_failed(&mut h, "compare_rows").await;
}

#[tokio::test]
async fn a_cancel_right_behind_a_start_cancels_the_run() {
    let (tx, mut h, gate) = gated_rig().await;
    send_start(&tx, 7, vec![same_pair(1), same_pair(2)]);
    let cancel = CompareCommand::Cancel { session: 1, run: 7 };
    tx.send(Command::Compare(cancel)).unwrap();
    barrier(&tx, &mut h).await;
    gate.release.notify_one();
    let done = collect_run(&mut h).await;
    assert_eq!(done.len(), 2);
    assert!(
        done.iter()
            .all(|(_, o)| matches!(o, PairOutcome::Cancelled(_)))
    );
}

#[tokio::test]
async fn two_starts_in_a_row_the_first_wins_and_the_second_is_refused() {
    let (tx, mut h, gate) = gated_rig().await;
    send_start(&tx, 7, vec![same_pair(1)]);
    send_start(&tx, 8, vec![same_pair(1)]);
    expect_failed(&mut h, "compare_start").await;
    gate.release.notify_one();
    let done = collect_run(&mut h).await;
    assert_eq!(done.len(), 1);
    assert!(matches!(done[0].1, PairOutcome::Completed(_)));
}

#[tokio::test]
async fn a_start_for_an_unknown_session_fails() {
    let (tx, mut h, _gate) = gated_rig().await;
    let start = CompareCommand::Start {
        session: 5,
        run: 1,
        pairs: vec![same_pair(1)],
    };
    tx.send(Command::Compare(start)).unwrap();
    expect_failed(&mut h, "compare_start").await;
}

#[tokio::test]
async fn a_cancel_for_another_run_is_ignored() {
    let (tx, mut h, gate) = parked_run(1).await;
    let cancel = CompareCommand::Cancel {
        session: 1,
        run: 99,
    };
    tx.send(Command::Compare(cancel)).unwrap();
    gate.release.notify_one();
    let done = collect_run(&mut h).await;
    assert!(matches!(done[0].1, PairOutcome::Completed(_)));
}

#[tokio::test]
async fn closing_mid_run_emits_nothing_after_the_close() {
    let (tx, mut h, gate) = parked_run(2).await;
    tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    // The session is gone once this synchronous answer arrives: the close was handled.
    send_rows(&tx, 1, Category::Different);
    expect_failed(&mut h, "compare_rows").await;
    gate.release.notify_one();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let late = h.rx.try_recv();
    assert!(late.is_err(), "a closed session stays silent: {late:?}");
}

#[tokio::test]
async fn a_panicking_pair_fails_alone_and_the_run_goes_on() {
    let source = InMemoryStore::new()
        .with_docs(&ns("d", "c"), docs(3))
        .with_docs(&ns("d", "boom"), docs(3));
    let source = GateStore::new(source).panicking_on("boom");
    let (tx, mut h) = rig(source, GateStore::new(store_with("c", 3)));
    open(&mut h, &tx, 1).await;
    let boom = pair(2, ns("d", "boom"), ns("d", "c"));
    send_start(&tx, 1, vec![same_pair(1), boom, same_pair(3)]);
    let done = collect_run(&mut h).await;
    assert!(matches!(done[0].1, PairOutcome::Completed(_)));
    match &done[1].1 {
        PairOutcome::Failed(CoreError::Database(m)) => assert_eq!(m, "internal error"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(done[2].1, PairOutcome::Completed(_)));
}

/// A session over two stores, driven directly so the shared budget is observable.
struct Direct {
    session: Arc<Session>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
}

impl Direct {
    fn new(source: InMemoryStore, target: InMemoryStore, limits: Limits) -> Self {
        let session = Session::new(limits);
        let connections = Connections {
            source: Arc::new(source),
            target: Arc::new(target),
            _tunnels: Vec::new(),
        };
        assert!(session.connections.set(connections).is_ok());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self { session, rx, tx }
    }

    async fn run(&mut self, run: u64, ids: &[u32]) -> Vec<PairOutcome> {
        let pairs = ids.iter().map(|id| same_pair(*id)).collect();
        self.run_pairs(run, pairs, Arc::new(|| {})).await
    }

    async fn run_pairs(
        &mut self,
        run: u64,
        pairs: Vec<crate::compare::protocol::PairSpec>,
        repaint: crate::runtime::Repaint,
    ) -> Vec<PairOutcome> {
        runner::start(
            self.tx.clone(),
            repaint,
            self.session.clone(),
            1,
            run,
            pairs,
        )
        .unwrap();
        let mut outcomes = Vec::new();
        loop {
            match self.rx.recv().await.expect("the channel is open") {
                Event::Compare(CompareEvent::PairDone { outcome, .. }) => outcomes.push(outcome),
                Event::Compare(CompareEvent::RunDone { .. }) => return outcomes,
                _ => {}
            }
        }
    }
}

#[tokio::test]
async fn stored_results_hold_budget_until_discarded() {
    let mut direct = Direct::new(store_with("c", 3), InMemoryStore::new(), Limits::default());
    direct.run(1, &[1, 2]).await;
    assert_eq!(direct.session.budget.used_entries(), 6);
    runner::discard_pair(&direct.session, 1);
    assert_eq!(direct.session.budget.used_entries(), 3);
    runner::discard_pair(&direct.session, 2);
    runner::discard_pair(&direct.session, 2);
    assert_eq!(direct.session.budget.used_entries(), 0);
    assert_eq!(direct.session.budget.used_bytes(), 0);
}

#[tokio::test]
async fn a_pair_that_fails_on_the_cap_returns_its_budget_share() {
    let limits = Limits {
        max_differences: 2,
        ..Limits::default()
    };
    let mut direct = Direct::new(store_with("c", 5), InMemoryStore::new(), limits);
    let outcomes = direct.run(1, &[1]).await;
    assert!(matches!(
        outcomes[0],
        PairOutcome::Failed(CoreError::CompareCapExceeded)
    ));
    assert_eq!(direct.session.budget.used_entries(), 0);
    assert_eq!(direct.session.budget.used_bytes(), 0);
}

#[tokio::test]
async fn closing_the_session_returns_every_stored_share() {
    let mut direct = Direct::new(store_with("c", 3), InMemoryStore::new(), Limits::default());
    direct.run(1, &[1]).await;
    assert_eq!(direct.session.budget.used_entries(), 3);
    direct.session.close();
    assert_eq!(direct.session.budget.used_entries(), 0);
}

#[tokio::test]
async fn a_run_on_a_closed_session_is_refused() {
    let direct = Direct::new(store_with("c", 3), InMemoryStore::new(), Limits::default());
    direct.session.close();
    let repaint: crate::runtime::Repaint = Arc::new(|| {});
    let started = runner::start(
        direct.tx.clone(),
        repaint,
        direct.session.clone(),
        1,
        1,
        vec![],
    );
    assert!(started.is_err());
}

#[tokio::test]
async fn pair_events_arrive_in_pair_order_with_progress_between() {
    let limits = Limits {
        batch_size: 2,
        ..Limits::default()
    };
    let mut direct = Direct::new(store_with("c", 9), InMemoryStore::new(), limits);
    let repaint: crate::runtime::Repaint = Arc::new(|| {});
    let pairs = vec![same_pair(1), same_pair(2)];
    runner::start(
        direct.tx.clone(),
        repaint,
        direct.session.clone(),
        1,
        4,
        pairs,
    )
    .unwrap();
    let mut seen = Vec::new();
    loop {
        match direct.rx.recv().await.unwrap() {
            Event::Compare(CompareEvent::Progress { pair, .. }) => seen.push(("progress", pair)),
            Event::Compare(CompareEvent::PairDone { pair, .. }) => seen.push(("done", pair)),
            Event::Compare(CompareEvent::RunDone { .. }) => break,
            _ => {}
        }
    }
    let dones: Vec<_> = seen.iter().filter(|(k, _)| *k == "done").collect();
    assert_eq!(dones, [&("done", 1), &("done", 2)]);
    assert_eq!(seen.last(), Some(&("done", 2)));
    assert!(seen.contains(&("progress", 1)));
}

#[tokio::test]
async fn a_panic_outside_the_engine_frees_the_slot_and_ends_the_run() {
    let mut direct = Direct::new(store_with("c", 3), InMemoryStore::new(), Limits::default());
    // The repaint hook runs after each emission, in the run task, outside the engine.
    let repaint: crate::runtime::Repaint = Arc::new(|| panic!("repaint blew up"));
    let outcomes = direct.run_pairs(1, vec![same_pair(1)], repaint).await;
    assert_eq!(
        outcomes.len(),
        1,
        "the PairDone was sent before the hook panicked"
    );
    // The slot is free again: a new run is accepted and finishes.
    assert_eq!(direct.run(2, &[1]).await.len(), 1);
}

#[tokio::test]
async fn a_rerun_that_fails_or_is_cancelled_leaves_no_stale_results() {
    let mut direct = Direct::new(store_with("c", 3), InMemoryStore::new(), Limits::default());
    direct.run(1, &[1]).await;
    assert_eq!(direct.session.budget.used_entries(), 3);
    let mut bad = same_pair(1);
    bad.key = KeySpec::new(vec!["nope".into()]).unwrap();
    bad.rules = crate::compare::rules::CompareRules::for_key(&bad.key);
    let repaint: crate::runtime::Repaint = Arc::new(|| {});
    let outcomes = direct.run_pairs(2, vec![bad], repaint).await;
    assert!(matches!(outcomes[0], PairOutcome::Failed(_)));
    assert_eq!(direct.session.budget.used_entries(), 0);
    let (events, repaint) = (
        direct.tx.clone(),
        Arc::new(|| {}) as crate::runtime::Repaint,
    );
    let rows = runner::rows(
        &direct.session,
        &events,
        &repaint,
        1,
        1,
        Category::OnlyInSource,
        0,
        10,
    );
    assert!(rows.is_err(), "no stale rows after a failed re-run");
}

#[tokio::test]
async fn a_rows_page_is_clamped() {
    let mut direct = Direct::new(
        store_with("c", (runner::MAX_ROWS_PAGE + 200) as i32),
        InMemoryStore::new(),
        Limits::default(),
    );
    direct.run(1, &[1]).await;
    let (events, repaint) = (
        direct.tx.clone(),
        Arc::new(|| {}) as crate::runtime::Repaint,
    );
    let category = Category::OnlyInSource;
    runner::rows(
        &direct.session,
        &events,
        &repaint,
        1,
        1,
        category,
        0,
        usize::MAX,
    )
    .unwrap();
    loop {
        if let Event::Compare(CompareEvent::Rows { rows, total, .. }) =
            direct.rx.recv().await.unwrap()
        {
            assert_eq!(rows.len(), runner::MAX_ROWS_PAGE);
            assert_eq!(total, runner::MAX_ROWS_PAGE + 200);
            return;
        }
    }
}
