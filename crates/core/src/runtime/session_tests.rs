//! Several connections at once: each has its own store, tunnel and epoch, all epochs come from
//! one counter.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document};

use super::tests::gated::{down, gated_find};
use super::*;
use crate::connection::ConnectionConfig;
use crate::store::{Namespace, Page, Topology};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);

/// Hands out the store registered for the connection's id.
struct PerIdConnector {
    stores: HashMap<SessionId, Arc<dyn DocumentStore>>,
}

#[async_trait]
impl Connector for PerIdConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        self.stores
            .get(&cfg.id)
            .cloned()
            .ok_or_else(|| CoreError::Database("unknown connection".into()))
    }
}

fn cfg(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, "mongodb://127.0.0.1:27017")
}

fn store_with_db(db: &str) -> Arc<dyn DocumentStore> {
    Arc::new(InMemoryStore::new().with_docs(&Namespace::new(db, "c"), vec![]))
}

fn start(stores: Vec<(&ConnectionConfig, Arc<dyn DocumentStore>)>) -> RuntimeHandle {
    let connector = PerIdConnector {
        stores: stores.into_iter().map(|(c, s)| (c.id, s)).collect(),
    };
    spawn_runtime_with_connector(
        None,
        None,
        Arc::new(connector),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    )
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connect(h: &mut RuntimeHandle, c: &ConnectionConfig) -> u64 {
    h.tx.send(Command::Connect(c.clone())).unwrap();
    match next(h).await {
        Event::Connected { session, epoch, .. } => {
            assert_eq!(session, c.id);
            epoch
        }
        other => panic!("unexpected {other:?}"),
    }
}

async fn databases(h: &mut RuntimeHandle, session: SessionId) -> Event {
    h.tx.send(Command::ListDatabases { session }).unwrap();
    next(h).await
}

fn names(session: SessionId, names: &[&str]) -> Event {
    Event::Databases {
        session,
        names: names.iter().map(|n| n.to_string()).collect(),
    }
}

#[tokio::test]
async fn two_sessions_stay_connected_independently() {
    let (a, b) = (cfg("a"), cfg("b"));
    let mut h = start(vec![
        (&a, store_with_db("a_db")),
        (&b, store_with_db("b_db")),
    ]);
    connect(&mut h, &a).await;
    connect(&mut h, &b).await;
    assert_eq!(databases(&mut h, a.id).await, names(a.id, &["a_db"]));
    assert_eq!(databases(&mut h, b.id).await, names(b.id, &["b_db"]));
}

#[tokio::test]
async fn disconnecting_one_session_leaves_the_other_answering() {
    let (a, b) = (cfg("a"), cfg("b"));
    let mut h = start(vec![
        (&a, store_with_db("a_db")),
        (&b, store_with_db("b_db")),
    ]);
    connect(&mut h, &a).await;
    connect(&mut h, &b).await;
    h.tx.send(Command::Disconnect { session: a.id }).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Disconnected { session, .. } if session == a.id
    ));
    assert_eq!(databases(&mut h, b.id).await, names(b.id, &["b_db"]));
    assert!(matches!(
        databases(&mut h, a.id).await,
        Event::Failed { session, error: CoreError::Unsupported(_), .. } if session == a.id
    ));
}

fn find(session: SessionId) -> Command {
    Command::Find {
        session,
        ns: mock_namespace(),
        query: Query::default(),
    }
}

#[tokio::test]
async fn reconnecting_one_session_does_not_supersede_the_other() {
    let (a, b) = (cfg("a"), cfg("b"));
    let (slow, started, release) = gated_find();
    let mut h = start(vec![(&a, store_with_db("a_db")), (&b, slow)]);
    connect(&mut h, &a).await;
    connect(&mut h, &b).await;
    h.tx.send(find(b.id)).unwrap();
    started.notified().await;
    connect(&mut h, &a).await;
    release.notify_one();
    assert!(matches!(
        next(&mut h).await,
        Event::Results { session, .. } if session == b.id
    ));
}

#[tokio::test]
async fn a_data_command_for_an_unknown_session_fails_closed_for_that_session() {
    let a = cfg("a");
    let mut h = start(vec![(&a, store_with_db("a_db"))]);
    connect(&mut h, &a).await;
    let stranger = SessionId::new_v4();
    h.tx.send(find(stranger)).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: stranger,
            context: "find",
            error: CoreError::Unsupported("not connected".into()),
        }
    );
}

/// Counts every call that reaches it; answers like an empty server.
struct CountingStore {
    calls: Arc<AtomicUsize>,
}

impl CountingStore {
    fn counted(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl DocumentStore for CountingStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.counted();
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        self.counted();
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, q: &Query) -> Result<Page> {
        self.counted();
        Ok(Page {
            docs: vec![],
            skip: q.skip,
            has_more: false,
        })
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        self.counted();
        Ok(Bson::Null)
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        self.counted();
        Ok(())
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        self.counted();
        Ok(())
    }
}

#[tokio::test]
async fn a_data_command_for_an_unknown_session_never_reads_another_sessions_store() {
    let b = cfg("b");
    let calls = Arc::new(AtomicUsize::new(0));
    let counting = Arc::new(CountingStore {
        calls: calls.clone(),
    });
    let mut h = start(vec![(&b, counting)]);
    connect(&mut h, &b).await;
    let a = SessionId::new_v4();
    let commands = [find(a), Command::ListDatabases { session: a }];
    for command in commands {
        let context = command.context();
        h.tx.send(command).unwrap();
        assert_eq!(
            next(&mut h).await,
            Event::Failed {
                session: a,
                context,
                error: CoreError::Unsupported("not connected".into()),
            }
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn a_late_answer_of_a_disconnected_session_is_superseded() {
    let a = cfg("a");
    let (slow, started, release) = gated_find();
    let mut h = start(vec![(&a, slow)]);
    connect(&mut h, &a).await;
    h.tx.send(find(a.id)).unwrap();
    started.notified().await;
    h.tx.send(Command::Disconnect { session: a.id }).unwrap();
    assert!(matches!(next(&mut h).await, Event::Disconnected { .. }));
    release.notify_one();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: a.id,
            context: "find",
            error: CoreError::Superseded,
        }
    );
}

#[tokio::test]
async fn epochs_never_repeat_across_sessions_or_reconnects() {
    let (a, b) = (cfg("a"), cfg("b"));
    let mut h = start(vec![
        (&a, store_with_db("a_db")),
        (&b, store_with_db("b_db")),
    ]);
    let e1 = connect(&mut h, &a).await;
    let e2 = connect(&mut h, &b).await;
    h.tx.send(Command::Disconnect { session: a.id }).unwrap();
    let e3 = match next(&mut h).await {
        Event::Disconnected { epoch, .. } => epoch,
        other => panic!("unexpected {other:?}"),
    };
    let e4 = connect(&mut h, &a).await;
    assert_eq!([e1, e2, e3, e4], [1, 2, 3, 4]);
}

#[test]
fn command_and_event_report_their_session() {
    let id = SessionId::new_v4();
    assert_eq!(find(id).session(), Some(id));
    assert_eq!(Command::Disconnect { session: id }.session(), Some(id));
    let connect = cfg("c");
    assert_eq!(
        Command::Connect(connect.clone()).session(),
        Some(connect.id)
    );
    let test = Command::TestConnection {
        request_id: 1,
        cfg: connect,
        secrets: TestSecrets::default(),
    };
    assert_eq!(test.session(), None);
    assert_eq!(
        Event::Mutated {
            session: id,
            ns: mock_namespace()
        }
        .session(),
        Some(id)
    );
    assert_eq!(
        Event::Connected {
            session: id,
            epoch: 1,
            topology: None
        }
        .session(),
        Some(id)
    );
    let test_event = Event::TestConnection {
        request_id: 1,
        outcome: Err(CoreError::Superseded),
    };
    assert_eq!(test_event.session(), None);
}

/// A store that only answers the topology question, with a fixed outcome.
struct TopologyStore {
    answer: std::result::Result<Option<Topology>, ()>,
}

#[async_trait]
impl DocumentStore for TopologyStore {
    async fn topology(&self) -> Result<Option<Topology>> {
        self.answer.map_err(|()| CoreError::Database("down".into()))
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        down()
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        down()
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        down()
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        down()
    }
}

async fn connected_topology(answer: std::result::Result<Option<Topology>, ()>) -> Event {
    let c = cfg("t");
    let mut h = start(vec![(&c, Arc::new(TopologyStore { answer }))]);
    h.tx.send(Command::Connect(c)).unwrap();
    next(&mut h).await
}

#[tokio::test]
async fn topology_of_the_store_is_reported_on_connect() {
    let event = connected_topology(Ok(Some(Topology::ReplicaSet))).await;
    assert!(matches!(
        event,
        Event::Connected {
            topology: Some(Topology::ReplicaSet),
            ..
        }
    ));
}

#[tokio::test]
async fn a_store_without_topology_connects_with_none() {
    let c = cfg("m");
    let mut h = start(vec![(&c, store_with_db("d"))]);
    h.tx.send(Command::Connect(c)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Connected { topology: None, .. }
    ));
}

#[tokio::test]
async fn a_failing_topology_read_never_fails_the_connect() {
    let event = connected_topology(Err(())).await;
    assert!(matches!(event, Event::Connected { topology: None, .. }));
}

/// Topology read that never answers.
struct HangingTopologyStore;

#[async_trait]
impl DocumentStore for HangingTopologyStore {
    async fn topology(&self) -> Result<Option<Topology>> {
        std::future::pending().await
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        down()
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        down()
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        down()
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        down()
    }
}

#[tokio::test]
async fn a_hanging_topology_read_is_cut_at_the_command_timeout_and_still_connects() {
    let c = cfg("slow");
    let connector = PerIdConnector {
        stores: HashMap::from([(
            c.id,
            Arc::new(HangingTopologyStore) as Arc<dyn DocumentStore>,
        )]),
    };
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(connector),
        Arc::new(|| {}),
        RuntimeConfig {
            command_timeout: Duration::from_millis(100),
            connect_timeout: Duration::from_secs(3),
            ..RuntimeConfig::default()
        },
    );
    h.tx.send(Command::Connect(c.clone())).unwrap();
    let event = next(&mut h).await;
    assert!(
        matches!(event, Event::Connected { topology: None, .. }),
        "{event:?}"
    );
    assert!(matches!(
        databases(&mut h, c.id).await,
        Event::Databases { .. }
    ));
}

/// A store whose server became unreachable after the connect.
struct LostServerStore;

#[async_trait]
impl DocumentStore for LostServerStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Err(CoreError::Unreachable)
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Err(CoreError::Unreachable)
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        Err(CoreError::Unreachable)
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        Err(CoreError::Unreachable)
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        Err(CoreError::Unreachable)
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        Err(CoreError::Unreachable)
    }
}

#[tokio::test]
async fn a_find_on_a_lost_server_is_answered_with_a_typed_unreachable_failure() {
    let c = cfg("lost");
    let mut h = start(vec![(&c, Arc::new(LostServerStore))]);
    connect(&mut h, &c).await;
    h.tx.send(Command::Find {
        session: c.id,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: c.id,
            context: "find",
            error: CoreError::Unreachable,
        }
    );
}
