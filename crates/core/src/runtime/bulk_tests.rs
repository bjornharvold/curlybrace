//! The bulk commands: classification, read-only refusal, outcomes, timeouts, currentness and a
//! cancel that reaches a write still in flight.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::Notify;

use super::*;
use crate::bulk::{
    BULK_PREVIEW_DOCS, BulkDeleteRequest, BulkOutcome, BulkRunId, BulkUpdateRequest, MatchScope,
    UpdatePreview, UpdateSpec,
};
use crate::store::Page;
use crate::test_support::mock_objects::{mock_doc_with_id, mock_namespace};

const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(100);

/// What the fake's `update_many` and `delete_many` do.
#[derive(Clone)]
enum Write {
    Answer(Result<BulkOutcome>),
    /// Never answers by itself.
    Hang,
    /// Answers `Unknown { cancelled: true }` once `cancel_bulk` is called.
    UntilCancelled,
    /// Answers `Applied` once the test notifies `release`.
    UntilReleased,
}

/// A store that counts its calls and behaves as told.
struct FakeStore {
    write: Write,
    count: Result<Option<u64>>,
    preview: Result<UpdatePreview>,
    calls: AtomicUsize,
    pings: AtomicUsize,
    entered: Notify,
    cancelled: Notify,
    release: Notify,
    /// Makes `count_matching` never answer.
    count_hangs: bool,
    /// While set, `is_read_only` blocks the calling thread until it receives.
    hold: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    held: Notify,
    queries: Mutex<Vec<Query>>,
}

impl FakeStore {
    fn new(write: Write) -> Arc<Self> {
        Arc::new(Self {
            write,
            count: Ok(Some(7)),
            preview: Ok(UpdatePreview {
                docs: vec![],
                gap: None,
            }),
            calls: AtomicUsize::new(0),
            pings: AtomicUsize::new(0),
            entered: Notify::new(),
            cancelled: Notify::new(),
            release: Notify::new(),
            count_hangs: false,
            hold: Mutex::new(None),
            held: Notify::new(),
            queries: Mutex::new(vec![]),
        })
    }

    fn with(self: Arc<Self>, edit: impl FnOnce(&mut Self)) -> Arc<Self> {
        let mut store = Arc::try_unwrap(self).ok().expect("unshared");
        edit(&mut store);
        Arc::new(store)
    }

    async fn write(&self) -> Result<BulkOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        match &self.write {
            Write::Answer(result) => result.clone(),
            Write::Hang => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(applied())
            }
            Write::UntilReleased => {
                self.release.notified().await;
                Ok(applied())
            }
            Write::UntilCancelled => {
                self.cancelled.notified().await;
                Ok(BulkOutcome::Unknown { cancelled: true })
            }
        }
    }
}

#[async_trait]
impl DocumentStore for FakeStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, q: &Query) -> Result<Page> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.queries.lock().unwrap().push(q.clone());
        Ok(Page {
            docs: vec![mock_doc_with_id(1_i32)],
            skip: 0,
            has_more: false,
        })
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
    fn is_read_only(&self) -> bool {
        let gate = self.hold.lock().unwrap().take();
        if let Some(gate) = gate {
            self.held.notify_one();
            let _ = gate.recv();
        }
        false
    }
    async fn ping(&self) -> Result<()> {
        self.pings.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn count_matching(
        &self,
        _ns: &Namespace,
        _filter: &Document,
        _max_time: Duration,
    ) -> Result<Option<u64>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.count_hangs {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        self.count.clone()
    }
    async fn preview_update(
        &self,
        _ns: &Namespace,
        _r: &BulkUpdateRequest,
    ) -> Result<UpdatePreview> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.preview.clone()
    }
    async fn update_many(
        &self,
        _ns: &Namespace,
        _req: &BulkUpdateRequest,
        _run: BulkRunId,
    ) -> Result<BulkOutcome> {
        self.write().await
    }
    async fn delete_many(
        &self,
        _ns: &Namespace,
        _req: &BulkDeleteRequest,
        _run: BulkRunId,
    ) -> Result<BulkOutcome> {
        self.write().await
    }
    async fn cancel_bulk(&self, _run: BulkRunId) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.cancelled.notify_one();
        Ok(())
    }
}

struct FakeConnector(Arc<FakeStore>);

#[async_trait]
impl Connector for FakeConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

fn applied() -> BulkOutcome {
    BulkOutcome::Applied {
        matched: 3,
        modified: 2,
        upserted: vec![],
        deleted: 0,
    }
}

fn update_request() -> BulkUpdateRequest {
    BulkUpdateRequest {
        filter: doc! { "a": 1 },
        update: UpdateSpec::Operators(doc! { "$set": { "b": 2 } }),
        upsert: false,
        scope: MatchScope::Filtered,
    }
}

fn delete_request() -> BulkDeleteRequest {
    BulkDeleteRequest {
        filter: doc! { "a": 1 },
        scope: MatchScope::Filtered,
    }
}

fn run_id() -> BulkRunId {
    BulkRunId(uuid::Uuid::new_v4())
}

fn config(read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    }
}

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        command_timeout: SHORT,
        bulk_write_timeout: SHORT,
        ping_timeout: SHORT,
        ..RuntimeConfig::default()
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

/// A runtime connected (through a connector) to `store`.
async fn connected(store: &Arc<FakeStore>, read_only: bool) -> (RuntimeHandle, SessionId) {
    connected_with(store, read_only, runtime_config()).await
}

async fn connected_with(
    store: &Arc<FakeStore>,
    read_only: bool,
    runtime: RuntimeConfig,
) -> (RuntimeHandle, SessionId) {
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FakeConnector(store.clone())),
        Arc::new(|| {}),
        runtime,
    );
    let cfg = config(read_only);
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    (h, cfg.id)
}

async fn no_more_events(h: &mut RuntimeHandle) {
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.rx.recv())
            .await
            .is_err(),
        "a second event arrived"
    );
}

fn all_bulk_commands(session: SessionId) -> Vec<Command> {
    let ns = mock_namespace();
    vec![
        Command::BulkCount {
            session,
            ns: ns.clone(),
            filter: doc! {},
            request: 1,
        },
        Command::BulkSample {
            session,
            ns: ns.clone(),
            filter: doc! {},
            request: 2,
        },
        Command::BulkPreview {
            session,
            ns: ns.clone(),
            request: update_request(),
            request_id: 3,
        },
        Command::BulkUpdate {
            session,
            ns: ns.clone(),
            request: update_request(),
            run: run_id(),
        },
        Command::BulkDelete {
            session,
            ns,
            request: delete_request(),
            run: run_id(),
        },
        Command::BulkCancel {
            session,
            run: run_id(),
        },
    ]
}

#[test]
fn bulk_commands_are_classified_as_writes_or_reads() {
    let expected = [
        (false, "bulk_count", Some(1)),
        (false, "bulk_sample", Some(2)),
        (true, "bulk_preview", None),
        (true, "bulk_update", None),
        (true, "bulk_delete", None),
        (false, "bulk_cancel", None),
    ];
    let session = uuid::Uuid::new_v4();
    for (command, (write, context, request)) in all_bulk_commands(session).iter().zip(expected) {
        assert_eq!(command.is_write(), write, "{context}");
        assert_eq!(command.context(), context);
        assert_eq!(command.session(), Some(session));
        assert_eq!(command.request(), request, "{context}");
    }
}

#[test]
fn the_default_bulk_write_bound_is_an_hour() {
    assert_eq!(BULK_WRITE_TIMEOUT, Duration::from_secs(3600));
    assert_eq!(
        RuntimeConfig::default().bulk_write_timeout,
        BULK_WRITE_TIMEOUT
    );
}

#[tokio::test]
async fn read_only_session_refuses_preview_update_and_delete_before_the_store() {
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, true).await;
    for command in all_bulk_commands(session)
        .into_iter()
        .filter(Command::is_write)
    {
        let context = command.context();
        h.tx.send(command).unwrap();
        assert_eq!(
            next(&mut h).await,
            Event::Failed {
                session,
                context,
                error: CoreError::ReadOnly
            }
        );
    }
    assert_eq!(store.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bulk_count_is_allowed_on_read_only() {
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, true).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! { "a": 1 },
        request: 5,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkCount {
            session,
            ns: mock_namespace(),
            request: 5,
            count: Some(7)
        }
    );
}

#[tokio::test]
async fn bulk_count_answers_none_when_the_count_timed_out() {
    let store = FakeStore::new(Write::Hang).with(|s| s.count = Ok(None));
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! {},
        request: 6,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::BulkCount {
            request: 6,
            count: None,
            ..
        }
    ));
}

#[tokio::test]
async fn bulk_count_failure_echoes_its_request() {
    let store =
        FakeStore::new(Write::Hang).with(|s| s.count = Err(CoreError::Database("x".into())));
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! {},
        request: 8,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::RequestFailed {
            context: "bulk_count",
            request: 8,
            ..
        }
    ));
}

#[tokio::test]
async fn bulk_sample_reads_the_first_documents_with_the_filter_alone() {
    let store = FakeStore::new(Write::Hang);
    let (mut h, session) = connected(&store, true).await;
    h.tx.send(Command::BulkSample {
        session,
        ns: mock_namespace(),
        filter: doc! { "a": 1 },
        request: 4,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkSample {
            session,
            ns: mock_namespace(),
            request: 4,
            docs: vec![mock_doc_with_id(1_i32)],
        }
    );
    let expected = Query {
        filter: doc! { "a": 1 },
        limit: BULK_PREVIEW_DOCS as u64,
        ..Query::default()
    };
    assert_eq!(*store.queries.lock().unwrap(), vec![expected]);
}

#[tokio::test]
async fn bulk_update_answers_finished_with_the_store_outcome() {
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(applied())
        }
    );
    no_more_events(&mut h).await;
}

#[tokio::test]
async fn bulk_delete_and_store_errors_are_outcomes_of_finished() {
    let error = CoreError::Database("scope".into());
    let store = FakeStore::new(Write::Answer(Err(error.clone())));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Err(error)
        }
    );
    assert_eq!(store.pings.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bulk_write_timeout_answers_unknown_not_failed() {
    let store = FakeStore::new(Write::Hang);
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(BulkOutcome::Unknown { cancelled: false })
        }
    );
    no_more_events(&mut h).await;
    assert_eq!(store.pings.load(Ordering::SeqCst), 0, "no settle by ping");
}

#[tokio::test]
async fn a_lost_connection_answers_unknown_without_a_ping() {
    let lost = CoreError::ConnectionInterrupted("reset".into());
    let store = FakeStore::new(Write::Answer(Err(lost)));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::BulkFinished {
            outcome: Ok(BulkOutcome::Unknown { cancelled: false }),
            ..
        }
    ));
    no_more_events(&mut h).await;
    assert_eq!(store.pings.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn preview_failures_are_inline_outcomes() {
    for error in [
        CoreError::Database("boom".into()),
        CoreError::ConnectionInterrupted("reset".into()),
    ] {
        let store = FakeStore::new(Write::Hang).with(|s| s.preview = Err(error.clone()));
        let (mut h, session) = connected(&store, false).await;
        h.tx.send(Command::BulkPreview {
            session,
            ns: mock_namespace(),
            request: update_request(),
            request_id: 11,
        })
        .unwrap();
        assert_eq!(
            next(&mut h).await,
            Event::BulkPreview {
                session,
                ns: mock_namespace(),
                request_id: 11,
                outcome: Err(error)
            }
        );
        assert_eq!(store.pings.load(Ordering::SeqCst), 0);
    }
}

/// The old store's thread is held inside its read-only check, between the runtime reading the
/// store and the currentness guard, while a newer `Connect` installs the new store.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn superseded_epoch_does_not_run_a_bulk_write() {
    let old = FakeStore::new(Write::Answer(Ok(applied())));
    let new = FakeStore::new(Write::Answer(Ok(applied())));
    let (release, gate) = std::sync::mpsc::channel();
    *old.hold.lock().unwrap() = Some(gate);
    let old_store: Arc<dyn DocumentStore> = old.clone();
    let mut h = spawn_runtime_with_connector(
        None,
        Some(old_store),
        Arc::new(FakeConnector(new.clone())),
        Arc::new(|| {}),
        runtime_config(),
    );
    h.tx.send(Command::BulkUpdate {
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        request: update_request(),
        run: run_id(),
    })
    .unwrap();
    tokio::time::timeout(WAIT, old.held.notified())
        .await
        .expect("the command reached the old store");
    h.tx.send(Command::Connect(ConnectionConfig {
        id: EMBEDDED_SESSION,
        ..config(false)
    }))
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "bulk_update",
            error: CoreError::Superseded,
            ..
        }
    ));
    assert_eq!(old.calls.load(Ordering::SeqCst), 0);
    assert_eq!(new.calls.load(Ordering::SeqCst), 0);
}

/// A write that completes after its session was replaced is still reported.
async fn finished_after_reconnect(command: impl FnOnce(SessionId, BulkRunId) -> Command) {
    let store = FakeStore::new(Write::UntilReleased);
    let config = RuntimeConfig {
        bulk_write_timeout: Duration::from_secs(60),
        ..runtime_config()
    };
    let (mut h, session) = connected_with(&store, false, config).await;
    let run = run_id();
    h.tx.send(command(session, run)).unwrap();
    tokio::time::timeout(WAIT, store.entered.notified())
        .await
        .expect("the write started");
    h.tx.send(Command::Connect(ConnectionConfig {
        id: session,
        ..config_for(session)
    }))
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    store.release.notify_one();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(applied())
        }
    );
}

fn config_for(session: SessionId) -> ConnectionConfig {
    ConnectionConfig {
        id: session,
        ..config(false)
    }
}

#[tokio::test]
async fn a_bulk_update_finishing_after_a_reconnect_is_still_reported() {
    finished_after_reconnect(|session, run| Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
    })
    .await;
}

#[tokio::test]
async fn a_bulk_delete_finishing_after_a_reconnect_is_still_reported() {
    finished_after_reconnect(|session, run| Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
    })
    .await;
}

#[tokio::test]
async fn a_hung_count_arrives_as_a_request_failed_timeout() {
    let store = FakeStore::new(Write::Hang).with(|s| s.count_hangs = true);
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! {},
        request: 9,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::RequestFailed {
            context: "bulk_count",
            request: 9,
            error: CoreError::Database(_),
            ..
        }
    ));
}

#[tokio::test]
async fn cancel_reaches_a_bulk_write_that_is_still_in_flight() {
    let store = FakeStore::new(Write::UntilCancelled);
    let config = RuntimeConfig {
        bulk_write_timeout: Duration::from_secs(60),
        ..runtime_config()
    };
    let (mut h, session) = connected_with(&store, false, config).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
    })
    .unwrap();
    tokio::time::timeout(WAIT, store.entered.notified())
        .await
        .expect("the write started");
    h.tx.send(Command::BulkCancel { session, run }).unwrap();
    let events = [next(&mut h).await, next(&mut h).await];
    assert!(events.contains(&Event::BulkCancelResult {
        session,
        run,
        outcome: Ok(())
    }));
    assert!(events.contains(&Event::BulkFinished {
        session,
        ns: mock_namespace(),
        run,
        outcome: Ok(BulkOutcome::Unknown { cancelled: true })
    }));
}
