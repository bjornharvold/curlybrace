//! `ProfilingState`, `SetProfilingLevel` and `QueryShapes`: the profiler answers are outcomes
//! inside `Event::ProfilingState` (never an `Event::Failed`), shapes come from the history.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use bson::{Bson, Document};

use super::*;
use crate::clock;
use crate::connection::ConnectionConfig;
use crate::explain::VerdictKind;
use crate::history::{ShapeKey, ShapeObservation, ShapeSource};
use crate::store::{PROFILER_MONGOS_MESSAGE, ProfilingState, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);

/// Hands out one store, and counts the profiler writes it saw.
struct Recording {
    inner: InMemoryStore,
    sets: Mutex<Vec<(String, i32)>>,
}

impl Recording {
    fn new(inner: InMemoryStore) -> Arc<Self> {
        Arc::new(Self {
            inner,
            sets: Mutex::new(Vec::new()),
        })
    }

    fn sets(&self) -> Vec<(String, i32)> {
        self.sets.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DocumentStore for Recording {
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
    async fn explain_find(&self, ns: &Namespace, q: &Query) -> Result<crate::store::Explain> {
        self.inner.explain_find(ns, q).await
    }
    async fn profiling_level(&self, db: &str) -> Result<ProfilingState> {
        self.inner.profiling_level(db).await
    }
    async fn set_profiling_level(&self, db: &str, level: i32) -> Result<ProfilingState> {
        self.sets.lock().unwrap().push((db.to_owned(), level));
        self.inner.set_profiling_level(db, level).await
    }
}

/// A store whose profiler never answers.
struct HangingProfiler;

#[async_trait::async_trait]
impl DocumentStore for HangingProfiler {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn profiling_level(&self, _db: &str) -> Result<ProfilingState> {
        std::future::pending().await
    }
    async fn set_profiling_level(&self, _db: &str, _level: i32) -> Result<ProfilingState> {
        std::future::pending().await
    }
}

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

/// Hands out its stores one per connect, in order.
struct SeqConnector(Mutex<VecDeque<Arc<dyn DocumentStore>>>);

#[async_trait::async_trait]
impl Connector for SeqConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| CoreError::Unsupported("no more stores".into()))
    }
}

struct FailingConnector;

#[async_trait::async_trait]
impl Connector for FailingConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Err(CoreError::Unsupported("refused".into()))
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connected_with(
    store: Arc<dyn DocumentStore>,
    read_only: bool,
    config: RuntimeConfig,
) -> (RuntimeHandle, SessionId) {
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(store)),
        Arc::new(|| {}),
        config,
    );
    let cfg = ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    };
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    (h, cfg.id)
}

async fn connected(store: Arc<dyn DocumentStore>, read_only: bool) -> (RuntimeHandle, SessionId) {
    connected_with(store, read_only, RuntimeConfig::default()).await
}

fn state_command(session: SessionId) -> Command {
    Command::ProfilingState {
        session,
        db: "shop".into(),
    }
}

fn set_command(session: SessionId, level: i32) -> Command {
    Command::SetProfilingLevel {
        session,
        db: "shop".into(),
        level,
    }
}

async fn outcome(h: &mut RuntimeHandle, session: SessionId) -> Result<ProfilingState> {
    match next(h).await {
        Event::ProfilingState {
            session: s,
            db,
            outcome,
        } => {
            assert_eq!(s, session);
            assert_eq!(db, "shop");
            outcome
        }
        other => panic!("expected ProfilingState, got {other:?}"),
    }
}

#[tokio::test]
async fn profiling_state_answers_with_the_level() {
    let store = Recording::new(InMemoryStore::new().with_profiling_level("shop", 1));
    let (mut h, session) = connected(store, false).await;
    let command = state_command(session);
    assert!(!command.is_write());
    assert_eq!(command.context(), "profiling_state");
    h.tx.send(command).unwrap();
    let state = outcome(&mut h, session).await.expect("a state");
    assert_eq!(state.level, 1);
}

#[tokio::test]
async fn set_profiling_level_updates_and_answers() {
    let store = Recording::new(InMemoryStore::new().with_profiling_level("shop", 0));
    let (mut h, session) = connected(store.clone(), false).await;
    let command = set_command(session, 2);
    assert!(command.is_write());
    h.tx.send(command).unwrap();
    assert_eq!(outcome(&mut h, session).await.expect("a state").level, 2);
    h.tx.send(state_command(session)).unwrap();
    assert_eq!(outcome(&mut h, session).await.expect("a state").level, 2);
    assert_eq!(store.sets(), [("shop".to_owned(), 2)]);
}

#[tokio::test]
async fn set_profiling_level_is_refused_on_a_read_only_session_and_leaves_the_level() {
    let store = Recording::new(InMemoryStore::new().with_profiling_level("shop", 1));
    let (mut h, session) = connected(store.clone(), true).await;
    h.tx.send(set_command(session, 2)).unwrap();
    assert!(matches!(
        outcome(&mut h, session).await,
        Err(CoreError::ReadOnly)
    ));
    h.tx.send(state_command(session)).unwrap();
    assert_eq!(outcome(&mut h, session).await.expect("a state").level, 1);
    assert!(store.sets().is_empty(), "the store never saw the write");
}

#[tokio::test]
async fn a_lock_sent_after_connecting_also_refuses_set_profiling_level() {
    let store = Recording::new(InMemoryStore::new().with_profiling_level("shop", 0));
    let (mut h, session) = connected(store.clone(), false).await;
    h.tx.send(Command::LockReadOnly {
        connection: session,
    })
    .unwrap();
    // The lock has no answer; a read after it proves it was applied before the write below.
    h.tx.send(state_command(session)).unwrap();
    outcome(&mut h, session).await.expect("a state");
    h.tx.send(set_command(session, 1)).unwrap();
    assert!(matches!(
        outcome(&mut h, session).await,
        Err(CoreError::ReadOnly)
    ));
    assert!(store.sets().is_empty());
}

#[tokio::test]
async fn profiler_errors_are_outcomes_not_failed_events() {
    let denied = InMemoryStore::new().with_profile_error("shop", CoreError::Unauthorized);
    let (mut h, session) = connected(Recording::new(denied), false).await;
    h.tx.send(state_command(session)).unwrap();
    assert!(matches!(
        outcome(&mut h, session).await,
        Err(CoreError::Unauthorized)
    ));
    h.tx.send(set_command(session, 1)).unwrap();
    assert!(matches!(
        outcome(&mut h, session).await,
        Err(CoreError::Unauthorized)
    ));

    let mongos = InMemoryStore::new().with_profile_error(
        "shop",
        CoreError::Unsupported(PROFILER_MONGOS_MESSAGE.into()),
    );
    let (mut h, session) = connected(Recording::new(mongos), false).await;
    h.tx.send(state_command(session)).unwrap();
    match outcome(&mut h, session).await {
        Err(CoreError::Unsupported(text)) => assert_eq!(text, PROFILER_MONGOS_MESSAGE),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[tokio::test]
async fn profiler_timeout_is_an_outcome_not_a_failure() {
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let (mut h, session) = connected_with(Arc::new(HangingProfiler), false, config).await;
    for command in [state_command(session), set_command(session, 1)] {
        h.tx.send(command).unwrap();
        match outcome(&mut h, session).await {
            Err(CoreError::Database(text)) => {
                assert_eq!(text, super::handlers::COMMAND_TIMEOUT_MESSAGE)
            }
            other => panic!("expected a timeout outcome, got {other:?}"),
        }
    }
}

fn observation(session: SessionId, hash: &str, ns: &Namespace) -> ShapeObservation {
    ShapeObservation {
        key: ShapeKey {
            connection: session,
            host: String::new(),
            db: ns.db.clone(),
            collection: ns.collection.clone(),
            hash: hash.into(),
            source: ShapeSource::Typed,
        },
        shape_json: r#"{"filter":{"a":"$eq"}}"#.into(),
        at_secs: clock::now_secs() - 60,
        executions: 3,
        misses: 2,
        total_millis: 30,
        max_millis: 20,
        worst: VerdictKind::CollectionScan,
        gap_secs: 0,
    }
}

fn shapes_command(session: SessionId, ns: Namespace, request: u64) -> Command {
    Command::QueryShapes {
        session,
        ns,
        range: ChartRange::Day,
        request,
    }
}

#[tokio::test]
async fn query_shapes_answers_from_history_for_the_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let ns = mock_namespace();
    let other = Namespace::new(&ns.db, "elsewhere");
    let config = RuntimeConfig {
        history_path: Some(path.clone()),
        ..RuntimeConfig::default()
    };
    let store = Recording::new(InMemoryStore::new());
    let (mut h, session) = connected_with(store, false, config).await;
    let (db, _) = HistoryDb::open(&path).unwrap();
    db.record_shapes(&[
        observation(session, "mine", &ns),
        observation(session, "theirs", &other),
    ])
    .unwrap();
    let command = shapes_command(session, ns.clone(), 9);
    assert!(!command.is_write());
    assert_eq!(command.context(), "query_shapes");
    assert_eq!(command.request(), Some(9));
    h.tx.send(command).unwrap();
    match next(&mut h).await {
        Event::QueryShapes {
            session: s,
            ns: n,
            range,
            shapes,
            request,
        } => {
            assert_eq!((s, n, range, request), (session, ns, ChartRange::Day, 9));
            assert_eq!(shapes.len(), 1);
            assert_eq!(shapes[0].hash, "mine");
            assert_eq!((shapes[0].executions, shapes[0].misses), (3, 2));
        }
        other => panic!("expected QueryShapes, got {other:?}"),
    }
}

#[tokio::test]
async fn query_shapes_without_history_is_empty() {
    let (mut h, session) = connected(Recording::new(InMemoryStore::new()), false).await;
    h.tx.send(shapes_command(session, mock_namespace(), 1))
        .unwrap();
    match next(&mut h).await {
        Event::QueryShapes { shapes, .. } => assert!(shapes.is_empty()),
        other => panic!("expected QueryShapes, got {other:?}"),
    }
}

#[tokio::test]
async fn query_shapes_failure_is_a_request_failed() {
    let mut h = spawn_runtime_with_config(
        None,
        Some(Arc::new(InMemoryStore::new())),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    // No session answers to this id: the read fails before reaching a store.
    let session = uuid::Uuid::from_u128(404);
    h.tx.send(shapes_command(session, mock_namespace(), 5))
        .unwrap();
    match next(&mut h).await {
        Event::RequestFailed {
            session: s,
            context,
            request,
            ..
        } => assert_eq!((s, context, request), (session, "query_shapes", 5)),
        other => panic!("expected RequestFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn only_the_set_command_ever_writes_the_profiler_level() {
    let dir = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        history_path: Some(dir.path().join("history.sqlite")),
        ..RuntimeConfig::default()
    };
    let store = Recording::new(InMemoryStore::new().with_profiling_level("shop", 1));
    let (mut h, session) = connected_with(store.clone(), false, config).await;
    h.tx.send(state_command(session)).unwrap();
    outcome(&mut h, session).await.expect("a state");
    h.tx.send(shapes_command(session, mock_namespace(), 1))
        .unwrap();
    // The sampler may report on its own before the answer.
    loop {
        match next(&mut h).await {
            Event::QueryShapes { .. } => break,
            Event::Sampler { .. } => {}
            other => panic!("expected QueryShapes, got {other:?}"),
        }
    }
    h.tx.send(Command::Disconnect { session }).unwrap();
    loop {
        if matches!(next(&mut h).await, Event::Disconnected { .. }) {
            break;
        }
    }
    // Let anything still running in the background show itself before counting.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(store.sets().is_empty(), "nothing changed the profiler");
}

#[tokio::test]
async fn query_shapes_returns_only_the_asking_connections_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let ns = mock_namespace();
    let config = RuntimeConfig {
        history_path: Some(path.clone()),
        ..RuntimeConfig::default()
    };
    let (mut h, session) =
        connected_with(Recording::new(InMemoryStore::new()), false, config).await;
    let stranger = uuid::Uuid::from_u128(77);
    let (db, _) = HistoryDb::open(&path).unwrap();
    db.record_shapes(&[
        observation(session, "mine", &ns),
        observation(stranger, "stranger", &ns),
    ])
    .unwrap();
    h.tx.send(shapes_command(session, ns, 1)).unwrap();
    loop {
        match next(&mut h).await {
            Event::QueryShapes { shapes, .. } => {
                let hashes: Vec<_> = shapes.iter().map(|s| s.hash.as_str()).collect();
                assert_eq!(hashes, ["mine"]);
                break;
            }
            Event::Sampler { .. } => {}
            other => panic!("expected QueryShapes, got {other:?}"),
        }
    }
}

async fn bare_runtime() -> RuntimeHandle {
    spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FailingConnector),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    )
}

#[tokio::test]
async fn profiling_state_for_a_missing_session_is_an_outcome() {
    let mut h = bare_runtime().await;
    let session = uuid::Uuid::from_u128(404);
    h.tx.send(state_command(session)).unwrap();
    assert!(matches!(
        outcome(&mut h, session).await,
        Err(CoreError::Unsupported(_))
    ));
}

#[tokio::test]
async fn set_profiling_level_for_a_missing_session_is_an_outcome() {
    let mut h = bare_runtime().await;
    let session = uuid::Uuid::from_u128(404);
    h.tx.send(set_command(session, 1)).unwrap();
    assert!(matches!(
        outcome(&mut h, session).await,
        Err(CoreError::Unsupported(_))
    ));
}

#[tokio::test]
async fn profiling_state_not_connected_is_an_outcome() {
    let mut h = bare_runtime().await;
    let cfg = ConnectionConfig::new("c", "mongodb://localhost:27017");
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::ConnectFailed { .. }));
    for command in [state_command(cfg.id), set_command(cfg.id, 1)] {
        h.tx.send(command).unwrap();
        assert!(matches!(
            outcome(&mut h, cfg.id).await,
            Err(CoreError::Unsupported(_))
        ));
    }
}

/// Connects to `old`, then sends `command` and a reconnect (to `new`) back to back, so the
/// command is issued for a generation that is over before it runs. Returns its answer.
async fn answer_of_a_command_overtaken_by_a_reconnect(
    command: impl FnOnce(SessionId) -> Command,
    new: Arc<Recording>,
) -> Event {
    let old = Recording::new(InMemoryStore::new().with_profiling_level("shop", 0));
    let connector = SeqConnector(Mutex::new(VecDeque::from([
        old as Arc<dyn DocumentStore>,
        new as Arc<dyn DocumentStore>,
    ])));
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(connector),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let cfg = ConnectionConfig::new("c", "mongodb://localhost:27017");
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    h.tx.send(command(cfg.id)).unwrap();
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    let mut answer = None;
    let mut connected = false;
    while answer.is_none() || !connected {
        match next(&mut h).await {
            Event::Connected { .. } => connected = true,
            Event::Sampler { .. } => {}
            other => answer = Some(other),
        }
    }
    answer.unwrap()
}

#[tokio::test]
async fn set_profiling_level_from_an_old_generation_never_reaches_the_new_connection() {
    let new = Recording::new(InMemoryStore::new().with_profiling_level("shop", 0));
    let event =
        answer_of_a_command_overtaken_by_a_reconnect(|s| set_command(s, 2), new.clone()).await;
    assert!(new.sets().is_empty(), "the new connection saw the write");
    assert!(
        !matches!(event, Event::ProfilingState { outcome: Ok(_), .. }),
        "{event:?}"
    );
}

#[tokio::test]
async fn superseded_profiler_command_is_not_a_banner() {
    // Kept as `Failed { Superseded }`, like every other superseded command: the UI drops it, and
    // it is no real failure to show inline.
    let new = Recording::new(InMemoryStore::new());
    let event = answer_of_a_command_overtaken_by_a_reconnect(|s| set_command(s, 2), new).await;
    assert!(
        matches!(
            event,
            Event::Failed {
                context: "set_profiling_level",
                error: CoreError::Superseded,
                ..
            }
        ),
        "{event:?}"
    );
    let new = Recording::new(InMemoryStore::new());
    let event = answer_of_a_command_overtaken_by_a_reconnect(state_command, new).await;
    assert!(
        matches!(
            event,
            Event::Failed {
                error: CoreError::Superseded,
                ..
            }
        ),
        "{event:?}"
    );
}
