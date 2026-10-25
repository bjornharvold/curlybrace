//! The index commands and events, the history they read and the sampler a connect starts.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::TempDir;
use uuid::Uuid;

use super::sampler::{SamplerConfig, SamplerStatus};
use super::*;
use crate::clock;
use crate::history::{ChartRange, HistoryDb, Sample, SeriesKey};
use crate::indexes::KeyKind;
use crate::store::{IndexUsage, Page, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc, mock_index_spec, mock_index_spec_named};
use bson::Bson;

const WAIT: Duration = Duration::from_secs(5);
const HOST: &str = "h1:27017";

fn ns(db: &str) -> Namespace {
    Namespace::new(db, "people")
}

fn usage(ops: i64) -> IndexUsage {
    IndexUsage {
        name: "a_1".into(),
        host: HOST.into(),
        ops,
        since_ms: 1_000,
        spec: mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]),
    }
}

/// A store with one document and one scripted index in each of `dbs`.
fn store_with(dbs: &[&str]) -> Arc<InMemoryStore> {
    Arc::new(dbs.iter().fold(InMemoryStore::new(), |s, db| {
        s.with_docs(&ns(db), vec![mock_doc()])
            .with_index_usage(&ns(db), vec![usage(5)])
    }))
}

fn key_in(connection: Uuid, db: &str) -> SeriesKey {
    SeriesKey {
        connection,
        host: HOST.into(),
        db: db.into(),
        collection: "people".into(),
        index_name: "a_1".into(),
        definition_hash: mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]).definition_hash(),
    }
}

fn config_at(dir: &TempDir) -> RuntimeConfig {
    RuntimeConfig {
        history_path: Some(dir.path().join("history.sqlite")),
        sampler: SamplerConfig {
            interval: Duration::from_secs(1),
            ..SamplerConfig::default()
        },
        ..RuntimeConfig::default()
    }
}

fn open_history(path: &Path) -> HistoryDb {
    HistoryDb::open(path).expect("history opens").0
}

/// Hands out the same store for every connect.
struct FixedConnector(Arc<InMemoryStore>);

#[async_trait]
impl Connector for FixedConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

fn spawn_connecting(store: Arc<InMemoryStore>, config: RuntimeConfig) -> RuntimeHandle {
    spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(store)),
        Arc::new(|| {}),
        config,
    )
}

fn spawn_embedded(store: Arc<InMemoryStore>, config: RuntimeConfig) -> RuntimeHandle {
    spawn_runtime_with_config(None, Some(store), Arc::new(|| {}), config)
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connect(h: &mut RuntimeHandle, cfg: &ConnectionConfig) {
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(h).await, Event::Connected { .. }));
}

async fn expect_sampler(h: &mut RuntimeHandle, session: SessionId, status: SamplerStatus) {
    assert_eq!(next(h).await, Event::Sampler { session, status });
}

fn conn_config(read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    }
}

#[tokio::test]
async fn list_indexes_answers_with_the_collections_indexes() {
    let mut h = spawn_embedded(store_with(&["testdb"]), RuntimeConfig::default());
    h.tx.send(Command::ListIndexes {
        session: EMBEDDED_SESSION,
        ns: ns("testdb"),
        request: 0,
    })
    .unwrap();
    let Event::Indexes {
        session,
        ns: answered,
        indexes,
        ..
    } = next(&mut h).await
    else {
        panic!("expected Indexes");
    };
    assert_eq!((session, answered), (EMBEDDED_SESSION, ns("testdb")));
    let names: Vec<_> = indexes.iter().map(|i| i.spec.name.as_str()).collect();
    assert_eq!(names, vec!["_id_"]);
}

#[tokio::test]
async fn estimated_count_answers_with_the_collections_document_count() {
    let mut h = spawn_embedded(store_with(&["testdb"]), RuntimeConfig::default());
    let command = Command::EstimatedCount {
        session: EMBEDDED_SESSION,
        ns: ns("testdb"),
    };
    assert_eq!(
        (command.context(), command.is_write(), command.session()),
        ("estimated_count", false, Some(EMBEDDED_SESSION))
    );
    h.tx.send(command).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::EstimatedCount {
            session: EMBEDDED_SESSION,
            ns: ns("testdb"),
            count: 1,
        }
    );
}

#[tokio::test]
async fn index_history_answers_with_series_from_the_history_db() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let path = config.history_path.clone().unwrap();
    let key = key_in(EMBEDDED_SESSION, "testdb");
    let now = clock::now_secs();
    {
        let db = open_history(&path);
        for (back, ops) in [(180, 10), (120, 12), (60, 15)] {
            let sample = Sample {
                at_secs: now - back,
                ops,
                since_ms: 1_000,
            };
            db.record(&key, sample).unwrap();
        }
        // Another collection and another connection are not part of the answer.
        db.record(
            &key_in(EMBEDDED_SESSION, "other"),
            Sample {
                at_secs: now - 60,
                ops: 1,
                since_ms: 1_000,
            },
        )
        .unwrap();
    }
    let mut h = spawn_embedded(store_with_listed_index().await, config);
    h.tx.send(Command::IndexHistory {
        session: EMBEDDED_SESSION,
        ns: ns("testdb"),
        range: ChartRange::Day,
        request: 0,
    })
    .unwrap();
    let Event::IndexHistory {
        session,
        ns: answered,
        range,
        series,
        ..
    } = next(&mut h).await
    else {
        panic!("expected IndexHistory");
    };
    assert_eq!(
        (session, answered, range),
        (EMBEDDED_SESSION, ns("testdb"), ChartRange::Day)
    );
    assert_eq!(series.len(), 1);
    let one = &series[0];
    assert_eq!(one.index_name, "a_1");
    assert_eq!(one.definition_hash, key.definition_hash);
    assert_eq!(one.host, HOST);
    assert_eq!(one.buckets.iter().map(|b| b.uses).sum::<i64>(), 5);
    assert_eq!(one.latest.map(|s| s.ops), Some(15));
}

#[tokio::test]
async fn index_history_without_a_history_db_answers_with_no_series() {
    let mut h = spawn_embedded(store_with(&["testdb"]), RuntimeConfig::default());
    h.tx.send(Command::IndexHistory {
        session: EMBEDDED_SESSION,
        ns: ns("testdb"),
        range: ChartRange::Week,
        request: 0,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: ns("testdb"),
            range: ChartRange::Week,
            series: vec![],
            request: 0,
        }
    );
}

#[tokio::test]
async fn create_index_answers_index_changed_and_the_next_list_shows_it() {
    let mut h = spawn_embedded(store_with(&["testdb"]), RuntimeConfig::default());
    let spec = mock_index_spec();
    let target = ns("testdb");
    h.tx.send(Command::CreateIndex {
        session: EMBEDDED_SESSION,
        ns: target.clone(),
        spec: spec.clone(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::IndexChanged {
            session: EMBEDDED_SESSION,
            ns: target.clone()
        }
    );
    h.tx.send(Command::ListIndexes {
        session: EMBEDDED_SESSION,
        ns: target.clone(),
        request: 0,
    })
    .unwrap();
    let Event::Indexes { indexes, .. } = next(&mut h).await else {
        panic!("expected Indexes");
    };
    assert!(indexes.iter().any(|i| i.spec.name == spec.name));

    h.tx.send(Command::SetIndexHidden {
        session: EMBEDDED_SESSION,
        ns: target.clone(),
        name: spec.name.clone(),
        hidden: true,
    })
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::IndexChanged { .. }));
    h.tx.send(Command::SetIndexTtl {
        session: EMBEDDED_SESSION,
        ns: target.clone(),
        name: spec.name.clone(),
        seconds: 60,
    })
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::IndexChanged { .. }));
    h.tx.send(Command::DropIndex {
        session: EMBEDDED_SESSION,
        ns: target,
        name: spec.name,
    })
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::IndexChanged { .. }));
}

fn index_writes(session: SessionId) -> Vec<Command> {
    let spec = mock_index_spec();
    vec![
        Command::CreateIndex {
            session,
            ns: ns("testdb"),
            spec: spec.clone(),
        },
        Command::DropIndex {
            session,
            ns: ns("testdb"),
            name: spec.name.clone(),
        },
        Command::SetIndexHidden {
            session,
            ns: ns("testdb"),
            name: spec.name.clone(),
            hidden: true,
        },
        Command::SetIndexTtl {
            session,
            ns: ns("testdb"),
            name: spec.name,
            seconds: 60,
        },
    ]
}

#[tokio::test]
async fn a_read_only_session_refuses_all_four_index_writes_with_read_only() {
    let store = store_with(&["testdb"]);
    let mut h = spawn_connecting(store.clone(), RuntimeConfig::default());
    let cfg = conn_config(true);
    connect(&mut h, &cfg).await;
    for command in index_writes(cfg.id) {
        assert!(command.is_write());
        let context = command.context();
        h.tx.send(command).unwrap();
        assert_eq!(
            next(&mut h).await,
            Event::Failed {
                session: cfg.id,
                context,
                error: CoreError::ReadOnly
            }
        );
    }
    // Nothing reached the store, and listing is still allowed.
    h.tx.send(Command::ListIndexes {
        session: cfg.id,
        ns: ns("testdb"),
        request: 0,
    })
    .unwrap();
    let Event::Indexes { indexes, .. } = next(&mut h).await else {
        panic!("expected Indexes");
    };
    assert_eq!(indexes.len(), 1);
}

#[test]
fn the_index_commands_report_their_context_session_and_write_flag() {
    let session = Uuid::from_u128(9);
    let read = [
        (
            Command::ListIndexes {
                session,
                ns: ns("d"),
                request: 0,
            },
            "list_indexes",
        ),
        (
            Command::IndexHistory {
                session,
                ns: ns("d"),
                range: ChartRange::Day,
                request: 0,
            },
            "index_history",
        ),
    ];
    for (command, context) in read {
        assert_eq!(
            (command.context(), command.is_write(), command.session()),
            (context, false, Some(session))
        );
    }
    let contexts = [
        "create_index",
        "drop_index",
        "set_index_hidden",
        "set_index_ttl",
    ];
    for (command, context) in index_writes(session).iter().zip(contexts) {
        assert_eq!(
            (command.context(), command.is_write(), command.session()),
            (context, true, Some(session))
        );
    }
    let sampling = Command::SetSamplingExcluded {
        databases: Default::default(),
    };
    let purge = Command::PurgeHistory {
        connection: session,
    };
    for command in [sampling, purge] {
        assert_eq!((command.is_write(), command.session()), (false, None));
    }
}

#[tokio::test]
async fn every_command_still_yields_exactly_one_event_except_sampler_and_compare() {
    let mut h = spawn_embedded(store_with(&["testdb"]), RuntimeConfig::default());
    let session = EMBEDDED_SESSION;
    let mut commands = vec![
        Command::ListIndexes {
            session,
            ns: ns("testdb"),
            request: 0,
        },
        Command::IndexHistory {
            session,
            ns: ns("testdb"),
            range: ChartRange::Month,
            request: 0,
        },
        Command::EstimatedCount {
            session,
            ns: ns("testdb"),
        },
    ];
    commands.extend(index_writes(session));
    // Unknown session and failing commands answer once too.
    commands.push(Command::ListIndexes {
        session: Uuid::from_u128(77),
        ns: ns("testdb"),
        request: 0,
    });
    for command in commands {
        h.tx.send(command).unwrap();
        next(&mut h).await;
    }
    // These two have no answer of their own; a stray event would show up next.
    h.tx.send(Command::SetSamplingExcluded {
        databases: ["x".to_string()].into(),
    })
    .unwrap();
    h.tx.send(Command::PurgeHistory {
        connection: Uuid::from_u128(5),
    })
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(h.rx.try_recv().is_err(), "no event beyond one per command");
}

#[tokio::test]
async fn connecting_starts_the_sampler_and_disconnect_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let path = config.history_path.clone().unwrap();
    let store = store_with(&["testdb"]);
    let mut h = spawn_connecting(store.clone(), config);
    let cfg = conn_config(false);
    connect(&mut h, &cfg).await;
    expect_sampler(&mut h, cfg.id, SamplerStatus::Running).await;

    let key = key_in(cfg.id, "testdb");
    let reader = open_history(&path);
    assert_eq!(reader.latest(&key).unwrap().map(|s| s.ops), Some(5));

    h.tx.send(Command::Disconnect { session: cfg.id }).unwrap();
    assert!(matches!(next(&mut h).await, Event::Disconnected { .. }));
    store.set_index_usage(&ns("testdb"), vec![usage(50)]);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(reader.latest(&key).unwrap().map(|s| s.ops), Some(5));
    assert!(h.rx.try_recv().is_err());
}

#[tokio::test]
async fn a_newer_connect_replaces_the_sampler_of_the_older_one() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let store = store_with(&["testdb"]);
    let mut h = spawn_connecting(store, config);
    let cfg = conn_config(false);
    connect(&mut h, &cfg).await;
    expect_sampler(&mut h, cfg.id, SamplerStatus::Running).await;
    connect(&mut h, &cfg).await;
    // The new sampler reports from scratch; the old one is gone and reports nothing more.
    expect_sampler(&mut h, cfg.id, SamplerStatus::Running).await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(h.rx.try_recv().is_err());
}

#[tokio::test]
async fn without_a_history_path_connecting_starts_no_sampler() {
    let mut h = spawn_connecting(store_with(&["testdb"]), RuntimeConfig::default());
    connect(&mut h, &conn_config(false)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.rx.try_recv().is_err());
}

#[tokio::test]
async fn purge_history_removes_the_connections_rows() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let path = config.history_path.clone().unwrap();
    let (gone, kept) = (Uuid::from_u128(1), Uuid::from_u128(2));
    {
        let db = open_history(&path);
        for connection in [gone, kept] {
            let sample = Sample {
                at_secs: clock::now_secs() - 60,
                ops: 3,
                since_ms: 1_000,
            };
            db.record(&key_in(connection, "testdb"), sample).unwrap();
        }
    }
    let mut h = spawn_embedded(store_with(&["testdb"]), config);
    h.tx.send(Command::PurgeHistory { connection: gone })
        .unwrap();
    let reader = open_history(&path);
    let mut left = usize::MAX;
    for _ in 0..50 {
        left = reader.series_for(gone, "testdb", "people").unwrap().len();
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(left, 0);
    assert_eq!(
        reader.series_for(kept, "testdb", "people").unwrap().len(),
        1
    );
    assert!(h.rx.try_recv().is_err(), "a purge has no answer event");
}

#[tokio::test]
async fn set_sampling_excluded_reaches_a_running_sampler() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let path = config.history_path.clone().unwrap();
    let store = store_with(&["shop", "audit"]);
    let mut h = spawn_connecting(store.clone(), config);
    let cfg = conn_config(false);
    connect(&mut h, &cfg).await;
    expect_sampler(&mut h, cfg.id, SamplerStatus::Running).await;

    h.tx.send(Command::SetSamplingExcluded {
        databases: ["audit".to_string()].into(),
    })
    .unwrap();
    for db in ["shop", "audit"] {
        store.set_index_usage(&ns(db), vec![usage(50)]);
    }
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    let reader = open_history(&path);
    let latest = |db: &str| reader.latest(&key_in(cfg.id, db)).unwrap().map(|s| s.ops);
    assert_eq!(
        latest("shop"),
        Some(50),
        "an included database keeps sampling"
    );
    assert_eq!(latest("audit"), Some(5), "an excluded one stops");
}

/// Counts the passes of a sampler: one `list_databases` call per pass.
struct CountingStore {
    inner: Arc<InMemoryStore>,
    passes: AtomicUsize,
}

#[async_trait]
impl DocumentStore for CountingStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.passes.fetch_add(1, Ordering::SeqCst);
        self.inner.list_databases().await
    }
    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: bson::Document) -> Result<Bson> {
        self.inner.insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: bson::Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }
    async fn list_collection_kinds(
        &self,
        db: &str,
    ) -> Result<Vec<(String, crate::store::CollectionKind)>> {
        self.inner.list_collection_kinds(db).await
    }
    async fn index_stats(&self, ns: &Namespace) -> Result<Vec<IndexUsage>> {
        self.inner.index_stats(ns).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_zero_sampler_interval_is_clamped_to_one_second() {
    let store = Arc::new(CountingStore {
        inner: store_with(&["testdb"]),
        passes: AtomicUsize::new(0),
    });
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    let zero = SamplerConfig {
        interval: Duration::ZERO,
        ..SamplerConfig::default()
    };
    let (_cfg_tx, cfg_rx) = tokio::sync::watch::channel(zero);
    let (status_tx, _statuses) = tokio::sync::mpsc::unbounded_channel();
    let task = sampler::spawn(
        store.clone(),
        history,
        Uuid::from_u128(3),
        cfg_rx,
        status_tx,
    );
    tokio::time::sleep(Duration::from_secs(10)).await;
    task.abort();
    let passes = store.passes.load(Ordering::SeqCst);
    // One pass at once and one per second after: a tight loop would make thousands.
    assert!((10..=12).contains(&passes), "{passes} passes in 10 seconds");
}

#[tokio::test]
async fn dropping_the_runtime_stops_the_samplers() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let path = config.history_path.clone().unwrap();
    let store = store_with(&["testdb"]);
    let mut h = spawn_connecting(store.clone(), config);
    let cfg = conn_config(false);
    connect(&mut h, &cfg).await;
    expect_sampler(&mut h, cfg.id, SamplerStatus::Running).await;
    drop(h);
    // Let the forwarder notice the closed channel, then change what a surviving sampler would read.
    tokio::time::sleep(Duration::from_millis(300)).await;
    store.set_index_usage(&ns("testdb"), vec![usage(50)]);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let latest = open_history(&path).latest(&key_in(cfg.id, "testdb"));
    assert_eq!(
        latest.unwrap().map(|s| s.ops),
        Some(5),
        "a sampler survived"
    );
}

/// A `Shared` with an in-memory history, for driving `start_sampler` by hand.
fn shared_with_history() -> Arc<Shared> {
    Arc::new(Shared {
        sessions: std::sync::RwLock::new(Default::default()),
        next_epoch: std::sync::atomic::AtomicU64::new(0),
        secrets: None,
        connector: Arc::new(FixedConnector(store_with(&[]))),
        config: RuntimeConfig::default(),
        compare: Default::default(),
        copy: Default::default(),
        latest_test: Default::default(),
        read_only: Default::default(),
        history: Some(Arc::new(HistoryDb::open_in_memory().unwrap())),
        sampler_config: tokio::sync::watch::channel(SamplerConfig::default().clamped()).0,
    })
}

// Provokes the window deterministically: the test holds the epoch lock for write, as `begin` does
// while it moves the generation, while the sampler produces its first status. Before the fix the
// forwarder passed its currentness check and sent anyway; now it waits for the lock and drops the
// status once the generation has moved. Blocking the lock needs worker threads to spare.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock, clippy::readonly_write_lock)]
async fn a_sampler_status_never_follows_disconnected() {
    let shared = shared_with_history();
    let id = Uuid::from_u128(11);
    let (session, generation) = shared.begin_session_epoch(id);
    assert!(session.install_if_current(generation, store_with(&["testdb"]), None));
    let (events, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let begin_in_progress = session.epoch_lock.write().unwrap();
    indexes::start_sampler(&shared, &session, id, generation, events, Arc::new(|| {}));
    // The sampler's first pass completes and its status reaches the forwarder meanwhile.
    tokio::time::sleep(Duration::from_millis(500)).await;
    session.move_generation_for_tests(generation + 1);
    drop(begin_in_progress);

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(rx.try_recv().is_err(), "a status followed the epoch change");
}

#[tokio::test]
async fn a_history_that_cannot_be_opened_is_reported_on_connect() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"a file, not a directory").unwrap();
    let config = RuntimeConfig {
        history_path: Some(blocker.join("history.sqlite")),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_connecting(store_with(&["testdb"]), config);
    let cfg = conn_config(false);
    connect(&mut h, &cfg).await;
    expect_sampler(
        &mut h,
        cfg.id,
        SamplerStatus::Paused(SamplerPause::HistoryUnavailable),
    )
    .await;
}

/// A store whose `testdb.people` really has the `a_1` index that `usage` reports on.
async fn store_with_listed_index() -> Arc<InMemoryStore> {
    let store = store_with(&["testdb"]);
    let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
    store.create_index(&ns("testdb"), &spec).await.unwrap();
    store
}

#[tokio::test]
async fn index_history_only_answers_for_the_indexes_currently_listed() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_at(&dir);
    let path = config.history_path.clone().unwrap();
    let now = clock::now_secs();
    let sample = Sample {
        at_secs: now - 60,
        ops: 3,
        since_ms: 1_000,
    };
    let listed = key_in(EMBEDDED_SESSION, "testdb");
    let other_host = SeriesKey {
        host: "h2:27017".into(),
        ..listed.clone()
    };
    let recreated = SeriesKey {
        definition_hash: "an-older-definition".into(),
        ..listed.clone()
    };
    let dropped = SeriesKey {
        index_name: "gone_1".into(),
        ..listed.clone()
    };
    {
        let db = open_history(&path);
        for key in [&listed, &other_host, &recreated, &dropped] {
            db.record(key, sample).unwrap();
        }
    }
    let mut h = spawn_embedded(store_with_listed_index().await, config);
    h.tx.send(Command::IndexHistory {
        session: EMBEDDED_SESSION,
        ns: ns("testdb"),
        range: ChartRange::Day,
        request: 0,
    })
    .unwrap();
    let Event::IndexHistory { series, .. } = next(&mut h).await else {
        panic!("expected IndexHistory");
    };
    let mut seen: Vec<(String, String, String)> = series
        .into_iter()
        .map(|s| (s.index_name, s.definition_hash, s.host))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("a_1".into(), listed.definition_hash.clone(), HOST.into()),
            ("a_1".into(), listed.definition_hash, "h2:27017".into()),
        ]
    );
}
