//! `Find`, `ExplainFind` and `EstimatedCount` are numbered reads: the answer, and a failure,
//! echo the `request` the command carried, so a caller with several collections open can tell
//! whose answer arrived (#159).

use std::time::Duration;

use bson::doc;

use super::*;
use crate::connection::ConnectionConfig;
use crate::guard::GuardTarget;
use crate::store::{Namespace, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc_with_id, mock_namespace};

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

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connected() -> (RuntimeHandle, SessionId) {
    let store = InMemoryStore::new().with_docs(
        &mock_namespace(),
        vec![mock_doc_with_id(1), mock_doc_with_id(2)],
    );
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(store))),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let cfg = ConnectionConfig::new("c", "mongodb://localhost:27017");
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    (h, cfg.id)
}

#[tokio::test]
async fn find_results_echo_the_request() {
    let (mut h, session) = connected().await;
    h.tx.send(Command::Find {
        session,
        ns: mock_namespace(),
        query: Query::default(),
        request: 4711,
    })
    .unwrap();
    match next(&mut h).await {
        Event::Results { request, page, .. } => {
            assert_eq!(request, 4711);
            assert_eq!(page.docs.len(), 2);
        }
        other => panic!("expected Results, got {other:?}"),
    }
}

#[tokio::test]
async fn a_failed_find_is_a_request_failure_with_its_number() {
    // No store is connected for this session: the find fails as a command.
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(InMemoryStore::new()))),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let session = uuid::Uuid::new_v4();
    h.tx.send(Command::Find {
        session,
        ns: mock_namespace(),
        query: Query::default(),
        request: 31,
    })
    .unwrap();
    match next(&mut h).await {
        Event::RequestFailed {
            context, request, ..
        } => assert_eq!((context, request), ("find", 31)),
        other => panic!("expected RequestFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn explain_echoes_the_request() {
    let (mut h, session) = connected().await;
    h.tx.send(Command::ExplainFind {
        session,
        ns: mock_namespace(),
        query: Query {
            filter: doc! { "x": 1 },
            ..Query::default()
        },
        request: 52,
    })
    .unwrap();
    match next(&mut h).await {
        Event::Explain { request, .. } => assert_eq!(request, 52),
        other => panic!("expected Explain, got {other:?}"),
    }
}

#[tokio::test]
async fn estimated_count_echoes_the_request() {
    let (mut h, session) = connected().await;
    h.tx.send(Command::EstimatedCount {
        session,
        ns: mock_namespace(),
        request: 63,
    })
    .unwrap();
    match next(&mut h).await {
        Event::EstimatedCount { request, count, .. } => assert_eq!((request, count), (63, 2)),
        other => panic!("expected EstimatedCount, got {other:?}"),
    }
}

#[test]
fn the_three_reads_are_numbered_commands() {
    let ns = Namespace::new("a", "b");
    let session = uuid::Uuid::nil();
    let find = Command::Find {
        session,
        ns: ns.clone(),
        query: Query::default(),
        request: 1,
    };
    let explain = Command::ExplainFind {
        session,
        ns: ns.clone(),
        query: Query::default(),
        request: 2,
    };
    let count = Command::EstimatedCount {
        session,
        ns,
        request: 3,
    };
    assert_eq!(
        [find.request(), explain.request(), count.request()],
        [Some(1), Some(2), Some(3)]
    );
}

/// Every numbered command, through a real runtime: its answer echoes `Command::request()`.
#[tokio::test]
async fn event_request_echoes_command_request_for_every_numbered_pair() {
    let (mut h, session) = connected().await;
    let ns = mock_namespace();
    let commands = vec![
        Command::Find {
            session,
            ns: ns.clone(),
            query: Query::default(),
            request: 101,
        },
        Command::ExplainFind {
            session,
            ns: ns.clone(),
            query: Query::default(),
            request: 102,
        },
        Command::EstimatedCount {
            session,
            ns: ns.clone(),
            request: 103,
        },
        Command::ListIndexes {
            session,
            ns: ns.clone(),
            request: 104,
        },
        Command::IndexHistory {
            session,
            ns: ns.clone(),
            range: crate::history::ChartRange::Day,
            request: 105,
        },
        Command::QueryShapes {
            session,
            ns: ns.clone(),
            range: crate::history::ChartRange::Day,
            request: 106,
        },
        Command::GuardCheck {
            session,
            request: 107,
            targets: vec![GuardTarget {
                ns: ns.clone(),
                method: "find".into(),
                query: Query::default(),
            }],
            min_docs: None,
        },
        Command::CollectionStats {
            session,
            ns,
            request: 108,
        },
    ];
    for command in commands {
        let sent = command.request();
        assert!(sent.is_some(), "{command:?} is numbered");
        let context = command.context();
        h.tx.send(command).unwrap();
        let event = next(&mut h).await;
        assert_eq!(event.request(), sent, "{context}: {event:?}");
    }
}

#[test]
fn event_request_is_none_for_unnumbered_events() {
    let session = uuid::Uuid::nil();
    for event in [
        Event::Mutated {
            session,
            ns: mock_namespace(),
        },
        Event::Databases {
            session,
            names: vec![],
        },
        Event::Failed {
            session,
            context: "find",
            error: CoreError::Superseded,
        },
    ] {
        assert_eq!(event.request(), None, "{event:?}");
    }
}
