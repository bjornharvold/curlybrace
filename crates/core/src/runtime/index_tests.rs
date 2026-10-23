//! The index commands and events, the history they read and the sampler a connect starts.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tempfile::TempDir;
use uuid::Uuid;

use super::sampler::{SamplerConfig, SamplerStatus};
use super::*;
use crate::clock;
use crate::history::{ChartRange, HistoryDb, Sample, SeriesKey};
use crate::indexes::KeyKind;
use crate::store::IndexUsage;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc, mock_index_spec, mock_index_spec_named};

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
    })
    .unwrap();
    let Event::Indexes {
        session,
        ns: answered,
        indexes,
    } = next(&mut h).await
    else {
        panic!("expected Indexes");
    };
    assert_eq!((session, answered), (EMBEDDED_SESSION, ns("testdb")));
    let names: Vec<_> = indexes.iter().map(|i| i.spec.name.as_str()).collect();
    assert_eq!(names, vec!["_id_"]);
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
    let mut h = spawn_embedded(store_with(&["testdb"]), config);
    h.tx.send(Command::IndexHistory {
        session: EMBEDDED_SESSION,
        ns: ns("testdb"),
        range: ChartRange::Day,
    })
    .unwrap();
    let Event::IndexHistory {
        session,
        ns: answered,
        range,
        series,
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
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: ns("testdb"),
            range: ChartRange::Week,
            series: vec![],
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
            },
            "list_indexes",
        ),
        (
            Command::IndexHistory {
                session,
                ns: ns("d"),
                range: ChartRange::Day,
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
        },
        Command::IndexHistory {
            session,
            ns: ns("testdb"),
            range: ChartRange::Month,
        },
    ];
    commands.extend(index_writes(session));
    // Unknown session and failing commands answer once too.
    commands.push(Command::ListIndexes {
        session: Uuid::from_u128(77),
        ns: ns("testdb"),
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

#[tokio::test]
async fn a_zero_sampler_interval_is_clamped_to_one_second() {
    let dir = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        sampler: SamplerConfig {
            interval: Duration::ZERO,
            ..SamplerConfig::default()
        },
        ..config_at(&dir)
    };
    let path = config.history_path.clone().unwrap();
    let store = store_with(&["testdb"]);
    let mut h = spawn_connecting(store, config);
    let cfg = conn_config(false);
    connect(&mut h, &cfg).await;
    expect_sampler(&mut h, cfg.id, SamplerStatus::Running).await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let reader = open_history(&path);
    let key = key_in(cfg.id, "testdb");
    // One pass at connect and about one more per second: a tight loop would record hundreds.
    let buckets = reader
        .usage(&key, ChartRange::Day, clock::now_secs())
        .unwrap();
    assert!(buckets.iter().map(|b| b.uses).sum::<i64>() < 5);
}
