//! `ExplainFind`: the verdict arrives as an outcome inside `Event::Explain`; a failure is an
//! `Err` outcome, never an `Event::Failed` (which would raise the error banner).

use std::time::Duration;

use bson::Document;

use super::*;
use crate::connection::ConnectionConfig;
use crate::ejson::editor_json_to_doc;
use crate::explain::VerdictKind;
use crate::store::Query;
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

fn collscan() -> Document {
    editor_json_to_doc(include_str!("../explain/fixtures/collscan.json")).expect("fixture parses")
}

fn filtered() -> Query {
    Query {
        filter: bson::doc! { "status": "open" },
        ..Query::default()
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connected(store: InMemoryStore, read_only: bool) -> (RuntimeHandle, SessionId) {
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(store))),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let cfg = ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    };
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    (h, cfg.id)
}

fn explain_command(session: SessionId) -> Command {
    Command::ExplainFind {
        session,
        ns: mock_namespace(),
        query: filtered(),
    }
}

#[tokio::test]
async fn explain_find_answers_with_a_verdict() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let (mut h, session) = connected(store, false).await;
    h.tx.send(explain_command(session)).unwrap();
    match next(&mut h).await {
        Event::Explain {
            session: s,
            ns,
            query,
            outcome,
        } => {
            assert_eq!(s, session);
            assert_eq!(ns, mock_namespace());
            assert_eq!(query, filtered());
            let verdict = outcome.expect("a verdict");
            assert_eq!(verdict.kind, VerdictKind::CollectionScan);
            assert!(!verdict.plan_only);
        }
        other => panic!("expected Explain, got {other:?}"),
    }
}

#[tokio::test]
async fn explain_failure_is_an_outcome_not_a_failed_event() {
    let (mut h, session) = connected(InMemoryStore::new(), false).await;
    h.tx.send(explain_command(session)).unwrap();
    match next(&mut h).await {
        Event::Explain { outcome, .. } => {
            assert!(matches!(outcome, Err(CoreError::Unsupported(_))), "{outcome:?}");
        }
        other => panic!("expected Explain, got {other:?}"),
    }
}

#[tokio::test]
async fn unclassifiable_explain_is_an_outcome_not_a_failed_event() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), Document::new());
    let (mut h, session) = connected(store, false).await;
    h.tx.send(explain_command(session)).unwrap();
    match next(&mut h).await {
        Event::Explain { outcome, .. } => assert!(outcome.is_err(), "{outcome:?}"),
        other => panic!("expected Explain, got {other:?}"),
    }
}

#[tokio::test]
async fn explain_find_is_allowed_on_read_only_session() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let (mut h, session) = connected(store, true).await;
    let command = explain_command(session);
    assert!(!command.is_write());
    assert_eq!(command.context(), "explain_find");
    assert_eq!(command.session(), Some(session));
    h.tx.send(command).unwrap();
    match next(&mut h).await {
        Event::Explain { outcome, .. } => assert!(outcome.is_ok(), "{outcome:?}"),
        other => panic!("expected Explain, got {other:?}"),
    }
}
