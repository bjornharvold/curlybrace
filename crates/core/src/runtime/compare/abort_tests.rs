//! Cancel and close abort a store call that never returns; the per-pair index guard; the
//! validation of pair specs at the runtime boundary.

use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::Notify;

use super::runner;
use super::session::{Connections, Session};
use super::tests::{
    MapConnector, Tx, collect_run, next_compare, ns, open, same_pair, send_start, start_with,
};
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::protocol::{CompareCommand, CompareEvent, PairOutcome, PairSpec};
use crate::compare::results::Limits;
use crate::compare::rules::FieldScope;
use crate::error::{CoreError, Result};
use crate::runtime::{Command, Event, RuntimeConfig, RuntimeHandle};
use crate::store::{DocumentStore, IndexStatus, Namespace, Page, Query};
use crate::test_support::InMemoryStore;

/// An in-memory store that can block every sorted scan forever and report a chosen index
/// status and estimated count.
struct ProbeStore {
    inner: InMemoryStore,
    block: Option<Arc<Notify>>,
    index: IndexStatus,
    estimate: Option<u64>,
}

impl ProbeStore {
    fn new(count: i32) -> Self {
        let docs = (0..count).map(|n| doc! { "_id": n }).collect();
        Self {
            inner: InMemoryStore::new().with_docs(&ns("d", "c"), docs),
            block: None,
            index: IndexStatus::Indexed,
            estimate: None,
        }
    }

    /// Every scan announces itself on `entered`, then never returns.
    fn blocking(self, entered: &Arc<Notify>) -> Self {
        Self {
            block: Some(entered.clone()),
            ..self
        }
    }

    fn unindexed(self, estimate: u64) -> Self {
        Self {
            index: IndexStatus::NotIndexed,
            estimate: Some(estimate),
            ..self
        }
    }
}

#[async_trait]
impl DocumentStore for ProbeStore {
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
        match self.estimate {
            Some(n) => Ok(n),
            None => self.inner.estimated_count(ns).await,
        }
    }
    async fn key_index_status(&self, _: &Namespace, _: &KeySpec) -> Result<IndexStatus> {
        Ok(self.index)
    }
    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        if let Some(entered) = &self.block {
            entered.notify_one();
            std::future::pending::<()>().await;
        }
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
}

fn rig(source: ProbeStore, target: ProbeStore) -> (Tx, RuntimeHandle) {
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

/// A session over two stores, driven directly (not through the registry).
fn direct_session(
    source: Arc<ProbeStore>,
    target: Arc<ProbeStore>,
    limits: Limits,
) -> Arc<Session> {
    let session = Session::new(limits);
    let connections = Connections {
        source,
        target,
        _tunnels: Vec::new(),
    };
    assert!(session.connections.set(connections).is_ok());
    session
}

async fn run_direct(session: &Arc<Session>, pairs: Vec<PairSpec>) -> Vec<PairOutcome> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    runner::start(tx, Arc::new(|| {}), session.clone(), 1, 1, pairs).unwrap();
    let mut outcomes = Vec::new();
    loop {
        match rx.recv().await.expect("the channel is open") {
            Event::Compare(CompareEvent::PairDone { outcome, .. }) => outcomes.push(outcome),
            Event::Compare(CompareEvent::RunDone { .. }) => return outcomes,
            _ => {}
        }
    }
}

/// Polls until `weak` can no longer be upgraded; false after two seconds.
async fn released<T: ?Sized>(weak: &Weak<T>) -> bool {
    for _ in 0..200 {
        if weak.upgrade().is_none() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn cancel_aborts_a_scan_that_never_returns() {
    let entered = Arc::new(Notify::new());
    let (tx, mut h) = rig(ProbeStore::new(5).blocking(&entered), ProbeStore::new(5));
    open(&mut h, &tx, 1).await;
    send_start(&tx, 7, vec![same_pair(1), same_pair(2)]);
    entered.notified().await;
    let cancel = CompareCommand::Cancel { session: 1, run: 7 };
    tx.send(Command::Compare(cancel)).unwrap();
    let done = tokio::time::timeout(Duration::from_secs(2), collect_run(&mut h))
        .await
        .expect("the run ends promptly although the store never answers");
    assert_eq!(done.len(), 2);
    assert!(
        done.iter()
            .all(|(_, o)| matches!(o, PairOutcome::Cancelled(_))),
        "{done:?}"
    );
}

#[tokio::test]
async fn close_releases_the_session_and_its_stores_while_a_scan_never_returns() {
    let entered = Arc::new(Notify::new());
    let source = Arc::new(ProbeStore::new(5).blocking(&entered));
    let target = Arc::new(ProbeStore::new(5));
    let (source_weak, target_weak) = (Arc::downgrade(&source), Arc::downgrade(&target));
    let session = direct_session(source, target, Limits::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    runner::start(
        tx,
        Arc::new(|| {}),
        session.clone(),
        1,
        1,
        vec![same_pair(1)],
    )
    .unwrap();
    entered.notified().await;
    let session_weak = Arc::downgrade(&session);
    session.close();
    drop(session);
    assert!(
        released(&session_weak).await,
        "the run still holds the session"
    );
    assert!(
        released(&source_weak).await,
        "the source store is still alive"
    );
    assert!(
        released(&target_weak).await,
        "the target store is still alive"
    );
}

#[tokio::test]
async fn an_unindexed_side_above_the_limit_fails_the_pair_before_scanning() {
    let limits = Limits {
        max_unindexed_documents: 10,
        ..Limits::default()
    };
    let entered = Arc::new(Notify::new());
    // A scan would block forever: the pair must fail without one.
    let source = Arc::new(ProbeStore::new(5).blocking(&entered));
    let target = Arc::new(ProbeStore::new(5).unindexed(11));
    let session = direct_session(source, target, limits);
    let outcomes = tokio::time::timeout(
        Duration::from_secs(2),
        run_direct(&session, vec![same_pair(1)]),
    )
    .await
    .expect("no scan is attempted");
    assert!(
        matches!(
            outcomes[..],
            [PairOutcome::Failed(CoreError::CompareKeyNotIndexed)]
        ),
        "{outcomes:?}"
    );
}

#[tokio::test]
async fn an_unindexed_side_within_the_limit_still_runs() {
    let limits = Limits {
        max_unindexed_documents: 10,
        ..Limits::default()
    };
    let source = Arc::new(ProbeStore::new(5).unindexed(10));
    let session = direct_session(source, Arc::new(ProbeStore::new(5)), limits);
    let outcomes = run_direct(&session, vec![same_pair(1)]).await;
    assert!(
        matches!(outcomes[..], [PairOutcome::Completed(_)]),
        "{outcomes:?}"
    );
}

fn invalid_pairs() -> Vec<PairSpec> {
    let mut empty_key = same_pair(2);
    empty_key.key = KeySpec { fields: Vec::new() };
    let mut repeated_key = same_pair(3);
    repeated_key.key = KeySpec {
        fields: vec!["a".into(), "a".into()],
    };
    let mut bad_rules = same_pair(4);
    bad_rules.rules.field_scope = FieldScope::Only(Vec::new());
    vec![empty_key, repeated_key, bad_rules]
}

#[tokio::test]
async fn start_refuses_an_invalid_pair_spec_with_the_typed_error() {
    let session = direct_session(
        Arc::new(ProbeStore::new(1)),
        Arc::new(ProbeStore::new(1)),
        Limits::default(),
    );
    for bad in invalid_pairs() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let started = runner::start(
            tx,
            Arc::new(|| {}),
            session.clone(),
            1,
            1,
            vec![same_pair(1), bad.clone()],
        );
        assert_eq!(started, Err(CoreError::CompareSetupInvalid), "{bad:?}");
    }
}

#[tokio::test]
async fn check_and_start_answer_an_invalid_pair_spec_with_failed() {
    let (tx, mut h) = rig(ProbeStore::new(1), ProbeStore::new(1));
    open(&mut h, &tx, 1).await;
    for bad in invalid_pairs() {
        let check = CompareCommand::Check {
            session: 1,
            pair: bad.clone(),
        };
        tx.send(Command::Compare(check)).unwrap();
        match next_compare(&mut h).await {
            CompareEvent::Failed { context, error, .. } => {
                assert_eq!(
                    (context, error),
                    ("compare_check", CoreError::CompareSetupInvalid)
                );
            }
            other => panic!("{other:?}"),
        }
        send_start(&tx, 1, vec![bad]);
        match next_compare(&mut h).await {
            CompareEvent::Failed { context, error, .. } => {
                assert_eq!(
                    (context, error),
                    ("compare_start", CoreError::CompareSetupInvalid)
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
