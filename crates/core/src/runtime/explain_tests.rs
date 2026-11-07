//! `ExplainFind`: the verdict arrives as an outcome inside `Event::Explain`; a failure is an
//! `Err` outcome, never an `Event::Failed` (which would raise the error banner).

use std::time::Duration;

use bson::{Bson, Document};

use super::*;
use crate::connection::ConnectionConfig;
use crate::ejson::editor_json_to_doc;
use crate::explain::VerdictKind;
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
        Event::Explain { outcome, .. } => {
            assert!(matches!(outcome, Err(CoreError::Database(_))), "{outcome:?}");
        }
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

/// A store whose explain never answers, as a slow `executionStats` run would not.
struct HangingExplain;

#[async_trait::async_trait]
impl DocumentStore for HangingExplain {
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
    async fn explain_find(&self, _ns: &Namespace, _q: &Query) -> Result<crate::store::Explain> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn explain_timeout_is_an_outcome_not_a_failed_event() {
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(HangingExplain))),
        Arc::new(|| {}),
        RuntimeConfig {
            command_timeout: Duration::from_millis(50),
            ..RuntimeConfig::default()
        },
    );
    let cfg = ConnectionConfig::new("c", "mongodb://localhost:27017");
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    h.tx.send(explain_command(cfg.id)).unwrap();
    match next(&mut h).await {
        Event::Explain { session, outcome, .. } => {
            assert_eq!(session, cfg.id);
            assert!(matches!(outcome, Err(CoreError::Database(_))), "{outcome:?}");
        }
        other => panic!("expected Explain, got {other:?}"),
    }
}
