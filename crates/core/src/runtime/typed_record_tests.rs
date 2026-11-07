//! A successful `ExplainFind` records the typed query's shape with its verdict in the history;
//! nothing about the recording may change the verdict event.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::clock;
use crate::connection::ConnectionConfig;
use crate::ejson::editor_json_to_doc;
use crate::explain::VerdictKind;
use crate::history::{ChartRange, HistoryDb, ShapeSource, ShapeSummary};
use crate::shape::from_query;
use crate::store::Query;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;
use bson::Document;

const WAIT: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_millis(400);
const SECRET: &str = "zebra-secret@example.com";

struct FixedConnector(Arc<dyn DocumentStore>);

#[async_trait::async_trait]
impl Connector for FixedConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

fn collscan() -> Document {
    editor_json_to_doc(include_str!("../explain/fixtures/collscan.json")).expect("fixture parses")
}

fn typed_query() -> Query {
    Query {
        filter: bson::doc! { "email": SECRET, "age": 424_242 },
        ..Query::default()
    }
}

/// The next event that is not a background status.
async fn next(h: &mut RuntimeHandle) -> Event {
    loop {
        let event = tokio::time::timeout(WAIT, h.rx.recv())
            .await
            .expect("event within bound")
            .expect("channel open");
        if !matches!(event, Event::Sampler { .. } | Event::Profiler { .. }) {
            return event;
        }
    }
}

struct Setup {
    h: RuntimeHandle,
    session: SessionId,
    path: PathBuf,
    _dir: tempfile::TempDir,
}

async fn connected(store: InMemoryStore, read_only: bool) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let config = RuntimeConfig {
        history_path: Some(path.clone()),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(store))),
        Arc::new(|| {}),
        config,
    );
    let cfg = ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    };
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    Setup {
        h,
        session: cfg.id,
        path,
        _dir: dir,
    }
}

fn explain(session: SessionId) -> Command {
    Command::ExplainFind {
        session,
        ns: mock_namespace(),
        query: typed_query(),
    }
}

async fn explained(setup: &mut Setup) -> Event {
    setup.h.tx.send(explain(setup.session)).unwrap();
    next(&mut setup.h).await
}

fn typed_rows(path: &Path, session: SessionId) -> Vec<ShapeSummary> {
    let ns = mock_namespace();
    HistoryDb::open(path)
        .unwrap()
        .0
        .shapes_for(
            session,
            &ns.db,
            &ns.collection,
            ChartRange::Day,
            clock::now_secs() + 60,
        )
        .unwrap()
}

/// Waits for the detached recording to land.
async fn recorded(setup: &Setup) -> Vec<ShapeSummary> {
    for _ in 0..100 {
        let rows = typed_rows(&setup.path, setup.session);
        if !rows.is_empty() {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Vec::new()
}

async fn nothing_recorded(setup: &Setup) -> bool {
    tokio::time::sleep(SETTLE).await;
    typed_rows(&setup.path, setup.session).is_empty()
}

#[tokio::test]
async fn successful_explain_records_a_typed_shape_with_its_verdict() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let mut setup = connected(store, false).await;
    let Event::Explain { outcome, .. } = explained(&mut setup).await else {
        panic!("expected Explain");
    };
    assert_eq!(outcome.unwrap().kind, VerdictKind::CollectionScan);

    let rows = recorded(&setup).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.source, ShapeSource::Typed);
    assert_eq!((row.executions, row.misses), (1, 1));
    assert_eq!(row.worst, VerdictKind::CollectionScan);
    let shape = from_query(&typed_query());
    assert_eq!(row.hash, shape.hash());
    assert_eq!(row.shape_json, shape.json());
}

#[tokio::test]
async fn failed_explain_records_nothing() {
    let mut setup = connected(InMemoryStore::new(), false).await;
    let Event::Explain { outcome, .. } = explained(&mut setup).await else {
        panic!("expected Explain");
    };
    assert!(outcome.is_err());
    assert!(nothing_recorded(&setup).await);
}

#[tokio::test]
async fn typed_record_works_on_a_read_only_session() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let mut setup = connected(store, true).await;
    let Event::Explain { outcome, .. } = explained(&mut setup).await else {
        panic!("expected Explain");
    };
    assert!(outcome.is_ok());
    assert_eq!(recorded(&setup).await.len(), 1);
}

#[tokio::test]
async fn typed_record_stores_no_literals() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let mut setup = connected(store, false).await;
    explained(&mut setup).await;
    let rows = recorded(&setup).await;
    assert!(!format!("{rows:?}").contains("zebra-secret"));
    assert!(!format!("{rows:?}").contains("424242"));
    for suffix in ["", "-wal"] {
        let mut name = setup.path.as_os_str().to_owned();
        name.push(suffix);
        let bytes = std::fs::read(PathBuf::from(name)).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("zebra-secret"),
            "literal in the file{suffix}"
        );
    }
}

#[tokio::test]
async fn purged_connection_is_not_recorded() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let mut setup = connected(store, false).await;
    setup
        .h
        .tx
        .send(Command::PurgeHistory {
            connection: setup.session,
        })
        .unwrap();
    let Event::Explain { outcome, .. } = explained(&mut setup).await else {
        panic!("expected Explain");
    };
    assert!(outcome.is_ok(), "the verdict is unaffected");
    assert!(nothing_recorded(&setup).await);
}

#[tokio::test]
async fn history_failure_never_changes_the_verdict_event() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let mut setup = connected(store, false).await;
    let other = rusqlite::Connection::open(&setup.path).unwrap();
    other
        .execute_batch("DROP TABLE shape_bucket; DROP TABLE shape_gap; DROP TABLE shape;")
        .unwrap();
    let Event::Explain { outcome, .. } = explained(&mut setup).await else {
        panic!("expected Explain");
    };
    assert_eq!(outcome.unwrap().kind, VerdictKind::CollectionScan);
}
