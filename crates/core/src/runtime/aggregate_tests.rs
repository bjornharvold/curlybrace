//! `AggregatePreview`: the documents arrive as an outcome inside `Event::AggregatePreviewed`; a
//! failure, a refused write stage or a timeout is an `Err` outcome, never an `Event::Failed`
//! (which would raise the error banner) and never a ping-settled `Unreachable`. Only a lost
//! connection fails the command, as for every read, so the session learns of it.

use std::time::Duration;

use bson::{Bson, Document};
use tokio::sync::Notify;

use super::*;
use crate::connection::ConnectionConfig;
use crate::store::{Namespace, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);

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

fn match_stage() -> Vec<Document> {
    vec![
        bson::doc! { "$match": { "status": "open" } },
        bson::doc! { "$limit": 20 },
    ]
}

fn preview_command(session: SessionId, pipeline: Vec<Document>) -> Command {
    Command::AggregatePreview {
        session,
        request: 9,
        ns: mock_namespace(),
        pipeline,
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connected_to(
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

/// The outcome of an `AggregatePreviewed` for `session` numbered 9; panics on any other event.
async fn previewed(h: &mut RuntimeHandle, session: SessionId) -> Result<Vec<Document>> {
    match next(h).await {
        Event::AggregatePreviewed {
            session: s,
            request,
            outcome,
        } => {
            assert_eq!(s, session);
            assert_eq!(request, 9);
            outcome
        }
        other => panic!("expected AggregatePreviewed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_preview_answers_with_the_documents() {
    let docs = vec![bson::doc! { "_id": 1 }, bson::doc! { "_id": 2 }];
    let store = Arc::new(InMemoryStore::new().with_aggregate(docs.clone()));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(preview_command(session, match_stage())).unwrap();
    assert_eq!(previewed(&mut h, session).await, Ok(docs));
    assert_eq!(store.received_pipelines(), vec![match_stage()]);
}

#[tokio::test]
async fn a_preview_is_a_numbered_read_allowed_on_a_read_only_session() {
    let docs = vec![bson::doc! { "_id": 1 }];
    let store = Arc::new(InMemoryStore::new().with_aggregate(docs.clone()));
    let (mut h, session) = connected_to(store, true, RuntimeConfig::default()).await;
    let command = preview_command(session, match_stage());
    assert!(!command.is_write());
    assert_eq!(command.context(), "aggregate_preview");
    assert_eq!(command.session(), Some(session));
    assert_eq!(command.request(), Some(9));
    h.tx.send(command).unwrap();
    assert_eq!(previewed(&mut h, session).await, Ok(docs));
}

#[tokio::test]
async fn a_write_stage_is_refused_before_the_store() {
    let store = Arc::new(InMemoryStore::new().with_aggregate(vec![bson::doc! { "_id": 1 }]));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    let pipeline = vec![
        bson::doc! { "$match": {} },
        bson::doc! { "$out": "elsewhere" },
    ];
    h.tx.send(preview_command(session, pipeline)).unwrap();
    let outcome = previewed(&mut h, session).await;
    assert!(outcome.is_err(), "{outcome:?}");
    assert!(store.received_pipelines().is_empty());
}

/// A store whose preview signals `started`, waits for `release`, then fails.
struct GatedAggregate {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

impl GatedAggregate {
    fn new() -> Self {
        Self {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }
}

#[async_trait::async_trait]
impl DocumentStore for GatedAggregate {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<crate::store::Page> {
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
    async fn ping(&self) -> Result<()> {
        // Never answers: a ping settle would turn a timeout into `Unreachable`.
        std::future::pending().await
    }
    async fn aggregate_preview(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _max_time: Duration,
    ) -> Result<Vec<Document>> {
        self.started.notify_one();
        self.release.notified().await;
        Err(CoreError::Database("pipeline failed".into()))
    }
}

#[tokio::test]
async fn a_store_error_is_an_outcome_not_a_failed_event() {
    let store = GatedAggregate::new();
    store.release.notify_one();
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(preview_command(session, match_stage())).unwrap();
    assert_eq!(
        previewed(&mut h, session).await,
        Err(CoreError::Database("pipeline failed".into()))
    );
}

#[tokio::test]
async fn a_timeout_is_an_outcome_without_a_ping_settle() {
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        ping_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let (mut h, session) = connected_to(Arc::new(GatedAggregate::new()), false, config).await;
    h.tx.send(preview_command(session, match_stage())).unwrap();
    assert_eq!(
        previewed(&mut h, session).await,
        Err(CoreError::Database(
            handlers::COMMAND_TIMEOUT_MESSAGE.into()
        ))
    );
}

#[tokio::test]
async fn a_late_preview_of_a_disconnected_session_is_superseded() {
    let store = GatedAggregate::new();
    let (started, release) = (store.started.clone(), store.release.clone());
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(preview_command(session, match_stage())).unwrap();
    started.notified().await;
    h.tx.send(Command::Disconnect { session }).unwrap();
    assert!(matches!(next(&mut h).await, Event::Disconnected { .. }));
    release.notify_one();
    match next(&mut h).await {
        Event::Failed { error, .. } | Event::RequestFailed { error, .. } => {
            assert_eq!(error, CoreError::Superseded)
        }
        other => panic!("expected a superseded answer, got {other:?}"),
    }
}

#[test]
fn a_preview_is_bounded_just_above_the_server_time_limit() {
    let command = preview_command(SessionId::new_v4(), match_stage());
    let server = Duration::from_millis(crate::pipeline::PREVIEW_MAX_TIME_MS);
    let limit = handlers::command_time_limit(&RuntimeConfig::default(), &command);
    assert!(limit > server && limit < COMMAND_TIMEOUT, "{limit:?}");
    let short = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    assert_eq!(
        handlers::command_time_limit(&short, &command),
        Duration::from_millis(50)
    );
}

/// A store whose preview fails at once with `error`; its ping answers when `ping_answers`.
struct FailingAggregate {
    error: CoreError,
    ping_answers: bool,
}

#[async_trait::async_trait]
impl DocumentStore for FailingAggregate {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<crate::store::Page> {
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
    async fn ping(&self) -> Result<()> {
        match self.ping_answers {
            true => Ok(()),
            false => Err(CoreError::Unreachable),
        }
    }
    async fn aggregate_preview(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _max_time: Duration,
    ) -> Result<Vec<Document>> {
        Err(self.error.clone())
    }
}

/// The event a preview against a store failing with `error` answers with.
async fn preview_failing_with(error: CoreError, ping_answers: bool) -> (Event, SessionId) {
    let store = FailingAggregate {
        error,
        ping_answers,
    };
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(preview_command(session, match_stage())).unwrap();
    (next(&mut h).await, session)
}

#[tokio::test]
async fn a_lost_server_fails_the_preview_as_a_read_so_the_session_learns_of_it() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let (event, session) = preview_failing_with(error.clone(), false).await;
        assert_eq!(
            event,
            Event::RequestFailed {
                session,
                context: "aggregate_preview",
                request: 9,
                error,
            }
        );
    }
}

#[tokio::test]
async fn an_interrupted_preview_is_settled_by_a_ping() {
    let interrupted = CoreError::ConnectionInterrupted("reset".into());
    let (event, _) = preview_failing_with(interrupted.clone(), false).await;
    assert!(
        matches!(
            &event,
            Event::RequestFailed {
                error: CoreError::Unreachable,
                ..
            }
        ),
        "{event:?}"
    );
    let (event, _) = preview_failing_with(interrupted, true).await;
    assert!(
        matches!(&event, Event::RequestFailed { error: CoreError::Database(t), .. } if t == "reset"),
        "a server that answers the ping makes it an ordinary failure: {event:?}"
    );
}

#[tokio::test]
async fn an_ordinary_store_error_stays_the_outcome_even_when_the_ping_fails() {
    let (event, _) = preview_failing_with(CoreError::Database("bad".into()), false).await;
    assert!(
        matches!(&event, Event::AggregatePreviewed { outcome: Err(CoreError::Database(t)), .. } if t == "bad"),
        "{event:?}"
    );
}
