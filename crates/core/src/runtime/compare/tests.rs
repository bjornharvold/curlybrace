//! Compare sessions against in-memory stores: no server, no network.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bson::doc;

use crate::compare::key::KeySpec;
use crate::compare::protocol::{CompareCommand, CompareEvent, PairOutcome, PairSpec, Side};
use crate::compare::results::Category;
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

pub(super) async fn collect_run(h: &mut RuntimeHandle) -> Vec<(u32, PairOutcome)> {
    let mut done = Vec::new();
    loop {
        match next_compare(h).await {
            CompareEvent::PairDone { pair, outcome, .. } => done.push((pair, outcome)),
            CompareEvent::RunDone { .. } => return done,
            CompareEvent::Progress { .. } => {}
            other => panic!("unexpected {other:?} after {} pair events", done.len()),
        }
    }
}

pub(super) type Tx = tokio::sync::mpsc::UnboundedSender<Command>;

pub(super) fn send_start(tx: &Tx, run: u64, pairs: Vec<PairSpec>) {
    let start = CompareCommand::Start {
        session: 1,
        run,
        pairs,
    };
    tx.send(Command::Compare(start)).unwrap();
}

pub(super) fn send_rows(tx: &Tx, pair: u32, category: Category) {
    let rows = CompareCommand::Rows {
        session: 1,
        pair,
        category,
        offset: 0,
        limit: 10,
    };
    tx.send(Command::Compare(rows)).unwrap();
}

pub(super) fn same_pair(id: u32) -> PairSpec {
    pair(id, ns("d", "c"), ns("d", "c"))
}

pub(super) async fn expect_failed(h: &mut RuntimeHandle, expected: &str) {
    match next_compare(h).await {
        CompareEvent::Failed { context, .. } => assert_eq!(context, expected),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_run_reports_one_terminal_per_pair_then_the_run_terminal() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    send_start(&tx, 10, vec![same_pair(1), same_pair(2)]);
    let done = collect_run(&mut h).await;
    assert_eq!(done.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [1, 2]);
    for (_, outcome) in &done {
        match outcome {
            PairOutcome::Completed(s) => {
                assert_eq!((s.equal, s.only_in_source, s.only_in_target), (3, 2, 2))
            }
            other => panic!("{other:?}"),
        }
    }
    let late = tokio::time::timeout(Duration::from_millis(100), h.rx.recv()).await;
    assert!(late.is_err(), "nothing follows RunDone: {late:?}");
}

#[tokio::test]
async fn duplicate_pair_ids_in_one_run_are_refused() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    send_start(&tx, 10, vec![same_pair(1), same_pair(1)]);
    expect_failed(&mut h, "compare_start").await;
}

#[tokio::test]
async fn one_failing_pair_does_not_stop_the_others_and_stores_nothing() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    // The second pair keys on a field no document has.
    let mut bad = same_pair(2);
    bad.key = KeySpec::new(vec!["nope".into()]).unwrap();
    bad.rules = CompareRules::for_key(&bad.key);
    send_start(&tx, 10, vec![same_pair(1), bad, same_pair(3)]);
    let done = collect_run(&mut h).await;
    assert!(matches!(done[0].1, PairOutcome::Completed(_)));
    assert!(matches!(
        done[1].1,
        PairOutcome::Failed(CoreError::CompareKeyMissing)
    ));
    assert!(matches!(done[2].1, PairOutcome::Completed(_)));
    send_rows(&tx, 2, Category::OnlyInSource);
    expect_failed(&mut h, "compare_rows").await;
}

#[tokio::test]
async fn rows_page_the_differences_and_detail_shows_the_field_changes() {
    let docs = |v: Vec<bson::Document>| InMemoryStore::new().with_docs(&ns("d", "c"), v);
    let a = docs(vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 2 }]);
    let b = docs(vec![doc! { "_id": 1, "v": 9 }, doc! { "_id": 3, "v": 3 }]);
    let (tx, mut h) = start(vec![("mongodb://a", a), ("mongodb://b", b)]);
    open(&mut h, &tx, 1).await;
    send_start(&tx, 1, vec![same_pair(1)]);
    collect_run(&mut h).await;
    send_rows(&tx, 1, Category::Different);
    let key = match next_compare(&mut h).await {
        CompareEvent::Rows { total: 1, rows, .. } => {
            assert!(rows[0].has_source && rows[0].has_target);
            rows[0].key.clone()
        }
        other => panic!("{other:?}"),
    };
    let detail = CompareCommand::Detail {
        session: 1,
        pair: 1,
        key,
    };
    tx.send(Command::Compare(detail)).unwrap();
    match next_compare(&mut h).await {
        CompareEvent::Detail {
            changes,
            source,
            target,
            ..
        } => {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].path, "v");
            assert!(source.is_some() && target.is_some());
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn detail_of_a_one_sided_row_has_only_that_document_and_unknown_pairs_fail() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    send_start(&tx, 1, vec![same_pair(1)]);
    collect_run(&mut h).await;
    let key = crate::compare::key::KeyTuple(vec![bson::Bson::Int32(0)]);
    let ask = |pair| {
        let detail = CompareCommand::Detail {
            session: 1,
            pair,
            key: key.clone(),
        };
        tx.send(Command::Compare(detail)).unwrap();
    };
    ask(1);
    match next_compare(&mut h).await {
        CompareEvent::Detail {
            source,
            target,
            changes,
            ..
        } => assert!(source.is_some() && target.is_none() && changes.is_empty()),
        other => panic!("{other:?}"),
    }
    ask(9);
    expect_failed(&mut h, "compare_detail").await;
}

#[tokio::test]
async fn discard_makes_rows_fail() {
    let (tx, mut h) = start(seeded());
    open(&mut h, &tx, 1).await;
    send_start(&tx, 1, vec![same_pair(1)]);
    collect_run(&mut h).await;
    let discard = CompareCommand::Discard {
        session: 1,
        pair: 1,
    };
    tx.send(Command::Compare(discard)).unwrap();
    send_rows(&tx, 1, Category::Different);
    expect_failed(&mut h, "compare_rows").await;
}
