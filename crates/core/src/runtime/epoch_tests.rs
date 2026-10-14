//! The last check before a data answer is sent must not race with a new Connect/Disconnect.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, RwLock};
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
        sessions: RwLock::new(Default::default()),
        next_epoch: AtomicU64::new(0),
        secrets: None,
        connector: Arc::new(NoConnector),
        config: RuntimeConfig::default(),
        compare: Default::default(),
    })
}

/// Starts the next epoch of the one session these tests use.
fn begin(s: &Shared) -> (Arc<Session>, u64) {
    s.begin_session_epoch(EMBEDDED_SESSION)
}

fn results() -> Event {
    Event::Results {
        session: EMBEDDED_SESSION,
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
        session: EMBEDDED_SESSION,
        context,
        error: CoreError::Superseded,
    }
}

#[test]
fn an_answer_produced_before_a_new_epoch_becomes_superseded() {
    let s = shared();
    let (session, before) = begin(&s); // generation 1: the command's epoch
    let answer = results();
    assert_eq!(
        handlers::finalize(&session, EMBEDDED_SESSION, before, "find", answer.clone()),
        answer
    );
    let (_, after) = begin(&s); // a newer Connect completes before finalize runs
    assert_eq!(after, before + 1);
    assert_eq!(
        handlers::finalize(&session, EMBEDDED_SESSION, before, "find", answer),
        superseded("find")
    );
}

#[test]
fn same_generation_answers_are_unaffected_including_failures() {
    let s = shared();
    let (session, generation) = begin(&s);
    let failed = Event::Failed {
        session: EMBEDDED_SESSION,
        context: "find",
        error: CoreError::Database("x".into()),
    };
    assert_eq!(
        handlers::finalize(
            &session,
            EMBEDDED_SESSION,
            generation,
            "find",
            failed.clone()
        ),
        failed
    );
}

#[test]
fn begin_epoch_waits_for_an_answer_that_is_being_sent() {
    let s = shared();
    let (session, generation) = begin(&s);
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));
    let sender = {
        let (session, start, release) = (session.clone(), start.clone(), release.clone());
        std::thread::spawn(move || {
            // Holds the same read lock the sender holds between the check and the send.
            let _guard = session.epoch_lock.read().unwrap();
            start.wait();
            release.wait();
            session.is_current(generation) // still current: the bump has not happened yet
        })
    };
    start.wait();
    let bumper = {
        let (s, finished) = (s.clone(), finished.clone());
        std::thread::spawn(move || {
            begin(&s);
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
            session: EMBEDDED_SESSION,
            ns: mock_namespace(),
            query: Query::default(),
        })
        .unwrap();
        // The same session as the Find: only a Connect of that session can supersede it.
        let mut cfg = ConnectionConfig::new("c", "mongodb://127.0.0.1:27017");
        cfg.id = EMBEDDED_SESSION;
        h.tx.send(Command::Connect(cfg)).unwrap();
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
