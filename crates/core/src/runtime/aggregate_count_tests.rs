//! `AggregateCount`: the number arrives as an outcome inside `Event::AggregateCounted`; a failure,
//! a refused write stage, an unreadable answer or a timeout is an `Err` outcome, never an
//! `Event::Failed` (a banner) and never a ping-settled `Unreachable`. Only a lost connection fails
//! the command, as for every read.

use std::time::Duration;

use bson::{Bson, Document, doc};

use super::aggregate_tests::{connected_to, next};
use super::*;
use crate::pipeline::{COUNT_CAP, Stage, count_pipeline};
use crate::store::{Namespace, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

fn count_command(session: SessionId, pipeline: Vec<Document>) -> Command {
    Command::AggregateCount {
        session,
        request: 4,
        ns: mock_namespace(),
        stage_index: 2,
        pipeline,
    }
}

fn counted_pipeline() -> Vec<Document> {
    let stages = vec![Stage {
        text: r#"{"$match": {"a": 1}}"#.into(),
        enabled: true,
    }];
    count_pipeline(&stages, 0, COUNT_CAP).unwrap()
}

async fn counted(h: &mut RuntimeHandle, session: SessionId) -> std::result::Result<u64, CoreError> {
    match next(h).await {
        Event::AggregateCounted {
            session: s,
            request,
            ns,
            stage_index,
            outcome,
        } => {
            assert_eq!((s, request, stage_index), (session, 4, 2));
            assert_eq!(ns, mock_namespace());
            outcome
        }
        other => panic!("expected AggregateCounted, got {other:?}"),
    }
}

#[tokio::test]
async fn a_count_answers_with_n_under_the_requests_number() {
    let store = Arc::new(InMemoryStore::new().with_aggregate(vec![doc! { "n": 42_i32 }]));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(count_command(session, counted_pipeline()))
        .unwrap();
    assert_eq!(counted(&mut h, session).await, Ok(42));
    assert_eq!(store.received_pipelines(), vec![counted_pipeline()]);
}

#[tokio::test]
async fn an_empty_answer_is_a_count_of_zero() {
    let store = Arc::new(InMemoryStore::new().with_aggregate(Vec::new()));
    let (mut h, session) = connected_to(store, false, RuntimeConfig::default()).await;
    h.tx.send(count_command(session, counted_pipeline()))
        .unwrap();
    assert_eq!(counted(&mut h, session).await, Ok(0));
}

#[tokio::test]
async fn an_unreadable_answer_is_an_outcome() {
    let store = Arc::new(InMemoryStore::new().with_aggregate(vec![doc! { "n": "many" }]));
    let (mut h, session) = connected_to(store, false, RuntimeConfig::default()).await;
    h.tx.send(count_command(session, counted_pipeline()))
        .unwrap();
    let outcome = counted(&mut h, session).await;
    assert!(
        matches!(outcome, Err(CoreError::InvalidDocument(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_count_is_a_numbered_read_allowed_on_a_read_only_session() {
    let store = Arc::new(InMemoryStore::new().with_aggregate(vec![doc! { "n": 3_i64 }]));
    let (mut h, session) = connected_to(store, true, RuntimeConfig::default()).await;
    let command = count_command(session, counted_pipeline());
    assert!(!command.is_write());
    assert_eq!(command.context(), "aggregate_count");
    assert_eq!(command.session(), Some(session));
    assert_eq!(command.request(), Some(4));
    h.tx.send(command).unwrap();
    assert_eq!(counted(&mut h, session).await, Ok(3));
}

#[tokio::test]
async fn a_write_stage_is_refused_before_the_store() {
    let store = Arc::new(InMemoryStore::new().with_aggregate(vec![doc! { "n": 1 }]));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    let nested = doc! { "$facet": { "f": [ { "$merge": "elsewhere" } ] } };
    for pipeline in [vec![doc! { "$out": "elsewhere" }], vec![nested]] {
        h.tx.send(count_command(session, pipeline)).unwrap();
        let outcome = counted(&mut h, session).await;
        assert!(outcome.is_err(), "{outcome:?}");
    }
    assert!(store.received_pipelines().is_empty());
}

/// A store whose aggregate fails with `error` (or stalls when `error` is `None`); its ping
/// answers when `ping_answers`.
struct ScriptedCount {
    error: Option<CoreError>,
    ping_answers: bool,
}

#[async_trait::async_trait]
impl DocumentStore for ScriptedCount {
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
            // A ping settle would turn a timeout into `Unreachable`.
            false => std::future::pending().await,
        }
    }
    async fn aggregate_preview(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _max_time: Duration,
    ) -> Result<Vec<Document>> {
        match &self.error {
            Some(error) => Err(error.clone()),
            None => std::future::pending().await,
        }
    }
}

async fn count_against(store: ScriptedCount, config: RuntimeConfig) -> (Event, SessionId) {
    let (mut h, session) = connected_to(Arc::new(store), false, config).await;
    h.tx.send(count_command(session, counted_pipeline()))
        .unwrap();
    (next(&mut h).await, session)
}

#[tokio::test]
async fn a_store_error_is_an_outcome_not_a_failed_event() {
    let store = ScriptedCount {
        error: Some(CoreError::Database("count failed".into())),
        ping_answers: false,
    };
    let (event, _) = count_against(store, RuntimeConfig::default()).await;
    assert!(
        matches!(&event, Event::AggregateCounted { outcome: Err(CoreError::Database(t)), .. } if t == "count failed"),
        "{event:?}"
    );
}

#[tokio::test]
async fn a_timeout_is_an_outcome_without_a_ping_settle() {
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        ping_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let store = ScriptedCount {
        error: None,
        ping_answers: false,
    };
    let (event, session) = count_against(store, config).await;
    assert_eq!(
        event,
        Event::AggregateCounted {
            session,
            request: 4,
            ns: mock_namespace(),
            stage_index: 2,
            outcome: Err(CoreError::Database(
                handlers::COMMAND_TIMEOUT_MESSAGE.into()
            )),
        }
    );
}

#[tokio::test]
async fn a_lost_server_fails_the_count_as_a_read_so_the_session_learns_of_it() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let store = ScriptedCount {
            error: Some(error.clone()),
            ping_answers: false,
        };
        let (event, session) = count_against(store, RuntimeConfig::default()).await;
        assert_eq!(
            event,
            Event::RequestFailed {
                session,
                context: "aggregate_count",
                request: 4,
                error,
            }
        );
    }
}

#[tokio::test]
async fn an_interrupted_count_is_settled_by_a_ping() {
    let store = ScriptedCount {
        error: Some(CoreError::ConnectionInterrupted("reset".into())),
        ping_answers: true,
    };
    let (event, _) = count_against(store, RuntimeConfig::default()).await;
    assert!(
        matches!(&event, Event::RequestFailed { error: CoreError::Database(t), .. } if t == "reset"),
        "{event:?}"
    );
}

#[test]
fn a_count_is_bounded_like_a_preview() {
    let command = count_command(SessionId::new_v4(), counted_pipeline());
    let server = Duration::from_millis(crate::pipeline::PREVIEW_MAX_TIME_MS);
    let limit = handlers::command_time_limit(&RuntimeConfig::default(), &command);
    assert!(limit > server, "{limit:?}");
    let short = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    assert_eq!(
        handlers::command_time_limit(&short, &command),
        Duration::from_millis(50)
    );
}
