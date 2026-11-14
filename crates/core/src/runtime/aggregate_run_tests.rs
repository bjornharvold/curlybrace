//! `AggregateRun`: a write pipeline runs on the store and is answered with the target's count.
//! A pipeline or server failure is an outcome shown inline; a lost connection fails the command
//! so the write-interruption path applies; a client timeout is an unknown outcome.

use std::time::Duration;

use bson::{Bson, Document, doc};

use super::aggregate_tests::{connected_to, next};
use super::*;
use crate::store::{Namespace, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

fn run_command(session: SessionId, pipeline: Vec<Document>) -> Command {
    Command::AggregateRun {
        session,
        ns: mock_namespace(),
        pipeline,
        run: 4,
    }
}

fn out_pipeline() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "open" } },
        doc! { "$out": "dest" },
    ]
}

fn target_docs(ns: &Namespace, n: i32) -> InMemoryStore {
    InMemoryStore::new().with_docs(ns, (0..n).map(|i| doc! { "_id": i }).collect())
}

#[tokio::test]
async fn a_run_reaches_the_store_and_answers_with_the_target_count() {
    let source = mock_namespace();
    let target = Namespace::new(source.db.clone(), "dest");
    let store = Arc::new(target_docs(&target, 3));
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::AggregateRan {
            session,
            run: 4,
            ns: source,
            target,
            outcome: Ok(Some(3)),
        }
    );
    assert_eq!(store.written_pipelines(), vec![out_pipeline()]);
    assert!(store.received_pipelines().is_empty());
}

#[tokio::test]
async fn the_target_follows_the_stage_forms() {
    let source = mock_namespace();
    let cases = [
        (
            vec![doc! { "$merge": { "into": "dest" } }],
            Namespace::new(source.db.clone(), "dest"),
        ),
        (
            vec![doc! { "$out": { "db": "other", "coll": "dest" } }],
            Namespace::new("other", "dest"),
        ),
        (
            vec![doc! { "$merge": { "into": { "db": "other", "coll": "dest" } } }],
            Namespace::new("other", "dest"),
        ),
    ];
    for (pipeline, expected) in cases {
        let store = Arc::new(target_docs(&expected, 2));
        let (mut h, session) = connected_to(store, false, RuntimeConfig::default()).await;
        h.tx.send(run_command(session, pipeline)).unwrap();
        match next(&mut h).await {
            Event::AggregateRan {
                target, outcome, ..
            } => {
                assert_eq!(target, expected);
                assert_eq!(outcome, Ok(Some(2)));
            }
            other => panic!("expected AggregateRan, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_read_only_session_refuses_a_run_and_the_store_sees_nothing() {
    let store = Arc::new(InMemoryStore::new());
    let (mut h, session) = connected_to(store.clone(), true, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            error: CoreError::ReadOnly,
            ..
        }
    ));
    assert!(store.written_pipelines().is_empty());
}

#[tokio::test]
async fn a_pipeline_without_a_write_stage_is_an_outcome_and_nothing_is_written() {
    let store = Arc::new(InMemoryStore::new());
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, vec![doc! { "$match": {} }]))
        .unwrap();
    match next(&mut h).await {
        Event::AggregateRan { outcome, .. } => {
            assert!(matches!(outcome, Err(CoreError::InvalidDocument(_))))
        }
        other => panic!("expected AggregateRan, got {other:?}"),
    }
    assert!(store.written_pipelines().is_empty());
}

#[tokio::test]
async fn a_server_failure_is_an_outcome_not_a_failed_event() {
    let store = InMemoryStore::new().with_aggregate_write_error(CoreError::Database("bad".into()));
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateRan { outcome: Err(CoreError::Database(t)), .. } if t == "bad"
    ));
}

#[tokio::test]
async fn a_lost_server_fails_the_command() {
    let store = InMemoryStore::new().with_aggregate_write_error(CoreError::Unreachable);
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session,
            context: "aggregate_run",
            error: CoreError::Unreachable,
        }
    );
}

#[tokio::test]
async fn an_interrupted_run_is_an_unknown_outcome_when_the_server_still_answers() {
    let store = InMemoryStore::new()
        .with_aggregate_write_error(CoreError::ConnectionInterrupted("reset".into()));
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    // Answered under the run's number, so a late answer can never settle another run.
    assert_eq!(
        next(&mut h).await,
        Event::AggregateRan {
            session,
            run: 4,
            ns: mock_namespace(),
            target: Namespace::new("testdb", "dest"),
            outcome: Err(CoreError::WriteOutcomeUnknown),
        }
    );
}

#[tokio::test]
async fn a_server_time_limit_expiry_is_an_unknown_outcome() {
    let store = InMemoryStore::new().with_aggregate_write_error(CoreError::Database(
        handlers::COMMAND_TIMEOUT_MESSAGE.into(),
    ));
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateRan {
            run: 4,
            outcome: Err(CoreError::WriteOutcomeUnknown),
            ..
        }
    ));
}

#[tokio::test]
async fn a_target_in_a_server_database_is_refused_before_anything_runs() {
    let store = Arc::new(InMemoryStore::new());
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    let into_local = vec![doc! { "$out": { "db": "local", "coll": "x" } }];
    h.tx.send(run_command(session, into_local)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateRan {
            outcome: Err(CoreError::ReservedDatabase),
            ..
        }
    ));
    // The source's own database, when the stage names none.
    h.tx.send(Command::AggregateRun {
        session,
        ns: Namespace::new("admin", "src"),
        pipeline: out_pipeline(),
        run: 5,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateRan {
            run: 5,
            outcome: Err(CoreError::ReservedDatabase),
            ..
        }
    ));
    assert!(store.written_pipelines().is_empty());
}

/// A store whose write run never finishes, and whose ping never answers.
struct HangingWrite;

#[async_trait::async_trait]
impl DocumentStore for HangingWrite {
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
        std::future::pending().await
    }
    async fn aggregate_write(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _max_time: Duration,
    ) -> Result<()> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn a_client_timeout_is_an_unknown_outcome_never_a_plain_failure() {
    let config = RuntimeConfig {
        index_build_timeout: Duration::from_millis(50),
        ping_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let (mut h, session) = connected_to(Arc::new(HangingWrite), false, config).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    match next(&mut h).await {
        Event::AggregateRan {
            session: s,
            run,
            ns,
            target,
            outcome,
        } => {
            assert_eq!(s, session);
            assert_eq!(run, 4);
            assert_eq!(ns, mock_namespace());
            assert_eq!(target, Namespace::new("testdb", "dest"));
            assert_eq!(outcome, Err(CoreError::WriteOutcomeUnknown));
        }
        other => panic!("expected AggregateRan, got {other:?}"),
    }
}

#[test]
fn a_run_is_bounded_just_above_the_server_time_limit() {
    let command = run_command(SessionId::new_v4(), out_pipeline());
    let server = Duration::from_millis(crate::pipeline::AGGREGATE_RUN_MAX_TIME_MS);
    let limit = handlers::command_time_limit(&RuntimeConfig::default(), &command);
    assert!(
        limit > server && limit < Duration::from_secs(120),
        "{limit:?}"
    );
    let short = RuntimeConfig {
        index_build_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    assert_eq!(
        handlers::command_time_limit(&short, &command),
        Duration::from_millis(50) + config::AGGREGATE_RUN_COUNT_TIMEOUT
    );
}

#[test]
fn the_command_is_a_session_write_with_its_own_name() {
    let session = SessionId::new_v4();
    let command = run_command(session, out_pipeline());
    assert!(command.is_write());
    assert_eq!(command.context(), "aggregate_run");
    assert_eq!(command.session(), Some(session));
    assert_eq!(command.request(), None);
}

#[tokio::test]
async fn a_preview_still_never_forwards_a_write_stage() {
    let store = Arc::new(InMemoryStore::new());
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(Command::AggregatePreview {
        session,
        request: 2,
        ns: mock_namespace(),
        pipeline: out_pipeline(),
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregatePreviewed {
            outcome: Err(CoreError::InvalidDocument(_)),
            ..
        }
    ));
    assert!(store.received_pipelines().is_empty());
    assert!(store.written_pipelines().is_empty());
}

/// A store whose write run succeeds and whose count of the target fails.
struct CountFails(Option<CoreError>);

#[async_trait::async_trait]
impl DocumentStore for CountFails {
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
        Ok(())
    }
    async fn estimated_count(&self, _ns: &Namespace) -> Result<u64> {
        match &self.0 {
            Some(error) => Err(error.clone()),
            None => std::future::pending().await,
        }
    }
    async fn aggregate_write(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _max_time: Duration,
    ) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_run_stays_a_success_when_only_the_count_cannot_be_read() {
    let store = CountFails(Some(CoreError::Database("count".into())));
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    match next(&mut h).await {
        Event::AggregateRan { outcome, run, .. } => {
            assert_eq!(outcome, Ok(None));
            assert_eq!(run, 4);
        }
        other => panic!("expected AggregateRan, got {other:?}"),
    }
}

#[tokio::test]
async fn a_lost_connection_while_counting_keeps_the_known_success() {
    let store = CountFails(Some(CoreError::Unreachable));
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, out_pipeline())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateRan {
            run: 4,
            outcome: Ok(None),
            ..
        }
    ));
}

#[tokio::test]
async fn a_count_outrunning_its_own_bound_leaves_the_success_without_a_count() {
    let store = CountFails(None);
    let limits = aggregate_run::RunLimits {
        write: Duration::from_secs(30),
        count: Duration::from_millis(50),
    };
    let event = aggregate_run::run(
        SessionId::new_v4(),
        &store,
        mock_namespace(),
        out_pipeline(),
        4,
        limits,
    )
    .await
    .unwrap();
    assert!(matches!(
        event,
        Event::AggregateRan {
            run: 4,
            outcome: Ok(None),
            ..
        }
    ));
}

#[tokio::test]
async fn the_write_bound_is_apart_from_the_count_bound() {
    let limits = aggregate_run::RunLimits {
        write: Duration::from_millis(50),
        count: Duration::from_secs(30),
    };
    let event = aggregate_run::run(
        SessionId::new_v4(),
        &HangingWrite,
        mock_namespace(),
        out_pipeline(),
        4,
        limits,
    )
    .await
    .unwrap();
    assert!(matches!(
        event,
        Event::AggregateRan {
            outcome: Err(CoreError::WriteOutcomeUnknown),
            ..
        }
    ));
}
