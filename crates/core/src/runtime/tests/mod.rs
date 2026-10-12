use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use super::*;
use crate::error::Result;
use crate::secrets::MemorySecretStore;
use crate::store::{PAGE_SIZE, Page};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc_with_id, mock_namespace};

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
    assert!(matches!(next(&mut h).await, Event::Disconnected { .. }));
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
        ..RuntimeConfig::default()
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
        ..RuntimeConfig::default()
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
        ..RuntimeConfig::default()
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

// ---- connection races (deterministic: gates, not sleeps) ----

mod races;
