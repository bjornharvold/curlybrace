//! The shared fake store, connector and helpers of the bulk runtime tests.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::Notify;

use super::*;
use crate::bulk::{
    BulkDeleteRequest, BulkOutcome, BulkRunId, BulkUpdateRequest, MatchScope, UpdatePreview,
    UpdateSpec,
};
use crate::store::Page;
use crate::test_support::mock_objects::{mock_doc_with_id, mock_namespace};

pub(super) const WAIT: Duration = Duration::from_secs(5);
pub(super) const SHORT: Duration = Duration::from_millis(100);

/// What the fake's `update_many` and `delete_many` do.
#[derive(Clone)]
pub(super) enum Write {
    Answer(Result<BulkOutcome>),
    /// Never answers by itself.
    Hang,
    /// Answers `Unknown { cancelled: true }` once `cancel_bulk` is called.
    UntilCancelled,
    /// Answers `Applied` once the test notifies `release`.
    UntilReleased,
}

/// A store that counts its calls and behaves as told.
pub(super) struct FakeStore {
    pub(super) write: Write,
    pub(super) count: Result<Option<u64>>,
    pub(super) preview: Result<UpdatePreview>,
    pub(super) calls: AtomicUsize,
    pub(super) pings: AtomicUsize,
    pub(super) entered: Notify,
    pub(super) cancelled: Notify,
    pub(super) release: Notify,
    /// Makes `count_matching` never answer.
    pub(super) count_hangs: bool,
    /// While set, `is_read_only` blocks the calling thread until it receives.
    pub(super) hold: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    pub(super) held: Notify,
    pub(super) queries: Mutex<Vec<Query>>,
    /// The runs `cancel_bulk` was called for.
    pub(super) cancels: Mutex<Vec<BulkRunId>>,
}

impl FakeStore {
    pub(super) fn new(write: Write) -> Arc<Self> {
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
            cancels: Mutex::new(vec![]),
        })
    }

    pub(super) fn with(self: Arc<Self>, edit: impl FnOnce(&mut Self)) -> Arc<Self> {
        let mut store = Arc::try_unwrap(self).ok().expect("unshared");
        edit(&mut store);
        Arc::new(store)
    }

    pub(super) async fn write(&self) -> Result<BulkOutcome> {
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
    async fn cancel_bulk(&self, run: BulkRunId) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.cancels.lock().unwrap().push(run);
        self.cancelled.notify_one();
        Ok(())
    }
}

pub(super) struct FakeConnector(pub(super) Arc<FakeStore>);

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

pub(super) fn applied() -> BulkOutcome {
    BulkOutcome::Applied {
        matched: 3,
        modified: 2,
        upserted: vec![],
        deleted: 0,
    }
}

pub(super) fn update_request() -> BulkUpdateRequest {
    BulkUpdateRequest {
        filter: doc! { "a": 1 },
        update: UpdateSpec::Operators(doc! { "$set": { "b": 2 } }),
        upsert: false,
        scope: MatchScope::Filtered,
    }
}

pub(super) fn delete_request() -> BulkDeleteRequest {
    BulkDeleteRequest {
        filter: doc! { "a": 1 },
        scope: MatchScope::Filtered,
    }
}

pub(super) fn run_id() -> BulkRunId {
    BulkRunId(uuid::Uuid::new_v4())
}

pub(super) fn config(read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        read_only,
        ..ConnectionConfig::new("c", "mongodb://localhost:27017")
    }
}

pub(super) fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        command_timeout: SHORT,
        bulk_write_timeout: SHORT,
        ping_timeout: SHORT,
        ..RuntimeConfig::default()
    }
}

pub(super) async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

/// A runtime connected (through a connector) to `store`.
pub(super) async fn connected(
    store: &Arc<FakeStore>,
    read_only: bool,
) -> (RuntimeHandle, SessionId) {
    connected_with(store, read_only, runtime_config()).await
}

pub(super) async fn connected_with(
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

pub(super) async fn no_more_events(h: &mut RuntimeHandle) {
    assert!(
        tokio::time::timeout(Duration::from_millis(150), h.rx.recv())
            .await
            .is_err(),
        "a second event arrived"
    );
}

pub(super) fn all_bulk_commands(session: SessionId) -> Vec<Command> {
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
