//! A closed session emits nothing more; panics and slow work are contained; preflight limits.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::{Notify, mpsc::UnboundedSender};

use super::tests::{MapConnector, cfg, next_compare, ns, pair, seeded, start_with};
use crate::compare::key::KeySpec;
use crate::compare::protocol::{CompareCommand, CompareEvent, Side};
use crate::compare::results::Limits;
use crate::compare::rules::CompareRules;
use crate::error::Result;
use crate::runtime::{Command, RuntimeConfig, RuntimeHandle};
use crate::store::{CollectionKind, DocumentStore, IndexStatus, Namespace, Page, Query};
use crate::test_support::InMemoryStore;

/// A rendezvous: the parked side announces itself, the test releases it.
#[derive(Default)]
pub(super) struct Hooks {
    pub(super) entered: Notify,
    pub(super) release: Notify,
}

impl Hooks {
    pub(super) async fn park(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}

/// An in-memory store whose listing/counting calls can park or panic.
struct ProbeStore {
    inner: InMemoryStore,
    hooks: Option<Arc<Hooks>>,
    panics: bool,
    index: IndexStatus,
}

impl ProbeStore {
    fn new(inner: InMemoryStore) -> Self {
        Self {
            inner,
            hooks: None,
            panics: false,
            index: IndexStatus::Indexed,
        }
    }

    fn parked(self, hooks: &Arc<Hooks>) -> Self {
        Self {
            hooks: Some(hooks.clone()),
            ..self
        }
    }

    fn panicking(self) -> Self {
        Self {
            panics: true,
            ..self
        }
    }

    fn unindexed(self) -> Self {
        Self {
            index: IndexStatus::NotIndexed,
            ..self
        }
    }

    async fn gate(&self) {
        assert!(!self.panics, "store blew up");
        if let Some(hooks) = &self.hooks {
            hooks.park().await;
        }
    }
}

#[async_trait]
impl DocumentStore for ProbeStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.gate().await;
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
        self.gate().await;
        self.inner.estimated_count(ns).await
    }
    async fn key_index_status(&self, _: &Namespace, _: &KeySpec) -> Result<IndexStatus> {
        Ok(self.index)
    }
    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        self.gate().await;
        self.inner.list_collection_kinds(db).await
    }
}

type Tx = UnboundedSender<Command>;
type Stores = Vec<(&'static str, Arc<dyn DocumentStore>)>;

fn plain() -> Stores {
    seeded()
        .into_iter()
        .map(|(uri, s)| (uri, Arc::new(s) as Arc<dyn DocumentStore>))
        .collect()
}

fn rig(stores: Stores, connector: MapConnector, config: RuntimeConfig) -> (Tx, RuntimeHandle) {
    let stores = stores
        .into_iter()
        .map(|(u, s)| (u.to_string(), s))
        .collect();
    start_with(
        MapConnector {
            stores,
            ..connector
        },
        config,
    )
}

fn send(tx: &Tx, command: CompareCommand) {
    tx.send(Command::Compare(command)).unwrap();
}

async fn open_pair(h: &mut RuntimeHandle, tx: &Tx, session: u64, source: &str, target: &str) {
    send(
        tx,
        CompareCommand::Open {
            session,
            source: cfg(source),
            target: cfg(target),
        },
    );
    assert_eq!(next_compare(h).await, CompareEvent::Opened { session });
}

/// A fast command on another open session: its answer is the first event, and nothing follows.
async fn assert_only_sentinel(h: &mut RuntimeHandle, tx: &Tx) {
    send(
        tx,
        CompareCommand::Databases {
            session: 2,
            side: Side::Source,
        },
    );
    assert!(matches!(
        next_compare(h).await,
        CompareEvent::Databases { session: 2, .. }
    ));
    assert_silent(h).await;
}

async fn assert_silent(h: &mut RuntimeHandle) {
    let late = tokio::time::timeout(Duration::from_millis(200), h.rx.recv()).await;
    assert!(late.is_err(), "a closed session stays silent: {late:?}");
}

fn slow_source(hooks: &Arc<Hooks>) -> Stores {
    let mut stores = plain();
    let (_, a) = seeded().into_iter().next().unwrap();
    stores.push(("mongodb://slow", Arc::new(ProbeStore::new(a).parked(hooks))));
    stores
}

async fn close_while_parked(command: CompareCommand) {
    let hooks = Arc::new(Hooks::default());
    let (tx, mut h) = rig(
        slow_source(&hooks),
        MapConnector::default(),
        RuntimeConfig::default(),
    );
    open_pair(&mut h, &tx, 1, "mongodb://slow", "mongodb://b").await;
    open_pair(&mut h, &tx, 2, "mongodb://a", "mongodb://b").await;
    send(&tx, command);
    hooks.entered.notified().await;
    send(&tx, CompareCommand::Close { session: 1 });
    hooks.release.notify_one();
    assert_only_sentinel(&mut h, &tx).await;
}

#[tokio::test]
async fn a_database_listing_finished_after_close_is_not_delivered() {
    close_while_parked(CompareCommand::Databases {
        session: 1,
        side: Side::Source,
    })
    .await;
}

#[tokio::test]
async fn a_collection_listing_finished_after_close_is_not_delivered() {
    let command = CompareCommand::Collections {
        session: 1,
        side: Side::Source,
        db: "d".into(),
    };
    close_while_parked(command).await;
}

#[tokio::test]
async fn a_check_finished_after_close_is_not_delivered() {
    let command = CompareCommand::Check {
        session: 1,
        pair: pair(1, ns("d", "c"), ns("d", "c")),
    };
    close_while_parked(command).await;
}

async fn close_while_connecting(target_uri: &'static str) {
    let hooks = Arc::new(Hooks::default());
    let connector = MapConnector {
        parked: HashMap::from([(target_uri.to_string(), hooks.clone())]),
        ..MapConnector::default()
    };
    let (tx, mut h) = rig(with_slow_target(), connector, RuntimeConfig::default());
    send(
        &tx,
        CompareCommand::Open {
            session: 1,
            source: cfg("mongodb://a"),
            target: cfg(target_uri),
        },
    );
    hooks.entered.notified().await;
    send(&tx, CompareCommand::Close { session: 1 });
    hooks.release.notify_one();
    open_pair(&mut h, &tx, 2, "mongodb://a", "mongodb://b").await;
    assert_silent(&mut h).await;
}

#[tokio::test]
async fn an_open_that_succeeds_after_close_reports_nothing() {
    close_while_connecting("mongodb://slow").await;
}

#[tokio::test]
async fn an_open_that_fails_after_close_reports_nothing() {
    close_while_connecting("mongodb://missing").await;
}

#[tokio::test]
async fn a_panicking_store_ends_in_the_commands_failed_event() {
    let (_, a) = seeded().into_iter().next().unwrap();
    let mut stores = plain();
    stores.push(("mongodb://boom", Arc::new(ProbeStore::new(a).panicking())));
    let (tx, mut h) = rig(stores, MapConnector::default(), RuntimeConfig::default());
    open_pair(&mut h, &tx, 1, "mongodb://boom", "mongodb://b").await;
    send(
        &tx,
        CompareCommand::Databases {
            session: 1,
            side: Side::Source,
        },
    );
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 1,
            context: "compare_databases",
            ..
        }
    ));
}

#[tokio::test]
async fn a_panicking_connector_ends_in_open_failed_and_drops_the_session() {
    let connector = MapConnector {
        panic_uri: Some("mongodb://boom".into()),
        ..MapConnector::default()
    };
    let (tx, mut h) = rig(plain(), connector, RuntimeConfig::default());
    send(
        &tx,
        CompareCommand::Open {
            session: 1,
            source: cfg("mongodb://a"),
            target: cfg("mongodb://boom"),
        },
    );
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::OpenFailed { session: 1, .. }
    ));
    send(
        &tx,
        CompareCommand::Databases {
            session: 1,
            side: Side::Source,
        },
    );
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 1,
            context: "compare_databases",
            ..
        }
    ));
}

/// Docs 5 per side, an unindexed `k` key, and a cap of 3 unindexed documents.
async fn preflight_unindexed(source_unindexed: bool, target_unindexed: bool, docs: i32) -> bool {
    let make = |unindexed: bool| {
        let inner = InMemoryStore::new().with_docs(
            &ns("d", "c"),
            (0..docs).map(|n| doc! { "_id": n, "k": n }).collect(),
        );
        let store = ProbeStore::new(inner);
        Arc::new(if unindexed { store.unindexed() } else { store }) as Arc<dyn DocumentStore>
    };
    let stores: Stores = vec![
        ("mongodb://a", make(source_unindexed)),
        ("mongodb://b", make(target_unindexed)),
    ];
    let config = RuntimeConfig {
        compare_limits: Limits {
            max_unindexed_documents: 3,
            ..Limits::default()
        },
        ..RuntimeConfig::default()
    };
    let (tx, mut h) = rig(stores, MapConnector::default(), config);
    open_pair(&mut h, &tx, 1, "mongodb://a", "mongodb://b").await;
    let key = KeySpec::new(vec!["k".to_string()]).unwrap();
    let mut spec = pair(1, ns("d", "c"), ns("d", "c"));
    spec.rules = CompareRules::for_key(&key);
    spec.key = key;
    send(
        &tx,
        CompareCommand::Check {
            session: 1,
            pair: spec,
        },
    );
    match next_compare(&mut h).await {
        CompareEvent::Checked { preflight, .. } => preflight.unindexed_too_large,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_large_unindexed_source_is_flagged() {
    assert!(preflight_unindexed(true, false, 5).await);
}

#[tokio::test]
async fn a_large_unindexed_target_is_flagged() {
    assert!(preflight_unindexed(false, true, 5).await);
}

#[tokio::test]
async fn a_small_unindexed_collection_is_not_flagged() {
    assert!(!preflight_unindexed(true, true, 2).await);
}

#[tokio::test]
async fn a_large_but_indexed_collection_is_not_flagged() {
    assert!(!preflight_unindexed(false, false, 5).await);
}

/// The plain servers plus a `mongodb://slow` twin of the target.
fn with_slow_target() -> Stores {
    let mut stores = plain();
    let (_, b) = seeded().into_iter().nth(1).unwrap();
    stores.push(("mongodb://slow", Arc::new(b)));
    stores
}
