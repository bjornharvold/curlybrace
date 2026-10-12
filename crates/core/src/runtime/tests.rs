use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use super::*;
use crate::connection::{SshAuth, SshConfig};
use crate::error::Result;
use crate::secrets::MemorySecretStore;
use crate::store::{PAGE_SIZE, Page};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_connection, mock_doc_with_id, mock_namespace};

const WAIT: Duration = Duration::from_secs(5);

struct FailingStore;

fn down<T>() -> Result<T> {
    Err(CoreError::Database("down".into()))
}

#[async_trait]
impl DocumentStore for FailingStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        down()
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        down()
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

/// Sleeps far longer than any test timeout override.
struct SlowStore;

#[async_trait]
impl DocumentStore for SlowStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        tokio::time::sleep(Duration::from_secs(30)).await;
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

/// Panics inside the handler task.
struct PanicStore;

#[async_trait]
impl DocumentStore for PanicStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        panic!("boom")
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        panic!("boom")
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        panic!("boom")
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        panic!("boom")
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        panic!("boom")
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        panic!("boom")
    }
}

fn counter() -> (Arc<AtomicUsize>, Arc<dyn Fn() + Send + Sync>) {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    (
        count,
        Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }),
    )
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

fn one_of_each() -> Vec<Command> {
    let ns = mock_namespace();
    let id = Bson::Int32(1);
    vec![
        Command::ListDatabases,
        Command::ListCollections { db: "d".into() },
        Command::Find {
            ns: ns.clone(),
            query: Query::default(),
        },
        Command::Insert {
            ns: ns.clone(),
            json: "{\"a\":1}".into(),
        },
        Command::Replace {
            ns: ns.clone(),
            id: id.clone(),
            json: "{\"_id\":1}".into(),
        },
        Command::Delete { ns: ns.clone(), id },
        Command::Clone {
            ns,
            source: doc! { "a": 1 },
        },
    ]
}

fn seeded() -> Arc<InMemoryStore> {
    let ns = mock_namespace();
    Arc::new(
        InMemoryStore::new().with_docs(&ns, vec![mock_doc_with_id(1_i32), mock_doc_with_id(2_i32)]),
    )
}

#[tokio::test]
async fn list_databases_yields_databases() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(seeded(), repaint);
    h.tx.send(Command::ListDatabases).unwrap();
    assert_eq!(next(&mut h).await, Event::Databases(vec!["testdb".into()]));
}

#[tokio::test]
async fn list_collections_yields_names_with_db() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(seeded(), repaint);
    h.tx.send(Command::ListCollections {
        db: "testdb".into(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Collections {
            db: "testdb".into(),
            names: vec!["people".into()]
        }
    );
}

#[tokio::test]
async fn find_returns_results_with_same_ns_and_query() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(seeded(), repaint);
    let query = Query {
        filter: doc! { "n": 1 },
        skip: 0,
        limit: PAGE_SIZE,
        ..Query::default()
    };
    h.tx.send(Command::Find {
        ns: mock_namespace(),
        query: query.clone(),
    })
    .unwrap();
    match next(&mut h).await {
        Event::Results { ns, query: q, page } => {
            assert_eq!(ns, mock_namespace());
            assert_eq!(q, query);
            assert_eq!(page.docs.len(), 2);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn insert_replace_delete_clone_yield_mutated() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(seeded(), repaint);
    let ns = mock_namespace();
    let cmds = vec![
        Command::Insert {
            ns: ns.clone(),
            json: "{\"x\":1}".into(),
        },
        Command::Replace {
            ns: ns.clone(),
            id: Bson::Int32(1),
            json: "{\"_id\":1,\"x\":2}".into(),
        },
        Command::Clone {
            ns: ns.clone(),
            source: mock_doc_with_id(1_i32),
        },
        Command::Delete {
            ns: ns.clone(),
            id: Bson::Int32(2),
        },
    ];
    for c in cmds {
        h.tx.send(c).unwrap();
        assert_eq!(next(&mut h).await, Event::Mutated { ns: ns.clone() });
    }
}

#[tokio::test]
async fn invalid_json_fails_and_runtime_keeps_serving() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(seeded(), repaint);
    h.tx.send(Command::Insert {
        ns: mock_namespace(),
        json: "{not json".into(),
    })
    .unwrap();
    match next(&mut h).await {
        Event::Failed {
            context: "insert",
            error: CoreError::InvalidJson(_),
        } => {}
        other => panic!("unexpected {other:?}"),
    }
    h.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut h).await, Event::Databases(_)));
}

#[tokio::test]
async fn failing_store_fails_all_seven_store_commands_with_matching_context() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(Arc::new(FailingStore), repaint);
    let expected = [
        "list_databases",
        "list_collections",
        "find",
        "insert",
        "replace",
        "delete",
        "clone",
    ];
    for (cmd, ctx) in one_of_each().into_iter().zip(expected) {
        h.tx.send(cmd).unwrap();
        match next(&mut h).await {
            Event::Failed { context, error } => {
                assert_eq!(context, ctx);
                assert_eq!(error, CoreError::Database("down".into()));
            }
            other => panic!("{ctx}: unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn repaint_fires_exactly_once_per_event() {
    let (count, repaint) = counter();
    let mut h = spawn_runtime_with_store(Arc::new(FailingStore), repaint);
    let cmds = one_of_each();
    let n = cmds.len();
    for c in cmds {
        h.tx.send(c).unwrap();
    }
    for _ in 0..n {
        next(&mut h).await;
    }
    let deadline = Instant::now() + WAIT;
    while count.load(Ordering::SeqCst) < n && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count.load(Ordering::SeqCst), n);
    assert!(h.rx.try_recv().is_err(), "no extra events");
}

#[tokio::test]
async fn commands_before_connect_fail_with_static_not_connected() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime(Arc::new(MemorySecretStore::new()), repaint);
    h.tx.send(Command::ListDatabases).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            context: "list_databases",
            error: CoreError::Unsupported("not connected".into())
        }
    );
}

#[tokio::test]
async fn disconnect_clears_store_and_emits_disconnected() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(seeded(), repaint);
    h.tx.send(Command::Disconnect).unwrap();
    assert_eq!(next(&mut h).await, Event::Disconnected);
    h.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "list_databases",
            ..
        }
    ));
}

#[tokio::test]
async fn slow_store_times_out_within_override() {
    let (_, repaint) = counter();
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(100),
    };
    let mut h = spawn_runtime_with_config(None, Some(Arc::new(SlowStore)), repaint, config);
    let start = Instant::now();
    h.tx.send(Command::Find {
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            context: "find",
            error: CoreError::Database("timeout".into())
        }
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn slow_command_does_not_block_others() {
    let (_, repaint) = counter();
    let config = RuntimeConfig {
        command_timeout: Duration::from_secs(10),
    };
    let mut h = spawn_runtime_with_config(None, Some(Arc::new(SlowStore)), repaint, config);
    h.tx.send(Command::Find {
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    h.tx.send(Command::ListDatabases).unwrap();
    assert_eq!(next(&mut h).await, Event::Databases(vec![]));
}

#[tokio::test]
async fn panicking_handler_still_yields_one_failed_event() {
    let (count, repaint) = counter();
    let mut h = spawn_runtime_with_store(Arc::new(PanicStore), repaint);
    h.tx.send(Command::ListDatabases).unwrap();
    match next(&mut h).await {
        Event::Failed {
            context: "list_databases",
            error: CoreError::Database(m),
        } => {
            assert!(!m.contains("boom"), "panic payload must not leak");
        }
        other => panic!("unexpected {other:?}"),
    }
    h.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut h).await, Event::Failed { .. }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert!(h.rx.try_recv().is_err());
}

#[tokio::test]
async fn connect_to_unreachable_host_fails_within_bound() {
    let (_, repaint) = counter();
    let config = RuntimeConfig {
        command_timeout: Duration::from_secs(2),
    };
    let mut h = spawn_runtime_with_config(
        Some(Arc::new(MemorySecretStore::new())),
        None,
        repaint,
        config,
    );
    let cfg = ConnectionConfig::new("x", "mongodb://127.0.0.1:1");
    let start = Instant::now();
    h.tx.send(Command::Connect(cfg)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "connect",
            error: CoreError::Database(_)
        }
    ));
    assert!(start.elapsed() < Duration::from_secs(4));
    h.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "list_databases",
            ..
        }
    ));
}

#[tokio::test]
async fn connect_with_invalid_uri_fails_with_invalid_uri() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime(Arc::new(MemorySecretStore::new()), repaint);
    h.tx.send(Command::Connect(ConnectionConfig::new("x", "not-a-uri")))
        .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "connect",
            error: CoreError::InvalidUri(_)
        }
    ));
}

#[tokio::test]
async fn tunnel_seam_is_none_without_ssh_and_unsupported_with_it() {
    let plain = mock_connection();
    assert_eq!(open_tunnel_if_configured(&plain).await, Ok(None));
    let mut ssh = mock_connection();
    ssh.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth: SshAuth::Password,
    });
    assert!(matches!(
        open_tunnel_if_configured(&ssh).await,
        Err(CoreError::Unsupported(_))
    ));
}

// ---- connection races (deterministic: gates, not sleeps) ----

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::{Notify, oneshot};

type Opened = Result<Arc<dyn DocumentStore>>;

/// Connector whose result for each cfg name is released by the test.
struct GatedConnector {
    gates: Mutex<HashMap<String, oneshot::Receiver<Opened>>>,
}

impl GatedConnector {
    fn new(names: &[&str]) -> (Arc<Self>, HashMap<String, oneshot::Sender<Opened>>) {
        let mut gates = HashMap::new();
        let mut senders = HashMap::new();
        for name in names {
            let (tx, rx) = oneshot::channel();
            gates.insert(name.to_string(), rx);
            senders.insert(name.to_string(), tx);
        }
        (
            Arc::new(Self {
                gates: Mutex::new(gates),
            }),
            senders,
        )
    }
}

#[async_trait]
impl Connector for GatedConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        _p: Option<&str>,
        _t: Option<std::net::SocketAddr>,
    ) -> Opened {
        let gate = self.gates.lock().unwrap().remove(&cfg.name).expect("gate");
        gate.await.unwrap_or_else(|_| down())
    }
}

fn gated_runtime(
    names: &[&str],
) -> (
    RuntimeHandle,
    HashMap<String, oneshot::Sender<Opened>>,
    Arc<AtomicUsize>,
) {
    let (connector, senders) = GatedConnector::new(names);
    let (count, repaint) = counter();
    let h = spawn_runtime_with_connector(None, None, connector, repaint, RuntimeConfig::default());
    (h, senders, count)
}

fn cfg(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, "mongodb://127.0.0.1:27017")
}

fn store_with_db(db: &str) -> Arc<dyn DocumentStore> {
    Arc::new(InMemoryStore::new().with_docs(&crate::store::Namespace::new(db, "c"), vec![]))
}

async fn assert_quiet(h: &mut RuntimeHandle) {
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.rx.try_recv().is_err(), "unexpected extra event");
}

fn superseded() -> Event {
    Event::Failed {
        context: "connect",
        error: CoreError::Unsupported("connection superseded".into()),
    }
}

#[tokio::test]
async fn disconnect_during_slow_connect_leaves_runtime_disconnected() {
    let (mut h, mut gates, _) = gated_runtime(&["slow"]);
    h.tx.send(Command::Connect(cfg("slow"))).unwrap();
    h.tx.send(Command::Disconnect).unwrap();
    assert_eq!(next(&mut h).await, Event::Disconnected);
    gates
        .remove("slow")
        .unwrap()
        .send(Ok(store_with_db("late")))
        .ok();
    assert_eq!(next(&mut h).await, superseded());
    h.tx.send(Command::Find {
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            context: "find",
            error: CoreError::Unsupported("not connected".into())
        }
    );
    assert_quiet(&mut h).await;
}

#[tokio::test]
async fn second_requested_connect_wins_even_if_first_finishes_last() {
    let (mut h, mut gates, _) = gated_runtime(&["first", "second"]);
    h.tx.send(Command::Connect(cfg("first"))).unwrap();
    h.tx.send(Command::Connect(cfg("second"))).unwrap();
    gates
        .remove("second")
        .unwrap()
        .send(Ok(store_with_db("second_db")))
        .ok();
    assert_eq!(next(&mut h).await, Event::Connected);
    gates
        .remove("first")
        .unwrap()
        .send(Ok(store_with_db("first_db")))
        .ok();
    assert_eq!(next(&mut h).await, superseded());
    h.tx.send(Command::ListDatabases).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Databases(vec!["second_db".into()])
    );
    assert_quiet(&mut h).await;
}

#[tokio::test]
async fn failed_connect_leaves_previous_store_cleared() {
    let (connector, mut senders) = GatedConnector::new(&["bad"]);
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_connector(
        None,
        Some(seeded()),
        connector,
        repaint,
        RuntimeConfig::default(),
    );
    h.tx.send(Command::Connect(cfg("bad"))).unwrap();
    senders.remove("bad").unwrap().send(down()).ok();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            context: "connect",
            error: CoreError::Database("down".into())
        }
    );
    h.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            error: CoreError::Unsupported(_),
            ..
        }
    ));
}

/// Find that signals when it started and waits for a release.
struct GatedFindStore {
    started: Arc<Notify>,
    release: Arc<Notify>,
    finished: Arc<AtomicUsize>,
}

#[async_trait]
impl DocumentStore for GatedFindStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, q: &Query) -> Result<Page> {
        self.started.notify_one();
        self.release.notified().await;
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(Page {
            docs: vec![],
            skip: q.skip,
            has_more: false,
        })
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

fn gated_find() -> (Arc<GatedFindStore>, Arc<Notify>, Arc<Notify>) {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let store = Arc::new(GatedFindStore {
        started: started.clone(),
        release: release.clone(),
        finished: Arc::new(AtomicUsize::new(0)),
    });
    (store, started, release)
}

#[tokio::test]
async fn disconnect_during_find_still_emits_exactly_one_event_for_each() {
    let (store, started, release) = gated_find();
    let (count, repaint) = counter();
    let mut h = spawn_runtime_with_store(store, repaint);
    h.tx.send(Command::Find {
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    started.notified().await;
    h.tx.send(Command::Disconnect).unwrap();
    assert_eq!(next(&mut h).await, Event::Disconnected);
    release.notify_one();
    assert!(matches!(next(&mut h).await, Event::Results { .. }));
    assert_quiet(&mut h).await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn dropped_receiver_does_not_panic_or_hang_handlers() {
    let (store, started, release) = gated_find();
    let finished = store.finished.clone();
    let (count, repaint) = counter();
    let RuntimeHandle { tx, rx } = spawn_runtime_with_store(store, repaint);
    drop(rx);
    tx.send(Command::Find {
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    started.notified().await;
    release.notify_one();
    let deadline = Instant::now() + WAIT;
    while finished.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        finished.load(Ordering::SeqCst),
        1,
        "handler ran to completion"
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "no repaint without a receiver"
    );
    // The runtime loop is still alive and accepts commands.
    assert!(tx.send(Command::ListDatabases).is_ok());
    let (_, repaint) = counter();
    let mut fresh = spawn_runtime_with_store(seeded(), repaint);
    fresh.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut fresh).await, Event::Databases(_)));
}

#[tokio::test]
async fn connect_with_ssh_config_is_unsupported_through_runtime() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime(Arc::new(MemorySecretStore::new()), repaint);
    let mut c = cfg("ssh");
    c.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth: SshAuth::Password,
    });
    h.tx.send(Command::Connect(c)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "connect",
            error: CoreError::Unsupported(_)
        }
    ));
}

#[tokio::test]
async fn connect_with_embedded_password_fails_validation_without_echo() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime(Arc::new(MemorySecretStore::new()), repaint);
    let c = ConnectionConfig::new("x", "mongodb://bob:hunter2@host");
    h.tx.send(Command::Connect(c)).unwrap();
    match next(&mut h).await {
        Event::Failed {
            context: "connect",
            error: error @ CoreError::InvalidUri(_),
        } => {
            assert!(!error.to_string().contains("hunter2"));
            assert!(!format!("{error:?}").contains("hunter2"));
        }
        other => panic!("unexpected {other:?}"),
    }
}
