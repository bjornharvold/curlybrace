//! A rig for the copy runtime tests: two connected sessions over in-memory stores that count
//! their calls and can send commands to the runtime in the middle of a copy.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use super::*;
use crate::compare::key::{KeySpec, KeyTuple};
use crate::copy::{ConflictPolicy, CopyCommand, CopyEvent, CopyItem, CopyRequest, CopyScope};
use crate::store::{CollectionKind, Page, WriteOp, WriteOutcome};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_numbered_docs;

pub(super) const WAIT: Duration = Duration::from_secs(5);
/// More than one `APPLY_BATCH` (500), so a run has several batches to be interrupted between.
pub(super) const MANY: i32 = 1200;
pub(super) const BATCH: u64 = 500;

/// Commands a store sends to the runtime right after one of its `bulk_write` calls completed.
pub(super) struct Armed {
    tx: UnboundedSender<Command>,
    after_call: usize,
    commands: Vec<Command>,
}

/// An in-memory store that counts its calls, can show views, fail scans of one collection and
/// send commands to the runtime in the middle of a copy.
pub(super) struct TestStore {
    inner: InMemoryStore,
    views: Vec<String>,
    fail_scan: Option<(String, CoreError)>,
    pub(super) scans: AtomicUsize,
    pub(super) writes: AtomicUsize,
    armed: Mutex<Option<Armed>>,
}

impl TestStore {
    pub(super) fn new(seed: &[(&str, &str, Vec<Document>)]) -> Self {
        let inner = seed
            .iter()
            .fold(InMemoryStore::new(), |store, (db, c, docs)| {
                store.with_docs(&Namespace::new(*db, *c), docs.clone())
            });
        Self {
            inner,
            views: Vec::new(),
            fail_scan: None,
            scans: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            armed: Mutex::new(None),
        }
    }

    pub(super) fn with_views(self, views: &[&str]) -> Self {
        Self {
            views: views.iter().map(|v| v.to_string()).collect(),
            ..self
        }
    }

    pub(super) fn failing_scan(self, collection: &str, error: CoreError) -> Self {
        Self {
            fail_scan: Some((collection.into(), error)),
            ..self
        }
    }

    pub(super) fn arm(
        &self,
        tx: &UnboundedSender<Command>,
        after_call: usize,
        commands: Vec<Command>,
    ) {
        *self.lock_armed() = Some(Armed {
            tx: tx.clone(),
            after_call,
            commands,
        });
    }

    pub(super) fn lock_armed(&self) -> MutexGuard<'_, Option<Armed>> {
        self.armed.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) async fn contents(&self, ns: &Namespace) -> Vec<Document> {
        let query = Query {
            limit: 0,
            ..Query::default()
        };
        self.inner.find(ns, &query).await.unwrap().docs
    }

    /// Sends the armed commands once the call numbered `call` is done, then lets the dispatcher
    /// handle them before the write returns (the test runtime is single threaded).
    pub(super) async fn fire_if_armed(&self, call: usize) {
        let due = {
            let mut armed = self.lock_armed();
            match armed.as_ref().is_some_and(|a| a.after_call == call) {
                true => armed.take(),
                false => None,
            }
        };
        let Some(armed) = due else { return };
        for command in armed.commands {
            armed.tx.send(command).unwrap();
        }
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }
}

#[async_trait]
impl DocumentStore for TestStore {
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
        self.inner.insert(ns, doc).await
    }

    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }

    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }

    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        let call = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        let outcomes = self.inner.bulk_write(ns, ops).await;
        self.fire_if_armed(call).await;
        outcomes
    }

    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.inner.estimated_count(ns).await
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        if let Some((collection, error)) = &self.fail_scan {
            if *collection == ns.collection {
                return Err(error.clone());
            }
        }
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }

    async fn key_index_status(
        &self,
        ns: &Namespace,
        key: &KeySpec,
    ) -> Result<crate::store::IndexStatus> {
        self.inner.key_index_status(ns, key).await
    }

    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        let mut kinds = self.inner.list_collection_kinds(db).await?;
        kinds.extend(self.views.iter().map(|v| (v.clone(), CollectionKind::View)));
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(kinds)
    }
}

/// Hands out the store registered for the connection id.
pub(super) struct PerId(HashMap<Uuid, Arc<dyn DocumentStore>>);

#[async_trait]
impl Connector for PerId {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0[&cfg.id].clone())
    }
}

/// Two connected sessions, a source and a target.
pub(super) struct Rig {
    pub(super) h: RuntimeHandle,
    pub(super) source: Arc<TestStore>,
    pub(super) target: Arc<TestStore>,
    pub(super) source_cfg: ConnectionConfig,
    pub(super) target_cfg: ConnectionConfig,
}

impl Rig {
    pub(super) async fn new(source: TestStore, target: TestStore) -> Self {
        Self::with_read_only(source, target, false).await
    }

    pub(super) async fn with_read_only(
        source: TestStore,
        target: TestStore,
        target_read_only: bool,
    ) -> Self {
        let (source, target) = (Arc::new(source), Arc::new(target));
        let source_cfg = ConnectionConfig::new("source", "mongodb://localhost:27017");
        let target_cfg = ConnectionConfig {
            read_only: target_read_only,
            ..ConnectionConfig::new("target", "mongodb://localhost:27018")
        };
        let stores: HashMap<Uuid, Arc<dyn DocumentStore>> = HashMap::from([
            (source_cfg.id, source.clone() as Arc<dyn DocumentStore>),
            (target_cfg.id, target.clone() as Arc<dyn DocumentStore>),
        ]);
        let mut h = spawn_runtime_with_connector(
            None,
            None,
            Arc::new(PerId(stores)),
            Arc::new(|| {}),
            RuntimeConfig::default(),
        );
        for cfg in [&source_cfg, &target_cfg] {
            h.tx.send(Command::Connect(cfg.clone())).unwrap();
            assert!(matches!(next(&mut h).await, Event::Connected { .. }));
        }
        Self {
            h,
            source,
            target,
            source_cfg,
            target_cfg,
        }
    }

    pub(super) fn request(&self, scope: CopyScope, policy: ConflictPolicy) -> CopyRequest {
        CopyRequest {
            source: self.source_cfg.id,
            target: self.target_cfg.id,
            scope,
            policy,
            with_indexes_and_options: false,
        }
    }

    pub(super) fn send(&self, command: CopyCommand) {
        self.h.tx.send(Command::Copy(command)).unwrap();
    }

    /// Every copy event up to and including the terminal one of `run`.
    pub(super) async fn until_terminal(&mut self, run: RunId) -> Vec<CopyEvent> {
        let mut seen = Vec::new();
        loop {
            if let Event::Copy(event) = next(&mut self.h).await {
                let terminal = is_terminal(&event, run);
                seen.push(event);
                if terminal {
                    return seen;
                }
            }
        }
    }

    /// Copy events that arrive within `grace`, to prove nothing more is sent.
    pub(super) async fn drain_copy_events(&mut self, grace: Duration) -> Vec<CopyEvent> {
        let mut seen = Vec::new();
        while let Ok(Some(event)) = tokio::time::timeout(grace, self.h.rx.recv()).await {
            if let Event::Copy(event) = event {
                seen.push(event);
            }
        }
        seen
    }
}

pub(super) type RunId = crate::compare::protocol::RunId;

pub(super) fn is_terminal(event: &CopyEvent, run: RunId) -> bool {
    match event {
        CopyEvent::PreviewDone { run: r }
        | CopyEvent::Done { run: r, .. }
        | CopyEvent::Failed { run: r, .. } => *r == run,
        _ => false,
    }
}

pub(super) async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

pub(super) fn item(source: (&str, &str), target: (&str, &str)) -> CopyItem {
    CopyItem {
        source: Namespace::new(source.0, source.1),
        target: Namespace::new(target.0, target.1),
        filter: Document::new(),
    }
}

pub(super) fn one_item() -> CopyScope {
    CopyScope::Collections(vec![item(("s", "orders"), ("t", "orders"))])
}

pub(super) fn ns(db: &str, collection: &str) -> Namespace {
    Namespace::new(db, collection)
}

pub(super) fn item_reports(events: &[CopyEvent]) -> Vec<crate::copy::CopyReport> {
    events
        .iter()
        .filter_map(|e| match e {
            CopyEvent::ItemDone { report, .. } => Some(report.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn seeded(count: i32) -> TestStore {
    TestStore::new(&[("s", "orders", mock_numbered_docs(count))])
}

pub(super) fn empty() -> TestStore {
    TestStore::new(&[])
}
