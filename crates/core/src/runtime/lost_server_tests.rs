//! A suspect failure of a data command (a lost connection, or the command outrunning its bound)
//! is settled by a short ping of the same store: unreachable when the ping fails.

use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document};

use super::*;
use crate::connection::ConnectionConfig;
use crate::store::{Namespace, Page};
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(100);

#[derive(Clone, Copy)]
enum Ping {
    Answers,
    Refuses,
    Hangs,
}

/// A store whose data commands all fail with `error` (or hang first), and whose ping answers as
/// told.
struct SuspectStore {
    error: CoreError,
    hang_first: bool,
    ping: Ping,
}

impl SuspectStore {
    async fn fail<T>(&self) -> Result<T> {
        if self.hang_first {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        Err(self.error.clone())
    }
}

#[async_trait]
impl DocumentStore for SuspectStore {
    async fn ping(&self) -> Result<()> {
        match self.ping {
            Ping::Answers => Ok(()),
            Ping::Refuses => Err(CoreError::Unreachable),
            Ping::Hangs => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            }
        }
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.fail().await
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        self.fail().await
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        self.fail().await
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        self.fail().await
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        self.fail().await
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        self.fail().await
    }
}

struct OneStore(Arc<dyn DocumentStore>);

#[async_trait]
impl Connector for OneStore {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

/// The failure of a `find` against `store`, after a real connect.
async fn failed_find(store: SuspectStore) -> CoreError {
    let cfg = ConnectionConfig::new("lost", "mongodb://127.0.0.1:27017");
    let config = RuntimeConfig {
        command_timeout: SHORT,
        ping_timeout: SHORT,
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(OneStore(Arc::new(store))),
        Arc::new(|| {}),
        config,
    );
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    h.tx.send(Command::Find {
        session: cfg.id,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    match next(&mut h).await {
        Event::Failed { error, .. } => error,
        other => panic!("unexpected {other:?}"),
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

fn interrupted(ping: Ping) -> SuspectStore {
    SuspectStore {
        error: CoreError::ConnectionInterrupted("connection reset".into()),
        hang_first: false,
        ping,
    }
}

#[tokio::test]
async fn an_interrupted_connection_with_a_live_server_is_an_ordinary_database_error() {
    assert_eq!(
        failed_find(interrupted(Ping::Answers)).await,
        CoreError::Database("connection reset".into())
    );
}

#[tokio::test]
async fn an_interrupted_connection_and_a_failed_ping_is_unreachable() {
    assert_eq!(
        failed_find(interrupted(Ping::Refuses)).await,
        CoreError::Unreachable
    );
}

#[tokio::test]
async fn an_interrupted_connection_and_a_ping_that_hangs_is_unreachable() {
    assert_eq!(
        failed_find(interrupted(Ping::Hangs)).await,
        CoreError::Unreachable
    );
}
