//! A store for the export-planning tests: records the writes it is asked for, or panics on one.

use crate::compare::key::{KeySpec, KeyTuple};
use crate::store::{DocumentStore, Namespace, Page, Query, WriteOp, WriteOutcome};
use crate::test_support::InMemoryStore;
use async_trait::async_trait;
use bson::{Bson, Document};
use std::sync::{Arc, Mutex};

/// A store over `InMemoryStore` that records every write it is asked for, or panics on one.
pub(super) struct Probe {
    pub(super) inner: InMemoryStore,
    pub(super) forbid_writes: bool,
    pub(super) writes: Mutex<Vec<WriteOp>>,
    /// Reads of this `_id` fail.
    pub(super) fail_id: Mutex<Option<i32>>,
}

impl Probe {
    pub(super) fn new(ns: &Namespace, docs: Vec<Document>, forbid_writes: bool) -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryStore::new().with_docs(ns, docs),
            forbid_writes,
            writes: Mutex::new(Vec::new()),
            fail_id: Mutex::new(None),
        })
    }

    pub(super) fn recorded(&self) -> Vec<WriteOp> {
        self.writes.lock().unwrap().clone()
    }

    fn check_read(&self, from: Option<&KeyTuple>) -> crate::error::Result<()> {
        let failing = *self.fail_id.lock().unwrap();
        match (failing, from.and_then(|f| f.0.first())) {
            (Some(id), Some(Bson::Int32(n))) if id == *n => {
                Err(crate::error::CoreError::Database("read".into()))
            }
            _ => Ok(()),
        }
    }
}

#[async_trait]
impl DocumentStore for Probe {
    async fn list_databases(&self) -> crate::error::Result<Vec<String>> {
        self.inner.list_databases().await
    }
    async fn list_collections(&self, db: &str) -> crate::error::Result<Vec<String>> {
        self.inner.list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> crate::error::Result<Page> {
        self.inner.find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: Document) -> crate::error::Result<Bson> {
        self.inner.insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> crate::error::Result<()> {
        self.inner.replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> crate::error::Result<()> {
        self.inner.delete(ns, id).await
    }
    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> crate::error::Result<Vec<Document>> {
        self.check_read(from)?;
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
    async fn scan_sorted_primary(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> crate::error::Result<Vec<Document>> {
        self.check_read(from)?;
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }
    async fn bulk_write(
        &self,
        ns: &Namespace,
        ops: Vec<WriteOp>,
    ) -> crate::error::Result<Vec<WriteOutcome>> {
        assert!(!self.forbid_writes, "export planning must never write");
        self.writes.lock().unwrap().extend(ops.iter().cloned());
        self.inner.bulk_write(ns, ops).await
    }
}
