//! `GuardCheck`: the report arrives as an outcome inside `Event::GuardChecked`; a failure or a
//! timeout is an `Err` outcome, never an `Event::Failed` (which would raise the error banner).

use std::time::Duration;

use bson::{Bson, Document};

use super::*;
use crate::connection::ConnectionConfig;
use crate::explain::VerdictKind;
use crate::guard::GuardTarget;
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
    bson::doc! { "queryPlanner": { "namespace": "db.c", "winningPlan": { "stage": "COLLSCAN" } } }
}

fn target() -> GuardTarget {
    GuardTarget {
        ns: mock_namespace(),
        method: "find".into(),
        query: Query {
            filter: bson::doc! { "status": "open" },
            ..Query::default()
        },
    }
}

fn guard_command(session: SessionId, targets: Vec<GuardTarget>) -> Command {
    Command::GuardCheck {
        session,
        request: 7,
        targets,
        min_docs: None,
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

#[tokio::test]
async fn guard_check_answers_with_a_report() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let (mut h, session) = connected_to(Arc::new(store), false, RuntimeConfig::default()).await;
    h.tx.send(guard_command(session, vec![target()])).unwrap();
    match next(&mut h).await {
        Event::GuardChecked {
            session: s,
            request,
            outcome,
        } => {
            assert_eq!(s, session);
            assert_eq!(request, 7);
            let report = outcome.expect("a report");
            assert_eq!(report.unchecked, 0);
            assert_eq!(report.findings.len(), 1);
            assert_eq!(report.findings[0].kind, VerdictKind::CollectionScan);
        }
        other => panic!("expected GuardChecked, got {other:?}"),
    }
}

#[tokio::test]
async fn a_failing_explain_leaves_every_target_unchecked() {
    let (mut h, session) = connected_to(
        Arc::new(InMemoryStore::new()),
        false,
        RuntimeConfig::default(),
    )
    .await;
    h.tx.send(guard_command(session, vec![target(), target()]))
        .unwrap();
    match next(&mut h).await {
        Event::GuardChecked { outcome, .. } => {
            let report = outcome.expect("a report");
            assert!(report.findings.is_empty());
            assert_eq!(report.unchecked, 2);
        }
        other => panic!("expected GuardChecked, got {other:?}"),
    }
}

#[tokio::test]
async fn guard_check_is_allowed_on_a_read_only_session() {
    let store = InMemoryStore::new().with_explain(&mock_namespace(), collscan());
    let (mut h, session) = connected_to(Arc::new(store), true, RuntimeConfig::default()).await;
    let command = guard_command(session, vec![target()]);
    assert!(!command.is_write());
    assert_eq!(command.context(), "guard_check");
    assert_eq!(command.session(), Some(session));
    assert_eq!(command.request(), Some(7));
    h.tx.send(command).unwrap();
    match next(&mut h).await {
        Event::GuardChecked { outcome, .. } => assert!(outcome.is_ok(), "{outcome:?}"),
        other => panic!("expected GuardChecked, got {other:?}"),
    }
}

/// A store whose explain never answers.
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
    async fn explain_plan(&self, _ns: &Namespace, _q: &Query) -> Result<crate::store::Explain> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn guard_timeout_is_an_outcome_not_a_failed_event() {
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let (mut h, session) = connected_to(Arc::new(HangingExplain), false, config).await;
    h.tx.send(guard_command(session, vec![target()])).unwrap();
    match next(&mut h).await {
        Event::GuardChecked {
            session: s,
            request,
            outcome,
        } => {
            assert_eq!(s, session);
            assert_eq!(request, 7);
            assert!(
                matches!(outcome, Err(CoreError::Database(_))),
                "{outcome:?}"
            );
        }
        other => panic!("expected GuardChecked, got {other:?}"),
    }
}
