//! Read-only enforcement on data sessions: the connect-time flag, the live lock and the
//! reconnect rule (a fresh flag per connect, so an unticked config is writable again).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

use async_trait::async_trait;
use bson::{Bson, Document};
use uuid::Uuid;

use super::read_only_registry::ReadOnlyRegistry;
use super::*;
use crate::compare::protocol::{CompareCommand, Side};
use crate::store::Page;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc_with_id, mock_index_spec, mock_namespace};

const WAIT: Duration = Duration::from_secs(5);

/// An in-memory store that counts the writes that reach it.
struct CountingStore {
    inner: InMemoryStore,
    writes: Arc<AtomicUsize>,
}

#[async_trait]
impl DocumentStore for CountingStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }
    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.delete(ns, id).await
    }
    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.inner.estimated_count(ns).await
    }
    async fn list_indexes(&self, ns: &Namespace) -> Result<Vec<crate::store::CollectionIndex>> {
        self.inner.list_indexes(ns).await
    }
    async fn create_index(&self, ns: &Namespace, spec: &crate::indexes::IndexSpec) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.create_index(ns, spec).await
    }
    async fn drop_index(&self, ns: &Namespace, name: &str) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.drop_index(ns, name).await
    }
    async fn set_index_hidden(&self, ns: &Namespace, name: &str, hidden: bool) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set_index_hidden(ns, name, hidden).await
    }
    async fn set_index_ttl(&self, ns: &Namespace, name: &str, seconds: i64) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.set_index_ttl(ns, name, seconds).await
    }
}

/// Hands out a fresh counting store per connect, all sharing one write counter.
struct CountingConnector {
    writes: Arc<AtomicUsize>,
}

#[async_trait]
impl Connector for CountingConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        let seeded = InMemoryStore::new().with_docs(
            &mock_namespace(),
            vec![mock_doc_with_id(1_i32), mock_doc_with_id(2_i32)],
        );
        Ok(Arc::new(CountingStore {
            inner: seeded,
            writes: self.writes.clone(),
        }))
    }
}

fn spawn() -> (RuntimeHandle, Arc<AtomicUsize>) {
    let writes = Arc::new(AtomicUsize::new(0));
    let connector = Arc::new(CountingConnector {
        writes: writes.clone(),
    });
    let handle = spawn_runtime_with_connector(
        None,
        None,
        connector,
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    (handle, writes)
}

fn config(read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

async fn connect(h: &mut RuntimeHandle, cfg: &ConnectionConfig) {
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(h).await, Event::Connected { .. }));
}

fn write_commands(session: SessionId) -> Vec<Command> {
    let ns = mock_namespace();
    vec![
        Command::Insert {
            session,
            ns: ns.clone(),
            json: "{\"x\":1}".into(),
        },
        Command::Replace {
            session,
            ns: ns.clone(),
            id: Bson::Int32(1),
            json: "{\"_id\":1,\"x\":2}".into(),
        },
        Command::Delete {
            session,
            ns: ns.clone(),
            id: Bson::Int32(2),
        },
        Command::Clone {
            session,
            ns: ns.clone(),
            source: mock_doc_with_id(1_i32),
        },
        Command::CreateIndex {
            session,
            ns: ns.clone(),
            spec: mock_index_spec(),
        },
        Command::DropIndex {
            session,
            ns: ns.clone(),
            name: "a_1".into(),
        },
        Command::SetIndexHidden {
            session,
            ns: ns.clone(),
            name: "a_1".into(),
            hidden: true,
        },
        Command::SetIndexTtl {
            session,
            ns,
            name: "a_1".into(),
            seconds: 60,
        },
    ]
}

fn read_commands(session: SessionId) -> Vec<Command> {
    vec![
        Command::ListDatabases { session },
        Command::ListCollections {
            session,
            db: "d".into(),
        },
        Command::Find {
            session,
            ns: mock_namespace(),
            query: Query::default(),
        },
        Command::ListIndexes {
            session,
            ns: mock_namespace(),
        },
        Command::IndexHistory {
            session,
            ns: mock_namespace(),
            range: crate::history::ChartRange::Day,
        },
        Command::EstimatedCount {
            session,
            ns: mock_namespace(),
        },
    ]
}

async fn assert_each_write_refused(h: &mut RuntimeHandle, session: SessionId) {
    for command in write_commands(session) {
        let context = command.context();
        h.tx.send(command).unwrap();
        assert_eq!(
            next(h).await,
            Event::Failed {
                session,
                context,
                error: CoreError::ReadOnly
            }
        );
    }
}

#[tokio::test]
async fn connect_with_read_only_config_refuses_each_write_command() {
    let (mut h, writes) = spawn();
    let cfg = config(true);
    connect(&mut h, &cfg).await;
    assert_each_write_refused(&mut h, cfg.id).await;
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn reads_still_work_on_a_read_only_session() {
    let (mut h, _) = spawn();
    let cfg = config(true);
    connect(&mut h, &cfg).await;
    for command in read_commands(cfg.id) {
        h.tx.send(command).unwrap();
        let event = next(&mut h).await;
        assert!(
            matches!(
                event,
                Event::Databases { .. }
                    | Event::Collections { .. }
                    | Event::Results { .. }
                    | Event::Indexes { .. }
                    | Event::IndexHistory { .. }
                    | Event::EstimatedCount { .. }
            ),
            "{event:?}"
        );
    }
}

#[tokio::test]
async fn lock_read_only_refuses_writes_on_an_open_session() {
    let (mut h, writes) = spawn();
    let cfg = config(false);
    connect(&mut h, &cfg).await;
    h.tx.send(write_commands(cfg.id).remove(0)).unwrap();
    assert!(matches!(next(&mut h).await, Event::Mutated { .. }));
    assert_eq!(writes.load(Ordering::SeqCst), 1);

    h.tx.send(Command::LockReadOnly { connection: cfg.id })
        .unwrap();
    assert_each_write_refused(&mut h, cfg.id).await;
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn reconnect_with_unticked_config_is_writable_again() {
    let (mut h, writes) = spawn();
    let locked = config(true);
    connect(&mut h, &locked).await;
    h.tx.send(Command::LockReadOnly {
        connection: locked.id,
    })
    .unwrap();
    assert_each_write_refused(&mut h, locked.id).await;

    let unticked = ConnectionConfig {
        read_only: false,
        ..locked.clone()
    };
    connect(&mut h, &unticked).await;
    h.tx.send(write_commands(unticked.id).remove(0)).unwrap();
    assert!(matches!(next(&mut h).await, Event::Mutated { .. }));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lock_for_unknown_connection_is_a_no_op() {
    let (mut h, _) = spawn();
    h.tx.send(Command::LockReadOnly {
        connection: Uuid::new_v4(),
    })
    .unwrap();
    // Any event the lock produced would arrive before the answer of the next command.
    let cfg = config(false);
    connect(&mut h, &cfg).await;
    h.tx.send(Command::ListDatabases { session: cfg.id })
        .unwrap();
    assert!(matches!(next(&mut h).await, Event::Databases { .. }));
    assert!(h.rx.try_recv().is_err());
}

#[test]
fn registry_gives_each_attach_a_fresh_flag_and_prunes_dead_ones() {
    let registry = ReadOnlyRegistry::default();
    let id = Uuid::new_v4();
    let first = registry.attach(id, true);
    let second = registry.attach(id, false);
    assert!(first.load(Ordering::SeqCst));
    assert!(!second.load(Ordering::SeqCst));
    registry.lock(id);
    assert!(second.load(Ordering::SeqCst));
    drop(first);
    drop(second);
    let third = registry.attach(id, false);
    assert!(!third.load(Ordering::SeqCst));
    assert_eq!(registry.live_count(id), 1);
    registry.lock(Uuid::new_v4());
    assert!(!third.load(Ordering::SeqCst));
}

/// How the read-only rule treats a command, classified here independently of `is_write`.
#[derive(Debug, PartialEq, Eq)]
enum Class {
    /// Refused on a read-only session.
    Write,
    /// A data command that only reads.
    Read,
    /// Not a data command (session lifecycle, compare, connection test, the lock itself).
    Other,
}

/// Every `Command` variant, no wildcard: a new variant does not compile until it is classified.
const COMMAND_VARIANTS: usize = 21;

fn classify(command: &Command) -> Class {
    match command {
        Command::Insert { .. }
        | Command::Replace { .. }
        | Command::Delete { .. }
        | Command::Clone { .. }
        | Command::CreateIndex { .. }
        | Command::DropIndex { .. }
        | Command::SetIndexHidden { .. }
        | Command::SetIndexTtl { .. } => Class::Write,
        Command::ListDatabases { .. }
        | Command::ListCollections { .. }
        | Command::Find { .. }
        | Command::ListIndexes { .. }
        | Command::IndexHistory { .. }
        | Command::EstimatedCount { .. } => Class::Read,
        Command::Connect(_)
        | Command::Disconnect { .. }
        | Command::Compare(_)
        | Command::TestConnection { .. }
        | Command::LockReadOnly { .. }
        | Command::SetSamplingExcluded { .. }
        | Command::PurgeHistory { .. } => Class::Other,
    }
}

#[tokio::test]
async fn is_write_agrees_with_what_the_read_only_store_refuses() {
    let (mut h, _) = spawn();
    let cfg = config(true);
    connect(&mut h, &cfg).await;
    for command in write_commands(cfg.id) {
        assert_eq!(classify(&command), Class::Write);
        assert!(command.is_write());
        h.tx.send(command).unwrap();
        assert!(matches!(
            next(&mut h).await,
            Event::Failed {
                error: CoreError::ReadOnly,
                ..
            }
        ));
    }
    for command in read_commands(cfg.id) {
        assert_eq!(classify(&command), Class::Read);
        assert!(!command.is_write());
        h.tx.send(command).unwrap();
        assert!(!matches!(next(&mut h).await, Event::Failed { .. }));
    }
    let others = [
        Command::Connect(cfg.clone()),
        Command::Disconnect { session: cfg.id },
        Command::Compare(CompareCommand::Databases {
            session: 1,
            side: Side::Source,
        }),
        Command::TestConnection {
            request_id: 1,
            cfg: cfg.clone(),
            secrets: TestSecrets::default(),
        },
        Command::LockReadOnly { connection: cfg.id },
        Command::SetSamplingExcluded {
            databases: Default::default(),
        },
        Command::PurgeHistory { connection: cfg.id },
    ];
    assert!(
        others
            .iter()
            .all(|c| classify(c) == Class::Other && !c.is_write())
    );
    let samples: Vec<Command> = write_commands(cfg.id)
        .into_iter()
        .chain(read_commands(cfg.id))
        .chain(others.iter().cloned())
        .collect();
    let variants: std::collections::HashSet<_> =
        samples.iter().map(std::mem::discriminant).collect();
    assert_eq!(variants.len(), COMMAND_VARIANTS, "every variant is sampled");
    let lock = &others[4];
    assert_eq!(lock.context(), "lock_read_only");
    assert_eq!(lock.session(), None);
}

/// Blocks every connect until the test releases it.
struct GatedConnector {
    writes: Arc<AtomicUsize>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl Connector for GatedConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        CountingConnector {
            writes: self.writes.clone(),
        }
        .connect(cfg, password, tunnel)
        .await
    }
}

#[tokio::test]
async fn lock_during_in_flight_connect_yields_a_locked_store() {
    let writes = Arc::new(AtomicUsize::new(0));
    let (release, gate) = oneshot::channel();
    let connector = Arc::new(GatedConnector {
        writes: writes.clone(),
        gate: Mutex::new(Some(gate)),
    });
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        connector,
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let cfg = config(false);
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    h.tx.send(Command::LockReadOnly { connection: cfg.id })
        .unwrap();
    // Let both commands reach the dispatcher while the connect is still blocked.
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.send(()).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    assert_each_write_refused(&mut h, cfg.id).await;
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lock_on_one_connection_leaves_another_writable() {
    let (mut h, writes) = spawn();
    let (a, b) = (config(false), config(false));
    connect(&mut h, &a).await;
    connect(&mut h, &b).await;
    h.tx.send(Command::LockReadOnly { connection: a.id })
        .unwrap();
    assert_each_write_refused(&mut h, a.id).await;
    h.tx.send(write_commands(b.id).remove(0)).unwrap();
    assert!(matches!(next(&mut h).await, Event::Mutated { .. }));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lock_reaches_an_open_store_keyed_by_id_alone() {
    // Nothing but the id is known to the lock: no saved connection is consulted.
    let (mut h, writes) = spawn();
    let cfg = ConnectionConfig {
        name: "unsaved".into(),
        ..config(false)
    };
    connect(&mut h, &cfg).await;
    h.tx.send(Command::LockReadOnly {
        connection: Uuid::from_u128(cfg.id.as_u128()),
    })
    .unwrap();
    assert_each_write_refused(&mut h, cfg.id).await;
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}
