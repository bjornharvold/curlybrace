//! The last check before a data answer is sent must not race with a new Connect/Disconnect.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::connection::ConnectionConfig;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

struct NoConnector;

#[async_trait]
impl Connector for NoConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

fn shared() -> Arc<Shared> {
    Arc::new(Shared {
        store: RwLock::new(None),
        tunnel: Mutex::new(None),
        generation: AtomicU64::new(0),
        epoch_lock: RwLock::new(()),
        secrets: None,
        connector: Arc::new(NoConnector),
        config: RuntimeConfig::default(),
    })
}

fn results() -> Event {
    Event::Results {
        ns: mock_namespace(),
        query: Query::default(),
        page: Page {
            docs: vec![],
            skip: 0,
            has_more: false,
        },
    }
}

fn superseded(context: &'static str) -> Event {
    Event::Failed {
        context,
        error: CoreError::Superseded,
    }
}

#[test]
fn an_answer_produced_before_a_new_epoch_becomes_superseded() {
    let s = shared();
    let before = s.begin_epoch(); // generation 1: the command's session
    let answer = results();
    assert_eq!(
        handlers::finalize(&s, before, "find", answer.clone()),
        answer
    );
    let after = s.begin_epoch(); // a newer Connect completes before finalize runs
    assert_eq!(after, before + 1);
    assert_eq!(
        handlers::finalize(&s, before, "find", answer),
        superseded("find")
    );
}

#[test]
fn same_generation_answers_are_unaffected_including_failures() {
    let s = shared();
    let generation = s.begin_epoch();
    let failed = Event::Failed {
        context: "find",
        error: CoreError::Database("x".into()),
    };
    assert_eq!(
        handlers::finalize(&s, generation, "find", failed.clone()),
        failed
    );
}

#[test]
fn begin_epoch_waits_for_an_answer_that_is_being_sent() {
    let s = shared();
    let generation = s.begin_epoch();
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));
    let sender = {
        let (s, start, release) = (s.clone(), start.clone(), release.clone());
        std::thread::spawn(move || {
            // Holds the same read lock the sender holds between the check and the send.
            let _guard = s.epoch_lock.read().unwrap();
            start.wait();
            release.wait();
            s.is_current(generation) // still current: the bump has not happened yet
        })
    };
    start.wait();
    let bumper = {
        let (s, finished) = (s.clone(), finished.clone());
        std::thread::spawn(move || {
            s.begin_epoch();
            finished.store(true, Ordering::SeqCst);
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !finished.load(Ordering::SeqCst),
        "begin_epoch must wait for the in-flight send"
    );
    release.wait();
    assert!(
        sender.join().unwrap(),
        "the check and the send saw one generation"
    );
    bumper.join().unwrap();
    assert!(finished.load(Ordering::SeqCst));
}

/// A Find racing a Connect: whatever the interleaving, the stale answer is either delivered
/// before the new connection's `Connected` or replaced by `Superseded`, never after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stale_data_answer_never_follows_the_new_connected() {
    for round in 0..300 {
        let store: Arc<dyn DocumentStore> = Arc::new(InMemoryStore::new());
        let (_, repaint) = (0, Arc::new(|| {}) as Repaint);
        let mut h = spawn_runtime_with_connector(
            None,
            Some(store),
            Arc::new(NoConnector),
            repaint,
            RuntimeConfig::default(),
        );
        h.tx.send(Command::Find {
            ns: mock_namespace(),
            query: Query::default(),
        })
        .unwrap();
        h.tx.send(Command::Connect(ConnectionConfig::new(
            "c",
            "mongodb://127.0.0.1:27017",
        )))
        .unwrap();
        let mut seen = Vec::new();
        for _ in 0..2 {
            let ev = tokio::time::timeout(Duration::from_secs(5), h.rx.recv())
                .await
                .expect("event")
                .expect("open");
            seen.push(ev);
        }
        if let Some(pos) = seen
            .iter()
            .position(|e| matches!(e, Event::Connected { .. }))
        {
            for later in &seen[pos + 1..] {
                assert!(
                    !matches!(later, Event::Results { .. }),
                    "round {round}: stale Results after Connected: {seen:?}"
                );
            }
        }
    }
}
