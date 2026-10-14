//! Races between sessions: connect ordering, superseded answers, dropped receivers.
use super::*;
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
    embedded_config(name, "mongodb://127.0.0.1:27017")
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
        session: EMBEDDED_SESSION,
        context: "connect",
        error: CoreError::Superseded,
    }
}

#[tokio::test]
async fn disconnect_during_slow_connect_leaves_runtime_disconnected() {
    let (mut h, mut gates, _) = gated_runtime(&["slow"]);
    h.tx.send(Command::Connect(cfg("slow"))).unwrap();
    h.tx.send(Command::Disconnect {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Disconnected {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    gates
        .remove("slow")
        .unwrap()
        .send(Ok(store_with_db("late")))
        .ok();
    assert_eq!(next(&mut h).await, superseded());
    h.tx.send(Command::Find {
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
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
    assert!(matches!(
        next(&mut h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    gates
        .remove("first")
        .unwrap()
        .send(Ok(store_with_db("first_db")))
        .ok();
    assert_eq!(next(&mut h).await, superseded());
    h.tx.send(Command::ListDatabases {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Databases {
            session: EMBEDDED_SESSION,
            names: vec!["second_db".into()]
        }
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
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            epoch: 1,
            error: CoreError::Database("down".into())
        }
    );
    h.tx.send(Command::ListDatabases {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
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
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    started.notified().await;
    h.tx.send(Command::Disconnect {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Disconnected {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    release.notify_one();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "find",
            error: CoreError::Superseded
        }
    );
    assert_quiet(&mut h).await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn find_without_a_session_change_still_delivers_results() {
    let (store, started, release) = gated_find();
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(store, repaint);
    h.tx.send(Command::Find {
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    started.notified().await;
    release.notify_one();
    assert!(matches!(
        next(&mut h).await,
        Event::Results {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    assert_quiet(&mut h).await;
}

/// `list_databases` waits for a release; everything else is trivial.
struct GatedListStore {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl DocumentStore for GatedListStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(vec!["old".into()])
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
async fn list_databases_finishing_after_a_newer_connect_is_superseded() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let old: Arc<dyn DocumentStore> = Arc::new(GatedListStore {
        started: started.clone(),
        release: release.clone(),
    });
    let (connector, mut gates) = GatedConnector::new(&["next"]);
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_connector(
        None,
        Some(old),
        connector,
        repaint,
        RuntimeConfig::default(),
    );
    h.tx.send(Command::ListDatabases {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    started.notified().await;
    h.tx.send(Command::Connect(cfg("next"))).unwrap();
    release.notify_one();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "list_databases",
            error: CoreError::Superseded
        }
    );
    gates
        .remove("next")
        .unwrap()
        .send(Ok(store_with_db("new_db")))
        .ok();
    assert!(matches!(
        next(&mut h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    h.tx.send(Command::ListDatabases {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Databases {
            session: EMBEDDED_SESSION,
            names: vec!["new_db".into()]
        },
        "commands of the new session are unaffected"
    );
    assert_quiet(&mut h).await;
}

#[tokio::test]
async fn dropped_receiver_does_not_panic_or_hang_handlers() {
    let (store, started, release) = gated_find();
    let finished = store.finished.clone();
    let (count, repaint) = counter();
    let RuntimeHandle { tx, rx } = spawn_runtime_with_store(store, repaint);
    drop(rx);
    tx.send(Command::Find {
        session: EMBEDDED_SESSION,
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
    assert!(
        tx.send(Command::ListDatabases {
            session: EMBEDDED_SESSION
        })
        .is_ok()
    );
    let (_, repaint) = counter();
    let mut fresh = spawn_runtime_with_store(seeded(), repaint);
    fresh
        .tx
        .send(Command::ListDatabases {
            session: EMBEDDED_SESSION,
        })
        .unwrap();
    assert!(matches!(next(&mut fresh).await, Event::Databases { .. }));
}

#[tokio::test]
async fn connect_with_embedded_password_fails_validation_without_echo() {
    let (_, repaint) = counter();
    let mut h = spawn_runtime(Arc::new(MemorySecretStore::new()), repaint);
    let c = embedded_config("x", "mongodb://bob:hunter2@host");
    h.tx.send(Command::Connect(c)).unwrap();
    match next(&mut h).await {
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            error: error @ CoreError::InvalidUri(_),
            ..
        } => {
            assert!(!error.to_string().contains("hunter2"));
            assert!(!format!("{error:?}").contains("hunter2"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn epochs_identify_the_request_each_outcome_belongs_to() {
    let (mut h, mut gates, _) = gated_runtime(&["a", "b"]);
    h.tx.send(Command::Connect(cfg("a"))).unwrap(); // epoch 1
    h.tx.send(Command::Connect(cfg("b"))).unwrap(); // epoch 2
    gates
        .remove("b")
        .unwrap()
        .send(Ok(store_with_db("b_db")))
        .ok();
    assert_eq!(
        next(&mut h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 2
        }
    );
    h.tx.send(Command::Disconnect {
        session: EMBEDDED_SESSION,
    })
    .unwrap(); // epoch 3
    assert_eq!(
        next(&mut h).await,
        Event::Disconnected {
            session: EMBEDDED_SESSION,
            epoch: 3
        }
    );
    gates
        .remove("a")
        .unwrap()
        .send(Ok(store_with_db("a_db")))
        .ok();
    assert_eq!(next(&mut h).await, superseded());
    assert_quiet(&mut h).await;
}

#[tokio::test]
async fn a_replaced_connect_reports_superseded_even_when_its_own_error_is_real() {
    let (mut h, mut gates, _) = gated_runtime(&["a", "b"]);
    h.tx.send(Command::Connect(cfg("a"))).unwrap();
    h.tx.send(Command::Connect(cfg("b"))).unwrap();
    // A fails with a genuine error AFTER B was requested: still Superseded.
    gates.remove("a").unwrap().send(down()).ok();
    assert_eq!(next(&mut h).await, superseded());
    // B is still current: its real error is reported as such.
    gates.remove("b").unwrap().send(down()).ok();
    assert_eq!(
        next(&mut h).await,
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            epoch: 2,
            error: CoreError::Database("down".into())
        }
    );
    assert_quiet(&mut h).await;
}

#[tokio::test]
async fn a_current_connect_reports_its_real_validation_error() {
    let (mut h, _gates, _) = gated_runtime(&[]);
    let bad = embedded_config("", "mongodb://127.0.0.1:27017");
    h.tx.send(Command::Connect(bad)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            error: CoreError::InvalidName,
            ..
        }
    ));
}

/// `find` waits for a release and then panics.
struct GatedPanicStore {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl DocumentStore for GatedPanicStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        self.started.notify_one();
        self.release.notified().await;
        panic!("boom")
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

async fn panicking_find(disconnect_first: bool) -> Event {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let store = Arc::new(GatedPanicStore {
        started: started.clone(),
        release: release.clone(),
    });
    let (_, repaint) = counter();
    let mut h = spawn_runtime_with_store(store, repaint);
    h.tx.send(Command::Find {
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    started.notified().await;
    if disconnect_first {
        h.tx.send(Command::Disconnect {
            session: EMBEDDED_SESSION,
        })
        .unwrap();
        assert!(matches!(
            next(&mut h).await,
            Event::Disconnected {
                session: EMBEDDED_SESSION,
                ..
            }
        ));
    }
    release.notify_one();
    next(&mut h).await
}

#[tokio::test]
async fn a_panic_in_a_replaced_session_is_superseded_otherwise_internal_error() {
    assert_eq!(
        panicking_find(true).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "find",
            error: CoreError::Superseded
        }
    );
    assert_eq!(
        panicking_find(false).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "find",
            error: CoreError::Database("internal error".into())
        }
    );
}

#[tokio::test]
async fn a_connect_failing_validation_still_consumes_an_epoch() {
    let (mut h, mut gates, _) = gated_runtime(&["ok"]);
    let bad = embedded_config("", "mongodb://127.0.0.1:27017");
    h.tx.send(Command::Connect(bad)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            epoch: 1,
            ..
        }
    ));
    h.tx.send(Command::Connect(cfg("ok"))).unwrap();
    gates
        .remove("ok")
        .unwrap()
        .send(Ok(store_with_db("d")))
        .ok();
    assert_eq!(
        next(&mut h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 2
        }
    );
    h.tx.send(Command::Disconnect {
        session: EMBEDDED_SESSION,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Disconnected {
            session: EMBEDDED_SESSION,
            epoch: 3
        }
    );
}

#[tokio::test]
async fn a_hanging_find_fails_at_the_command_timeout_even_with_a_long_connect_timeout() {
    let (_, repaint) = counter();
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(100),
        connect_timeout: Duration::from_secs(600),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_config(None, Some(Arc::new(SlowStore)), repaint, config);
    let start = Instant::now();
    h.tx.send(Command::Find {
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        query: Query::default(),
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "find",
            error: CoreError::Database("timeout".into())
        }
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_slow_connect_is_not_cut_at_the_command_timeout() {
    let (connector, mut gates) = GatedConnector::new(&["slow"]);
    let (_, repaint) = counter();
    let config = RuntimeConfig {
        command_timeout: Duration::from_millis(50),
        connect_timeout: Duration::from_secs(30),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_connector(None, None, connector, repaint, config);
    h.tx.send(Command::Connect(cfg("slow"))).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await; // well past command_timeout
    gates
        .remove("slow")
        .unwrap()
        .send(Ok(store_with_db("d")))
        .ok();
    assert_eq!(
        next(&mut h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 1
        }
    );
}

#[tokio::test]
async fn a_connect_that_exceeds_the_connect_timeout_fails_with_a_timeout() {
    let (connector, _gates) = GatedConnector::new(&["never"]);
    let (_, repaint) = counter();
    let config = RuntimeConfig {
        connect_timeout: Duration::from_millis(100),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_connector(None, None, connector, repaint, config);
    h.tx.send(Command::Connect(cfg("never"))).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            epoch: 1,
            error: CoreError::Database(_)
        }
    ));
}
