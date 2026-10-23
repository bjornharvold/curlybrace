//! An index build is bound by `index_build_timeout`, every other command by `command_timeout`.

use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document};

use super::*;
use crate::indexes::IndexSpec;
use crate::store::Page;
use crate::test_support::mock_objects::mock_index_spec;

const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(100);
const BUILD: Duration = Duration::from_millis(400);

/// A store whose index writes take `delay` and whose reads and ping answer at once.
struct SlowIndexStore {
    delay: Duration,
}

#[async_trait]
impl DocumentStore for SlowIndexStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        Err(CoreError::Unsupported("find".into()))
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        Err(CoreError::Unsupported("insert".into()))
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        Err(CoreError::Unsupported("replace".into()))
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        Err(CoreError::Unsupported("delete".into()))
    }
    async fn create_index(&self, _ns: &Namespace, _spec: &IndexSpec) -> Result<()> {
        tokio::time::sleep(self.delay).await;
        Ok(())
    }
    async fn set_index_hidden(&self, _ns: &Namespace, _name: &str, _hidden: bool) -> Result<()> {
        tokio::time::sleep(self.delay).await;
        Ok(())
    }
}

fn target() -> Namespace {
    Namespace::new("db", "c")
}

/// The one event `command` answers with, after the store took `delay` per index write.
async fn answer(delay: Duration, command: impl FnOnce(SessionId) -> Command) -> Event {
    let config = RuntimeConfig {
        command_timeout: SHORT,
        index_build_timeout: BUILD,
        ping_timeout: SHORT,
        ..RuntimeConfig::default()
    };
    let store: Arc<dyn DocumentStore> = Arc::new(SlowIndexStore { delay });
    let mut h = spawn_runtime_with_config(None, Some(store), Arc::new(|| {}), config);
    h.tx.send(command(EMBEDDED_SESSION)).unwrap();
    let event = tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("an event within the bound")
        .expect("channel open");
    // Exactly one: nothing else follows.
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.rx.recv())
            .await
            .is_err(),
        "a second event arrived"
    );
    event
}

fn create(session: SessionId) -> Command {
    Command::CreateIndex {
        session,
        ns: target(),
        spec: mock_index_spec(),
    }
}

#[test]
fn the_default_index_build_bound_is_an_hour() {
    assert_eq!(INDEX_BUILD_TIMEOUT, Duration::from_secs(3600));
    assert_eq!(
        RuntimeConfig::default().index_build_timeout,
        INDEX_BUILD_TIMEOUT
    );
}

#[tokio::test]
async fn a_build_slower_than_the_command_timeout_still_answers_index_changed() {
    let event = answer(Duration::from_millis(250), create).await;
    assert_eq!(
        event,
        Event::IndexChanged {
            session: EMBEDDED_SESSION,
            ns: target()
        }
    );
}

#[tokio::test]
async fn a_build_slower_than_the_index_build_timeout_fails_once_as_an_unknown_outcome() {
    let event = answer(Duration::from_secs(30), create).await;
    assert_eq!(
        event,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "create_index",
            error: CoreError::WriteOutcomeUnknown,
        }
    );
}

#[tokio::test]
async fn other_commands_keep_the_command_timeout() {
    let event = answer(Duration::from_millis(250), |session| {
        Command::SetIndexHidden {
            session,
            ns: target(),
            name: "a_1".into(),
            hidden: true,
        }
    })
    .await;
    assert_eq!(
        event,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "set_index_hidden",
            error: CoreError::WriteOutcomeUnknown,
        }
    );
}
