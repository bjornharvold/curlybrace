//! `AggregateExplain`: the explain document arrives as an outcome inside
//! `Event::AggregateExplained`; a failure, a refused write stage or a timeout is an `Err` outcome,
//! never an `Event::Failed` (a banner) and never a ping-settled `Unreachable`. Only a lost
//! connection fails the command, as for every read.

use std::time::Duration;

use bson::{Bson, Document, doc};

use super::aggregate_tests::{connected_to, next};
use super::*;
use crate::explain::{AGGREGATE_EXPLAIN_MAX_TIME_MS, ExplainVerbosity};
use crate::store::{Explain, Namespace, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

fn stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "open" } },
        doc! { "$group": { "_id": "$customer" } },
    ]
}

fn explain_command(session: SessionId, stages: Vec<Document>, v: ExplainVerbosity) -> Command {
    Command::AggregateExplain {
        session,
        request: 5,
        ns: mock_namespace(),
        stages,
        verbosity: v,
    }
}

fn canned() -> Document {
    doc! { "ok": 1, "stages": [ { "$cursor": { "queryPlanner": {} } } ] }
}

/// The answer of an `AggregateExplained` for `session` numbered 5 on the mock namespace.
async fn explained(
    h: &mut RuntimeHandle,
    session: SessionId,
    verbosity: ExplainVerbosity,
) -> std::result::Result<Explain, CoreError> {
    match next(h).await {
        Event::AggregateExplained {
            session: s,
            request,
            ns,
            verbosity: v,
            outcome,
        } => {
            assert_eq!((s, request, v), (session, 5, verbosity));
            assert_eq!(ns, mock_namespace());
            outcome
        }
        other => panic!("expected AggregateExplained, got {other:?}"),
    }
}

#[tokio::test]
async fn an_explain_answers_with_the_document_under_the_requests_number() {
    let store = Arc::new(InMemoryStore::new().with_aggregate_explain(canned()));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    let v = ExplainVerbosity::QueryPlanner;
    h.tx.send(explain_command(session, stages(), v)).unwrap();
    let out = explained(&mut h, session, v).await.unwrap();
    assert_eq!(out.document, canned());
    assert!(out.plan_only);
    assert_eq!(store.explained_pipelines(), vec![(stages(), v)]);
}

#[tokio::test]
async fn execution_stats_are_forwarded_as_asked() {
    let store = Arc::new(InMemoryStore::new().with_aggregate_explain(canned()));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    let v = ExplainVerbosity::ExecutionStats;
    h.tx.send(explain_command(session, stages(), v)).unwrap();
    assert!(!explained(&mut h, session, v).await.unwrap().plan_only);
    assert_eq!(store.explained_pipelines(), vec![(stages(), v)]);
}

#[tokio::test]
async fn an_explain_is_a_numbered_read_allowed_on_a_read_only_session() {
    let store = Arc::new(InMemoryStore::new().with_aggregate_explain(canned()));
    let (mut h, session) = connected_to(store, true, RuntimeConfig::default()).await;
    let v = ExplainVerbosity::QueryPlanner;
    let command = explain_command(session, stages(), v);
    assert!(!command.is_write());
    assert_eq!(command.context(), "aggregate_explain");
    assert_eq!(command.session(), Some(session));
    assert_eq!(command.request(), Some(5));
    h.tx.send(command).unwrap();
    assert!(explained(&mut h, session, v).await.is_ok());
}

#[tokio::test]
async fn a_write_stage_is_refused_before_the_store_at_any_depth() {
    let store = Arc::new(InMemoryStore::new().with_aggregate_explain(canned()));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    let v = ExplainVerbosity::ExecutionStats;
    let nested = doc! { "$facet": { "f": [ { "$merge": "elsewhere" } ] } };
    for pipeline in [vec![doc! { "$out": "elsewhere" }], vec![nested]] {
        h.tx.send(explain_command(session, pipeline, v)).unwrap();
        let outcome = explained(&mut h, session, v).await;
        assert!(
            matches!(outcome, Err(CoreError::InvalidDocument(_))),
            "{outcome:?}"
        );
    }
    assert!(store.explained_pipelines().is_empty());
}

/// A store whose explain fails with `error` (or stalls when `error` is `None`); its ping
/// answers when `ping_answers`.
struct ScriptedExplain {
    error: Option<CoreError>,
    ping_answers: bool,
}

#[async_trait::async_trait]
impl DocumentStore for ScriptedExplain {
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
    async fn aggregate_explain(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _verbosity: ExplainVerbosity,
        _max_time: Duration,
    ) -> Result<Explain> {
        match &self.error {
            Some(error) => Err(error.clone()),
            None => std::future::pending().await,
        }
    }
}

async fn explain_against(store: ScriptedExplain, config: RuntimeConfig) -> (Event, SessionId) {
    let (mut h, session) = connected_to(Arc::new(store), false, config).await;
    let v = ExplainVerbosity::QueryPlanner;
    h.tx.send(explain_command(session, stages(), v)).unwrap();
    (next(&mut h).await, session)
}

#[tokio::test]
async fn a_store_error_is_an_outcome_not_a_failed_event() {
    let store = ScriptedExplain {
        error: Some(CoreError::Database("explain failed".into())),
        ping_answers: false,
    };
    let (event, _) = explain_against(store, RuntimeConfig::default()).await;
    assert!(
        matches!(&event, Event::AggregateExplained { outcome: Err(CoreError::Database(t)), .. } if t == "explain failed"),
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
    let store = ScriptedExplain {
        error: None,
        ping_answers: false,
    };
    let (event, session) = explain_against(store, config).await;
    assert_eq!(
        event,
        Event::AggregateExplained {
            session,
            request: 5,
            ns: mock_namespace(),
            verbosity: ExplainVerbosity::QueryPlanner,
            outcome: Err(CoreError::Database(
                handlers::COMMAND_TIMEOUT_MESSAGE.into()
            )),
        }
    );
}

#[tokio::test]
async fn a_lost_server_fails_the_explain_as_a_read_so_the_session_learns_of_it() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let store = ScriptedExplain {
            error: Some(error.clone()),
            ping_answers: false,
        };
        let (event, session) = explain_against(store, RuntimeConfig::default()).await;
        assert_eq!(
            event,
            Event::RequestFailed {
                session,
                context: "aggregate_explain",
                request: 5,
                error,
            }
        );
    }
}

#[tokio::test]
async fn an_interrupted_explain_is_settled_by_a_ping() {
    let interrupted = CoreError::ConnectionInterrupted("reset".into());
    let store = ScriptedExplain {
        error: Some(interrupted),
        ping_answers: true,
    };
    let (event, _) = explain_against(store, RuntimeConfig::default()).await;
    assert!(
        matches!(&event, Event::RequestFailed { error: CoreError::Database(t), .. } if t == "reset"),
        "{event:?}"
    );
}

#[test]
fn an_explain_is_bounded_just_above_the_server_time_limit() {
    let command = explain_command(
        SessionId::new_v4(),
        stages(),
        ExplainVerbosity::ExecutionStats,
    );
    let server = Duration::from_millis(AGGREGATE_EXPLAIN_MAX_TIME_MS);
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
