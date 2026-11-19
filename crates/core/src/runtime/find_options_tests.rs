//! A find the server refused for one of its options is an answer: it fails the command under its
//! own error, is never settled by a ping and never makes the session lost.

use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::connection::ConnectionConfig;
use crate::pipeline_options::{PipelineOptions, ReadPreferenceMode};
use crate::store::{Namespace, Page};
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);

/// A store whose finds fail with `error` and whose ping would say the server is gone.
struct RefusingStore {
    error: CoreError,
}

#[async_trait]
impl DocumentStore for RefusingStore {
    async fn ping(&self) -> Result<()> {
        Err(CoreError::Unreachable)
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        Err(self.error.clone())
    }
    async fn insert(&self, _ns: &Namespace, _doc: bson::Document) -> Result<bson::Bson> {
        Err(CoreError::Unsupported("not scripted".into()))
    }
    async fn replace(&self, _ns: &Namespace, _id: &bson::Bson, _doc: bson::Document) -> Result<()> {
        Err(CoreError::Unsupported("not scripted".into()))
    }
    async fn delete(&self, _ns: &Namespace, _id: &bson::Bson) -> Result<()> {
        Err(CoreError::Unsupported("not scripted".into()))
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

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn find_failure(error: CoreError) -> (CoreError, bool) {
    let cfg = ConnectionConfig::new("c", "mongodb://127.0.0.1:27017");
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(OneStore(Arc::new(RefusingStore { error }))),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    let query = Query {
        options: PipelineOptions {
            read_preference: Some(ReadPreferenceMode::Secondary),
            ..PipelineOptions::default()
        },
        ..Query::default()
    };
    h.tx.send(Command::Find {
        session: cfg.id,
        ns: mock_namespace(),
        query,
        request: 77,
    })
    .unwrap();
    // A find is a numbered read: its failure echoes the number it was sent with.
    let error = match next(&mut h).await {
        Event::RequestFailed {
            error,
            request: 77,
            context: "find",
            ..
        } => error,
        other => panic!("unexpected {other:?}"),
    };
    // The session is still usable: a ping-settled failure would have marked it lost.
    h.tx.send(Command::ListDatabases { session: cfg.id })
        .unwrap();
    let still_connected = matches!(next(&mut h).await, Event::Databases { .. });
    (error, still_connected)
}

#[tokio::test]
async fn a_find_answered_read_preference_unavailable_is_failed_not_lost() {
    let (error, still_connected) = find_failure(CoreError::ReadPreferenceUnavailable).await;
    assert_eq!(error, CoreError::ReadPreferenceUnavailable);
    assert!(still_connected);
}

#[tokio::test]
async fn a_hint_or_collation_refusal_is_not_settled_by_a_ping_either() {
    for typed in [CoreError::HintNotFound, CoreError::CollationRejected] {
        let (error, still_connected) = find_failure(typed.clone()).await;
        assert_eq!(error, typed);
        assert!(still_connected);
    }
}
