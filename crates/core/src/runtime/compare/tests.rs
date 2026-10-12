//! Compare sessions against in-memory stores: no server, no network.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bson::doc;

use crate::compare::key::KeySpec;
use crate::compare::protocol::{CompareCommand, CompareEvent, PairSpec, Side};
use crate::compare::rules::CompareRules;
use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};
use crate::runtime::{
    Command, Connector, Event, RuntimeConfig, RuntimeHandle, spawn_runtime_with_connector,
};
use crate::store::{CollectionKind, DocumentStore, IndexStatus, Namespace};
use crate::test_support::InMemoryStore;

/// Hands out the in-memory store registered for the connection's URI.
#[derive(Default)]
pub(super) struct MapConnector {
    pub stores: HashMap<String, Arc<dyn DocumentStore>>,
    /// Connecting to these URIs parks until the test releases it.
    pub parked: HashMap<String, Arc<super::closed_tests::Hooks>>,
    /// Connecting to this URI panics.
    pub panic_uri: Option<String>,
}

#[async_trait::async_trait]
impl Connector for MapConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        if self.panic_uri.as_deref() == Some(cfg.uri.as_str()) {
            panic!("connector blew up");
        }
        if let Some(hooks) = self.parked.get(&cfg.uri) {
            hooks.park().await;
        }
        self.stores
            .get(&cfg.uri)
            .cloned()
            .ok_or(CoreError::Database("no such server".into()))
    }
}

pub(super) fn cfg(uri: &str) -> ConnectionConfig {
    ConnectionConfig::new(uri, uri)
}

pub(super) fn ns(db: &str, c: &str) -> Namespace {
    Namespace::new(db, c)
}

pub(super) fn pair(id: u32, source: Namespace, target: Namespace) -> PairSpec {
    PairSpec {
        id,
        source,
        target,
        source_filter: doc! {},
        target_filter: doc! {},
        key: KeySpec::id(),
        rules: CompareRules::for_key(&KeySpec::id()),
    }
}

pub(super) fn start(
    servers: Vec<(&str, InMemoryStore)>,
) -> (tokio::sync::mpsc::UnboundedSender<Command>, RuntimeHandle) {
    let stores = servers
        .into_iter()
        .map(|(uri, store)| (uri.to_string(), Arc::new(store) as Arc<dyn DocumentStore>))
        .collect();
    start_with(
        MapConnector {
            stores,
            ..MapConnector::default()
        },
        RuntimeConfig::default(),
    )
}

pub(super) fn start_with(
    connector: MapConnector,
    config: RuntimeConfig,
) -> (tokio::sync::mpsc::UnboundedSender<Command>, RuntimeHandle) {
    let handle =
        spawn_runtime_with_connector(None, None, Arc::new(connector), Arc::new(|| {}), config);
    (handle.tx.clone(), handle)
}

pub(super) async fn next_compare(h: &mut RuntimeHandle) -> CompareEvent {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), h.rx.recv())
            .await
            .expect("an event arrives")
            .expect("the channel is open");
        if let Event::Compare(e) = event {
            return e;
        }
    }
}

pub(super) fn seeded() -> Vec<(&'static str, InMemoryStore)> {
    let a =
        InMemoryStore::new().with_docs(&ns("d", "c"), (0..5).map(|n| doc! { "_id": n }).collect());
    let b =
        InMemoryStore::new().with_docs(&ns("d", "c"), (2..7).map(|n| doc! { "_id": n }).collect());
    vec![("mongodb://a", a), ("mongodb://b", b)]
}

pub(super) async fn open(
    h: &mut RuntimeHandle,
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    session: u64,
) {
    tx.send(Command::Compare(CompareCommand::Open {
        session,
        source: cfg("mongodb://a"),
        target: cfg("mongodb://b"),
    }))
    .unwrap();
    assert_eq!(next_compare(h).await, CompareEvent::Opened { session });
}

#[tokio::test]
async fn open_connects_both_sides_and_answers_once() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
}

#[tokio::test]
async fn open_with_a_failing_side_reports_open_failed_and_leaves_no_session() {
    let (tx, mut h) = start(seeded());
    tx.send(Command::Compare(CompareCommand::Open {
        session: 1,
        source: cfg("mongodb://a"),
        target: cfg("mongodb://missing"),
    }))
    .unwrap();
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::OpenFailed { session: 1, .. }
    ));
    tx.send(Command::Compare(CompareCommand::Databases {
        session: 1,
        side: Side::Source,
    }))
    .unwrap();
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
async fn listing_uses_the_chosen_side() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    tx.send(Command::Compare(CompareCommand::Collections {
        session: 1,
        side: Side::Target,
        db: "d".into(),
    }))
    .unwrap();
    match next_compare(&mut h).await {
        CompareEvent::Collections {
            session: 1,
            side: Side::Target,
            db,
            entries,
        } => {
            assert_eq!(db, "d");
            assert_eq!(entries, vec![("c".to_string(), CollectionKind::Collection)]);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn check_reports_counts_and_index_status() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    tx.send(Command::Compare(CompareCommand::Check {
        session: 1,
        pair: pair(7, ns("d", "c"), ns("d", "c")),
    }))
    .unwrap();
    match next_compare(&mut h).await {
        CompareEvent::Checked {
            session: 1,
            pair: 7,
            preflight,
        } => {
            assert_eq!((preflight.source_count, preflight.target_count), (5, 5));
            assert_eq!(preflight.source_index, IndexStatus::Indexed);
            assert!(!preflight.unindexed_too_large);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn commands_for_a_closed_or_unknown_session_fail_with_their_context() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    tx.send(Command::Compare(CompareCommand::Check {
        session: 1,
        pair: pair(1, ns("d", "c"), ns("d", "c")),
    }))
    .unwrap();
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 1,
            context: "compare_check",
            ..
        }
    ));
    // A session id that was never opened fails the same way.
    tx.send(Command::Compare(CompareCommand::Databases {
        session: 99,
        side: Side::Target,
    }))
    .unwrap();
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 99,
            context: "compare_databases",
            ..
        }
    ));
}

#[tokio::test]
async fn the_main_connection_is_untouched_by_compare_sessions() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    tx.send(Command::ListDatabases).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), h.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            event,
            Event::Failed {
                context: "list_databases",
                ..
            }
        ),
        "not connected is still not connected: {event:?}"
    );
}

#[tokio::test]
async fn an_open_followed_by_a_close_never_reports_opened() {
    let (tx, mut h) = start(seeded());
    tx.send(Command::Compare(CompareCommand::Open {
        session: 1,
        source: cfg("mongodb://a"),
        target: cfg("mongodb://b"),
    }))
    .unwrap();
    tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    tx.send(Command::Compare(CompareCommand::Databases {
        session: 1,
        side: Side::Source,
    }))
    .unwrap();
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 1,
            context: "compare_databases",
            ..
        }
    ));
    let late = tokio::time::timeout(Duration::from_millis(200), h.rx.recv()).await;
    assert!(late.is_err(), "a closed session stays silent: {late:?}");
}
